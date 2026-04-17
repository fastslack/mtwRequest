mod types;

pub use types::*;

use async_trait::async_trait;
use dashmap::DashMap;
use mtw_core::MtwError;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::chain::ChainRegistry;
use crate::feedback::FeedbackStore;
use crate::provider::{
    CompletionRequest, Message, MtwAIProvider, ToolCall, ToolDef, ToolResult,
};
use crate::store::{
    AgentMemoryRecord, AgentStore, MemoryRole, RunFilter, RunUpdate,
};
use crate::trigger::TriggerType;

// ---------------------------------------------------------------------------
// ExecutorEngine -- the concrete executor
// ---------------------------------------------------------------------------

/// Concrete agent executor that implements the LLM tool-use loop.
///
/// The engine holds references to LLM providers, tool definitions, and
/// optional registries for chains and feedback/learnings.
pub struct ExecutorEngine {
    /// Registered LLM providers keyed by provider name
    pub providers: Arc<DashMap<String, Arc<dyn MtwAIProvider>>>,
    /// Available tools keyed by tool name
    pub tools: Arc<DashMap<String, ToolDefinition>>,
    /// Active run cancellation flags keyed by run ID
    pub active_run_flags: Arc<DashMap<String, Arc<AtomicBool>>>,
    /// Default provider name used when agent config omits one
    pub default_provider: String,
    /// Optional chain registry for post-execution chaining
    pub chain_registry: Option<Arc<ChainRegistry>>,
    /// Optional feedback store for injecting learnings into prompts
    pub feedback_store: Option<Arc<FeedbackStore>>,
    /// Agent configurations keyed by agent ID
    pub agent_configs: Arc<DashMap<String, AgentConfig>>,
    /// Optional persistent store for agents, runs, steps, and memory.
    /// When set, runs/steps/memory are persisted and `execute()` falls back
    /// to the store if the in-memory agent config map misses.
    pub store: Option<Arc<dyn AgentStore>>,
}

impl ExecutorEngine {
    /// Create a new executor engine with a default provider name.
    pub fn new(default_provider: impl Into<String>) -> Self {
        Self {
            providers: Arc::new(DashMap::new()),
            tools: Arc::new(DashMap::new()),
            active_run_flags: Arc::new(DashMap::new()),
            default_provider: default_provider.into(),
            chain_registry: None,
            feedback_store: None,
            agent_configs: Arc::new(DashMap::new()),
            store: None,
        }
    }

    /// Set the chain registry for post-execution chaining.
    pub fn with_chain_registry(mut self, registry: Arc<ChainRegistry>) -> Self {
        self.chain_registry = Some(registry);
        self
    }

    /// Set the feedback store for learnings injection.
    pub fn with_feedback_store(mut self, store: Arc<FeedbackStore>) -> Self {
        self.feedback_store = Some(store);
        self
    }

    /// Set the persistent agent store.
    ///
    /// When set, `execute()` creates and updates an `AgentRun` row, appends
    /// each emitted `AgentStep`, and persists a `MemoryRole::User` and
    /// `MemoryRole::Assistant` entry per run. The engine also falls back
    /// to the store when `agent_configs` has no entry for the requested
    /// agent ID.
    pub fn with_store(mut self, store: Arc<dyn AgentStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Look up an agent config, first in the in-memory map, then in the
    /// persistent store if configured.
    async fn resolve_agent_config(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentConfig>, MtwError> {
        if let Some(cfg) = self.agent_configs.get(agent_id) {
            return Ok(Some(cfg.value().clone()));
        }
        if let Some(store) = &self.store {
            return store.get_agent(agent_id).await;
        }
        Ok(None)
    }

    /// List currently-active run IDs from the persistent store for a given
    /// agent, or an empty vec if no store is configured.
    pub async fn list_runs(
        &self,
        filter: &RunFilter,
    ) -> Result<Vec<AgentRun>, MtwError> {
        match &self.store {
            Some(s) => s.list_runs(filter).await,
            None => Ok(Vec::new()),
        }
    }

    /// Register an LLM provider.
    pub fn register_provider(&self, provider: Arc<dyn MtwAIProvider>) {
        let name = provider.name().to_string();
        tracing::info!(provider = %name, "registered AI provider");
        self.providers.insert(name, provider);
    }

    /// Register a callable tool.
    pub fn register_tool(&self, tool: ToolDefinition) {
        tracing::info!(tool = %tool.name, "registered tool");
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Register an agent configuration.
    pub fn register_agent_config(&self, config: AgentConfig) {
        tracing::info!(agent = %config.id, name = %config.name, "registered agent config");
        self.agent_configs.insert(config.id.clone(), config);
    }

    // -- helpers -------------------------------------------------------------

    /// Interpolate `{{variable}}` patterns in a template using provided vars.
    fn interpolate(template: &str, vars: &HashMap<String, String>) -> String {
        let mut result = template.to_string();
        for (key, value) in vars {
            let pattern = format!("{{{{{}}}}}", key);
            result = result.replace(&pattern, value);
        }
        result
    }

    /// Build the system prompt, optionally injecting learnings.
    fn build_system_prompt(
        &self,
        agent_config: &AgentConfig,
        vars: &HashMap<String, String>,
    ) -> String {
        let mut prompt = Self::interpolate(&agent_config.system_prompt, vars);

        // Inject active learnings if a feedback store is available
        if let Some(store) = &self.feedback_store {
            let learnings = store.get_active_learnings(&agent_config.id);
            if !learnings.is_empty() {
                prompt.push_str("\n\n## Learnings from previous runs\n");
                for learning in &learnings {
                    prompt.push_str(&format!(
                        "- [{}] {}\n",
                        serde_json::to_string(&learning.learning_type)
                            .unwrap_or_default()
                            .trim_matches('"'),
                        learning.content,
                    ));
                }
            }
        }

        prompt
    }

    /// Collect ToolDef descriptors for the tools this agent is allowed to use.
    fn collect_tool_defs(&self, agent_config: &AgentConfig) -> Vec<ToolDef> {
        agent_config
            .tool_names
            .iter()
            .filter_map(|name| {
                self.tools.get(name).map(|t| ToolDef {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                })
            })
            .collect()
    }

    /// Create a timestamp string (seconds since UNIX epoch).
    fn now_timestamp() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string()
    }

    /// Record a step and return it.
    fn make_step(
        run_id: &str,
        step_number: u32,
        step_type: StepType,
        content: &str,
        tool_name: &str,
        tool_input: Value,
        tool_output: &str,
        tokens: u32,
    ) -> AgentStep {
        AgentStep {
            id: ulid::Ulid::new().to_string(),
            run_id: run_id.to_string(),
            step_number,
            step_type,
            content: content.to_string(),
            tool_name: tool_name.to_string(),
            tool_input,
            tool_output: tool_output.to_string(),
            tokens,
            created_at: Self::now_timestamp(),
        }
    }

    /// Persist a step to the store if one is configured.
    /// Persistence failures are logged but non-fatal.
    async fn persist_step(&self, step: &AgentStep) {
        if let Some(store) = &self.store {
            if let Err(e) = store.add_step(step).await {
                tracing::warn!(
                    run_id = %step.run_id,
                    step = step.step_number,
                    error = %e,
                    "add_step failed"
                );
            }
        }
    }

    /// Persist multiple steps in one call. Falls back to sequential if the
    /// store doesn't support batch, but saves spawn_blocking overhead.
    async fn persist_steps_batch(&self, batch: &[AgentStep]) {
        if batch.is_empty() {
            return;
        }
        if let Some(store) = &self.store {
            for step in batch {
                if let Err(e) = store.add_step(step).await {
                    tracing::warn!(
                        run_id = %step.run_id,
                        step = step.step_number,
                        error = %e,
                        "add_step (batch) failed"
                    );
                }
            }
        }
    }

    /// The main execution loop: call the LLM, process tool calls, repeat.
    async fn run_loop(
        &self,
        agent_config: &AgentConfig,
        goal: &str,
        config: &ExecutionConfig,
        run_id: &str,
        cancelled: Arc<AtomicBool>,
    ) -> ExecutionResult {
        // Resolve provider
        let provider_name = if agent_config.provider.is_empty() {
            &self.default_provider
        } else {
            &agent_config.provider
        };

        let provider = match self.providers.get(provider_name) {
            Some(p) => Arc::clone(p.value()),
            None => {
                return ExecutionResult {
                    status: RunStatus::Failed,
                    result: String::new(),
                    error: format!("provider not found: {}", provider_name),
                    steps_count: 0,
                    tokens_used: 0,
                };
            }
        };

        // Build system prompt with variable interpolation
        let mut vars = HashMap::new();
        vars.insert("goal".to_string(), goal.to_string());
        vars.insert("agent_name".to_string(), agent_config.name.clone());
        let system_prompt = self.build_system_prompt(agent_config, &vars);

        // Collect tool definitions once (reused across iterations)
        let tool_defs = self.collect_tool_defs(agent_config);
        // Pre-compute the Option to avoid re-checking every iteration
        let tools_for_request: Option<Vec<ToolDef>> = if tool_defs.is_empty() {
            None
        } else {
            Some(tool_defs)
        };

        // Initialize messages — pre-allocate for typical multi-step runs
        let mut messages: Vec<Message> = Vec::with_capacity(2 + (config.max_iterations as usize * 2));
        messages.push(Message::system(&system_prompt));
        messages.push(Message::user(goal));

        // Loop detection: (tool_name, result_snippet) -> count
        let mut loop_signatures: HashMap<(String, String), u32> = HashMap::new();
        let mut consecutive_errors: u32 = 0;
        let mut tokens_used: u32 = 0;
        let mut steps: Vec<AgentStep> = Vec::new();
        let mut step_number: u32 = 0;
        let start = Instant::now();

        for _iteration in 0..config.max_iterations {
            // Check cancellation
            if cancelled.load(Ordering::Relaxed) {
                return ExecutionResult {
                    status: RunStatus::Cancelled,
                    result: String::new(),
                    error: "run cancelled".to_string(),
                    steps_count: steps.len() as u32,
                    tokens_used,
                };
            }

            // Check timeout
            if start.elapsed().as_millis() as u64 > config.timeout_ms {
                return ExecutionResult {
                    status: RunStatus::Failed,
                    result: String::new(),
                    error: "execution timed out".to_string(),
                    steps_count: steps.len() as u32,
                    tokens_used,
                };
            }

            // Check token budget
            if agent_config.token_budget > 0 && tokens_used >= agent_config.token_budget {
                return ExecutionResult {
                    status: RunStatus::Failed,
                    result: String::new(),
                    error: "token budget exhausted".to_string(),
                    steps_count: steps.len() as u32,
                    tokens_used,
                };
            }

            // Check consecutive errors
            if consecutive_errors >= config.max_errors {
                return ExecutionResult {
                    status: RunStatus::Failed,
                    result: String::new(),
                    error: format!(
                        "too many consecutive errors ({})",
                        consecutive_errors
                    ),
                    steps_count: steps.len() as u32,
                    tokens_used,
                };
            }

            // Build completion request — messages.clone() is unavoidable since
            // CompletionRequest owns the Vec. Pre-allocation above minimises
            // re-alloc churn. tools_for_request.clone() copies the tool schemas
            // but they're stable across iterations.
            let req = CompletionRequest {
                model: agent_config.model.clone(),
                messages: messages.clone(),
                tools: tools_for_request.clone(),
                temperature: None,
                max_tokens: None,
                metadata: HashMap::new(),
            };

            // Call the LLM
            let response = match provider.complete(req).await {
                Ok(r) => r,
                Err(e) => {
                    consecutive_errors += 1;
                    tracing::error!(
                        run_id = %run_id,
                        error = %e,
                        "LLM provider call failed"
                    );
                    step_number += 1;
                    let step = Self::make_step(
                        run_id,
                        step_number,
                        StepType::Error,
                        &format!("LLM call failed: {}", e),
                        "",
                        Value::Null,
                        "",
                        0,
                    );
                    self.persist_step(&step).await;
                    steps.push(step);
                    continue;
                }
            };

            tokens_used += response.usage.total_tokens;

            // No tool calls -- final answer
            if response.tool_calls.is_empty() {
                step_number += 1;
                let step = Self::make_step(
                    run_id,
                    step_number,
                    StepType::Final,
                    &response.content,
                    "",
                    Value::Null,
                    "",
                    response.usage.total_tokens,
                );
                self.persist_step(&step).await;
                steps.push(step);

                return ExecutionResult {
                    status: RunStatus::Completed,
                    result: response.content,
                    error: String::new(),
                    steps_count: steps.len() as u32,
                    tokens_used,
                };
            }

            // Record thought if the response also includes text content
            if !response.content.is_empty() {
                step_number += 1;
                let step = Self::make_step(
                    run_id,
                    step_number,
                    StepType::Thought,
                    &response.content,
                    "",
                    Value::Null,
                    "",
                    0,
                );
                self.persist_step(&step).await;
                steps.push(step);
                messages.push(Message::assistant(&response.content));
            }

            // Process each tool call — steps are collected and persisted in batch
            let mut tool_results: Vec<ToolResult> = Vec::new();
            let mut pending_steps: Vec<AgentStep> = Vec::new();

            for tc in &response.tool_calls {
                let (result_value, is_error) = self.invoke_tool(tc).await;

                let result_str = match serde_json::to_string(&result_value) {
                    Ok(s) => s,
                    Err(_) => result_value.to_string(),
                };

                // Record tool_call + tool_result steps (batched persist below)
                step_number += 1;
                let call_step = Self::make_step(
                    run_id,
                    step_number,
                    StepType::ToolCall,
                    "",
                    &tc.name,
                    tc.arguments.clone(),
                    "",
                    0,
                );
                step_number += 1;
                let result_step = Self::make_step(
                    run_id,
                    step_number,
                    StepType::ToolResult,
                    "",
                    &tc.name,
                    Value::Null,
                    &result_str,
                    0,
                );
                pending_steps.push(call_step);
                pending_steps.push(result_step);

                // Loop detection
                let snippet: String = result_str.chars().take(100).collect();
                let key = (tc.name.clone(), snippet);
                let counter = loop_signatures.entry(key).or_insert(0);
                *counter += 1;
                if *counter >= 3 {
                    tracing::warn!(
                        run_id = %run_id,
                        tool = %tc.name,
                        "loop detected: same tool+result repeated 3 times"
                    );
                    return ExecutionResult {
                        status: RunStatus::Failed,
                        result: String::new(),
                        error: format!(
                            "loop detected: tool '{}' produced the same result 3 times",
                            tc.name
                        ),
                        steps_count: steps.len() as u32,
                        tokens_used,
                    };
                }

                // Track consecutive errors
                if is_error {
                    consecutive_errors += 1;
                } else {
                    consecutive_errors = 0;
                }

                tool_results.push(ToolResult {
                    tool_call_id: tc.id.clone(),
                    name: tc.name.clone(),
                    result: result_value,
                    is_error,
                });
            }

            // Flush batched steps in one go (fewer spawn_blocking calls)
            self.persist_steps_batch(&pending_steps).await;
            steps.extend(pending_steps);

            // Append tool results as messages for the next LLM call.
            // We add the assistant message indicating tool use, then the tool
            // result messages. The exact wire format depends on the provider,
            // but we approximate with role=Tool messages carrying the output.
            messages.push(Message::assistant(
                &format!(
                    "[tool_use: {}]",
                    response
                        .tool_calls
                        .iter()
                        .map(|tc| tc.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));

            for tr in &tool_results {
                let tool_msg = Message {
                    role: crate::provider::MessageRole::Tool,
                    content: serde_json::to_string(&serde_json::json!({
                        "tool_call_id": tr.tool_call_id,
                        "name": tr.name,
                        "result": tr.result,
                        "is_error": tr.is_error,
                    }))
                    .unwrap_or_default(),
                };
                messages.push(tool_msg);
            }
        }

        // Exhausted max iterations
        ExecutionResult {
            status: RunStatus::Failed,
            result: String::new(),
            error: format!(
                "max iterations reached ({})",
                config.max_iterations
            ),
            steps_count: steps.len() as u32,
            tokens_used,
        }
    }

    /// Invoke a single tool by name, returning the result value and whether
    /// it was an error.
    async fn invoke_tool(&self, tc: &ToolCall) -> (Value, bool) {
        let tool_ref = match self.tools.get(&tc.name) {
            Some(t) => t,
            None => {
                tracing::warn!(tool = %tc.name, "tool not found");
                return (
                    serde_json::json!({ "error": format!("tool not found: {}", tc.name) }),
                    true,
                );
            }
        };

        let handler = Arc::clone(&tool_ref.handler);
        // Drop the DashMap ref before the async call to avoid holding it
        // across an await point.
        drop(tool_ref);

        match (handler)(tc.arguments.clone()).await {
            Ok(value) => (value, false),
            Err(e) => {
                tracing::error!(tool = %tc.name, error = %e, "tool execution failed");
                (
                    serde_json::json!({ "error": format!("tool failed: {}", e) }),
                    true,
                )
            }
        }
    }

    /// Evaluate and execute post-completion chains.
    fn execute_chains<'a>(
        &'a self,
        agent_id: &'a str,
        result: &'a ExecutionResult,
        config: &'a ExecutionConfig,
        depth: u32,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(self.execute_chains_inner(agent_id, result, config, depth))
    }

    /// Inner implementation for chain execution.
    async fn execute_chains_inner(
        &self,
        agent_id: &str,
        result: &ExecutionResult,
        config: &ExecutionConfig,
        depth: u32,
    ) {
        if depth >= config.max_chain_depth {
            tracing::warn!(
                agent_id = %agent_id,
                depth = depth,
                "max chain depth reached, skipping further chains"
            );
            return;
        }

        let registry = match &self.chain_registry {
            Some(r) => Arc::clone(r),
            None => return,
        };

        let chains = registry.get_chains_for_source(agent_id);
        let success = result.status == RunStatus::Completed;

        for chain in chains {
            let should_run = registry.evaluate_condition(&chain.condition, success);
            if !should_run {
                continue;
            }

            tracing::info!(
                chain_id = %chain.id,
                source = %chain.source_agent_id,
                target = %chain.target_agent_id,
                "executing chain"
            );

            // Build the goal for the chained agent
            let chained_goal = if chain.pass_result {
                format!(
                    "Continue from previous agent result:\n\n{}",
                    result.result
                )
            } else {
                // Use the target agent's default goal or a generic one
                "Execute your default task".to_string()
            };

            // Clone what we need for the spawned task
            let engine_providers = Arc::clone(&self.providers);
            let engine_tools = Arc::clone(&self.tools);
            let engine_active = Arc::clone(&self.active_run_flags);
            let engine_configs = Arc::clone(&self.agent_configs);
            let chain_reg = self.chain_registry.clone();
            let feedback = self.feedback_store.clone();
            let engine_store = self.store.clone();
            let default_provider = self.default_provider.clone();
            let exec_config = config.clone();
            let target_id = chain.target_agent_id.clone();
            let delay = chain.delay_ms;
            let next_depth = depth + 1;

            tokio::spawn(async move {
                // Apply optional delay
                if delay > 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                }

                let engine = ExecutorEngine {
                    providers: engine_providers,
                    tools: engine_tools,
                    active_run_flags: engine_active,
                    default_provider,
                    chain_registry: chain_reg,
                    feedback_store: feedback,
                    agent_configs: engine_configs,
                    store: engine_store,
                };

                match engine.execute(&target_id, &chained_goal, &exec_config).await {
                    Ok(chain_result) => {
                        tracing::info!(
                            target = %target_id,
                            status = ?chain_result.status,
                            "chain execution completed"
                        );
                        // Recurse chains from the target
                        engine
                            .execute_chains(&target_id, &chain_result, &exec_config, next_depth)
                            .await;
                    }
                    Err(e) => {
                        tracing::error!(
                            target = %target_id,
                            error = %e,
                            "chain execution failed"
                        );
                    }
                }
            });
        }
    }
}

#[async_trait]
impl MtwAgentExecutor for ExecutorEngine {
    async fn execute(
        &self,
        agent_id: &str,
        goal: &str,
        config: &ExecutionConfig,
    ) -> Result<ExecutionResult, MtwError> {
        // Look up agent config (DashMap first, store as fallback)
        let agent_config = self
            .resolve_agent_config(agent_id)
            .await?
            .ok_or_else(|| {
                MtwError::Agent(format!("agent config not found: {}", agent_id))
            })?;

        let run_id = ulid::Ulid::new().to_string();
        let created_at = Self::now_timestamp();

        tracing::info!(
            run_id = %run_id,
            agent_id = %agent_id,
            goal = %goal,
            "starting agent execution"
        );

        // Persist run + user memory if a store is configured.
        // Failures to persist are logged but don't abort the run — the
        // executor degrades gracefully to in-memory-only mode.
        if let Some(store) = &self.store {
            let run = AgentRun {
                id: run_id.clone(),
                agent_id: agent_id.to_string(),
                trigger_type: TriggerType::Manual,
                trigger_payload: Value::Null,
                goal: goal.to_string(),
                status: RunStatus::Running,
                result: String::new(),
                error: String::new(),
                steps_count: 0,
                tokens_used: 0,
                started_at: Some(created_at.clone()),
                completed_at: None,
                created_at: created_at.clone(),
            };
            if let Err(e) = store.create_run(&run).await {
                tracing::warn!(run_id = %run_id, error = %e, "create_run failed");
            }
            let memory = AgentMemoryRecord {
                id: ulid::Ulid::new().to_string(),
                agent_id: agent_id.to_string(),
                role: MemoryRole::User,
                content: goal.to_string(),
                run_id: run_id.clone(),
                created_at: created_at.clone(),
            };
            if let Err(e) = store.add_memory(&memory).await {
                tracing::warn!(run_id = %run_id, error = %e, "add_memory(user) failed");
            }
        }

        // Set up cancellation flag
        let cancelled = Arc::new(AtomicBool::new(false));
        self.active_run_flags
            .insert(run_id.clone(), Arc::clone(&cancelled));

        // Run the main loop
        let result = self
            .run_loop(&agent_config, goal, config, &run_id, cancelled)
            .await;

        // Clean up active run
        self.active_run_flags.remove(&run_id);

        // Persist final run state + assistant memory.
        if let Some(store) = &self.store {
            let completed_at = Self::now_timestamp();
            let update = RunUpdate {
                status: Some(result.status.clone()),
                result: Some(result.result.clone()),
                error: Some(result.error.clone()),
                steps_count: Some(result.steps_count),
                tokens_used: Some(result.tokens_used),
                completed_at: Some(completed_at.clone()),
                ..Default::default()
            };
            if let Err(e) = store.update_run(&run_id, &update).await {
                tracing::warn!(run_id = %run_id, error = %e, "update_run failed");
            }
            // Only save assistant memory when there is actual content to remember.
            let content = if result.result.is_empty() {
                result.error.clone()
            } else {
                result.result.clone()
            };
            if !content.is_empty() {
                let memory = AgentMemoryRecord {
                    id: ulid::Ulid::new().to_string(),
                    agent_id: agent_id.to_string(),
                    role: MemoryRole::Assistant,
                    content,
                    run_id: run_id.clone(),
                    created_at: completed_at,
                };
                if let Err(e) = store.add_memory(&memory).await {
                    tracing::warn!(run_id = %run_id, error = %e, "add_memory(assistant) failed");
                }
            }
        }

        tracing::info!(
            run_id = %run_id,
            status = ?result.status,
            tokens = result.tokens_used,
            steps = result.steps_count,
            "agent execution finished"
        );

        // Execute chains (fire-and-forget via spawn inside)
        self.execute_chains(agent_id, &result, config, 0).await;

        Ok(result)
    }

    fn cancel_run(&self, run_id: &str) -> bool {
        if let Some(flag) = self.active_run_flags.get(run_id) {
            flag.value().store(true, Ordering::Relaxed);
            tracing::info!(run_id = %run_id, "run cancellation requested");
            true
        } else {
            false
        }
    }

    fn active_runs(&self) -> Vec<String> {
        self.active_run_flags
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        CompletionResponse, FinishReason, ModelInfo, ProviderCapabilities,
        StreamChunk, Usage,
    };
    use futures::Stream;
    use std::pin::Pin;

    // -- Mock provider -------------------------------------------------------

    struct MockProvider {
        responses: std::sync::Mutex<Vec<CompletionResponse>>,
    }

    impl MockProvider {
        fn new(responses: Vec<CompletionResponse>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }

        fn simple_response(content: &str) -> CompletionResponse {
            CompletionResponse {
                id: ulid::Ulid::new().to_string(),
                model: "mock".to_string(),
                content: content.to_string(),
                tool_calls: vec![],
                usage: Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                },
                finish_reason: FinishReason::Stop,
            }
        }

        fn tool_call_response(
            tool_name: &str,
            args: Value,
        ) -> CompletionResponse {
            CompletionResponse {
                id: ulid::Ulid::new().to_string(),
                model: "mock".to_string(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "tc-1".to_string(),
                    name: tool_name.to_string(),
                    arguments: args,
                }],
                usage: Usage {
                    prompt_tokens: 10,
                    completion_tokens: 8,
                    total_tokens: 18,
                },
                finish_reason: FinishReason::ToolUse,
            }
        }

        fn thought_and_tool_response(
            thought: &str,
            tool_name: &str,
            args: Value,
        ) -> CompletionResponse {
            CompletionResponse {
                id: ulid::Ulid::new().to_string(),
                model: "mock".to_string(),
                content: thought.to_string(),
                tool_calls: vec![ToolCall {
                    id: "tc-2".to_string(),
                    name: tool_name.to_string(),
                    arguments: args,
                }],
                usage: Usage {
                    prompt_tokens: 12,
                    completion_tokens: 10,
                    total_tokens: 22,
                },
                finish_reason: FinishReason::ToolUse,
            }
        }
    }

    #[async_trait]
    impl MtwAIProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                streaming: false,
                tool_calling: true,
                vision: false,
                embeddings: false,
                max_context: 8192,
            }
        }

        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, MtwError> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(Self::simple_response("default response"))
            } else {
                Ok(responses.remove(0))
            }
        }

        fn stream(
            &self,
            _req: CompletionRequest,
        ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>>
        {
            Box::pin(futures::stream::empty())
        }

        async fn models(&self) -> Result<Vec<ModelInfo>, MtwError> {
            Ok(vec![])
        }
    }

    // -- Helpers -------------------------------------------------------------

    fn make_engine(responses: Vec<CompletionResponse>) -> ExecutorEngine {
        let engine = ExecutorEngine::new("mock");
        engine.register_provider(Arc::new(MockProvider::new(responses)));
        engine.register_agent_config(AgentConfig {
            id: "test-agent".to_string(),
            name: "Test Agent".to_string(),
            provider: "mock".to_string(),
            model: "mock-model".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            tool_names: vec!["echo".to_string()],
            token_budget: 0,
        });
        engine
    }

    fn register_echo_tool(engine: &ExecutorEngine) {
        engine.register_tool(ToolDefinition {
            name: "echo".to_string(),
            description: "Echoes back input".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                }
            }),
            handler: Arc::new(|args| {
                Box::pin(async move {
                    let text = args
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("echo");
                    Ok(serde_json::json!({ "echoed": text }))
                })
            }),
        });
    }

    // -- Tests ---------------------------------------------------------------

    #[test]
    fn test_execution_config_default() {
        let c = ExecutionConfig::default();
        assert_eq!(c.max_iterations, 15);
        assert_eq!(c.timeout_ms, 300_000);
        assert_eq!(c.max_chain_depth, 5);
    }

    #[test]
    fn test_run_status_serialization() {
        assert_eq!(
            serde_json::to_string(&RunStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&StepType::ToolCall).unwrap(),
            "\"tool_call\""
        );
    }

    #[test]
    fn test_interpolate() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("task".to_string(), "search".to_string());

        let result =
            ExecutorEngine::interpolate("Hello {{name}}, please {{task}}", &vars);
        assert_eq!(result, "Hello Alice, please search");
    }

    #[test]
    fn test_interpolate_no_vars() {
        let vars = HashMap::new();
        let result = ExecutorEngine::interpolate("No variables here", &vars);
        assert_eq!(result, "No variables here");
    }

    #[tokio::test]
    async fn test_simple_execution() {
        let engine = make_engine(vec![MockProvider::simple_response(
            "The answer is 42",
        )]);

        let config = ExecutionConfig::default();
        let result = engine
            .execute("test-agent", "What is the answer?", &config)
            .await
            .unwrap();

        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.result, "The answer is 42");
        assert!(result.error.is_empty());
        assert_eq!(result.steps_count, 1); // one final step
        assert_eq!(result.tokens_used, 15);
    }

    #[tokio::test]
    async fn test_tool_call_execution() {
        let engine = make_engine(vec![
            MockProvider::tool_call_response(
                "echo",
                serde_json::json!({"text": "hello"}),
            ),
            MockProvider::simple_response("Done echoing"),
        ]);
        register_echo_tool(&engine);

        let config = ExecutionConfig::default();
        let result = engine
            .execute("test-agent", "Echo hello", &config)
            .await
            .unwrap();

        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.result, "Done echoing");
        // Steps: tool_call + tool_result + final = 3
        assert_eq!(result.steps_count, 3);
        assert_eq!(result.tokens_used, 18 + 15);
    }

    #[tokio::test]
    async fn test_thought_plus_tool_call() {
        let engine = make_engine(vec![
            MockProvider::thought_and_tool_response(
                "Let me think...",
                "echo",
                serde_json::json!({"text": "test"}),
            ),
            MockProvider::simple_response("All done"),
        ]);
        register_echo_tool(&engine);

        let config = ExecutionConfig::default();
        let result = engine
            .execute("test-agent", "Think and echo", &config)
            .await
            .unwrap();

        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.result, "All done");
        // Steps: thought + tool_call + tool_result + final = 4
        assert_eq!(result.steps_count, 4);
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let engine = make_engine(vec![
            MockProvider::tool_call_response(
                "nonexistent",
                serde_json::json!({}),
            ),
            MockProvider::simple_response("Recovered"),
        ]);

        let config = ExecutionConfig::default();
        let result = engine
            .execute("test-agent", "Use unknown tool", &config)
            .await
            .unwrap();

        // Should still complete because the error is returned to LLM
        assert_eq!(result.status, RunStatus::Completed);
    }

    #[tokio::test]
    async fn test_missing_agent_config() {
        let engine = ExecutorEngine::new("mock");
        let config = ExecutionConfig::default();
        let result = engine
            .execute("nonexistent", "Hello", &config)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_missing_provider() {
        let engine = ExecutorEngine::new("mock");
        engine.register_agent_config(AgentConfig {
            id: "agent-no-provider".to_string(),
            name: "No Provider".to_string(),
            provider: "nonexistent".to_string(),
            model: "m".to_string(),
            system_prompt: "test".to_string(),
            tool_names: vec![],
            token_budget: 0,
        });

        let config = ExecutionConfig::default();
        let result = engine
            .execute("agent-no-provider", "Hello", &config)
            .await
            .unwrap();

        assert_eq!(result.status, RunStatus::Failed);
        assert!(result.error.contains("provider not found"));
    }

    #[tokio::test]
    async fn test_cancel_run() {
        let engine = ExecutorEngine::new("mock");
        assert!(!engine.cancel_run("nonexistent"));
        assert!(engine.active_runs().is_empty());
    }

    #[tokio::test]
    async fn test_loop_detection() {
        // Return the same tool call 3 times with the same result
        let engine = make_engine(vec![
            MockProvider::tool_call_response(
                "echo",
                serde_json::json!({"text": "same"}),
            ),
            MockProvider::tool_call_response(
                "echo",
                serde_json::json!({"text": "same"}),
            ),
            MockProvider::tool_call_response(
                "echo",
                serde_json::json!({"text": "same"}),
            ),
        ]);
        register_echo_tool(&engine);

        let config = ExecutionConfig::default();
        let result = engine
            .execute("test-agent", "Loop forever", &config)
            .await
            .unwrap();

        assert_eq!(result.status, RunStatus::Failed);
        assert!(result.error.contains("loop detected"));
    }

    #[test]
    fn test_build_system_prompt_with_learnings() {
        use crate::feedback::{AgentLearning, LearningType};

        let store = Arc::new(FeedbackStore::new());
        store.add_learning(AgentLearning {
            id: "l1".to_string(),
            agent_id: "a1".to_string(),
            learning_type: LearningType::Pattern,
            content: "always check input".to_string(),
            confidence: 0.9,
            source_runs: vec![],
            active: true,
            created_at: "0".to_string(),
            updated_at: "0".to_string(),
        });

        let engine = ExecutorEngine::new("mock").with_feedback_store(store);
        let config = AgentConfig {
            id: "a1".to_string(),
            name: "Agent".to_string(),
            provider: "mock".to_string(),
            model: "m".to_string(),
            system_prompt: "You are helpful.".to_string(),
            tool_names: vec![],
            token_budget: 0,
        };

        let prompt = engine.build_system_prompt(&config, &HashMap::new());
        assert!(prompt.contains("You are helpful."));
        assert!(prompt.contains("Learnings from previous runs"));
        assert!(prompt.contains("always check input"));
    }

    #[test]
    fn test_chain_condition_integration() {
        use crate::chain::ChainCondition;
        let registry = ChainRegistry::new();
        assert!(registry.evaluate_condition(&ChainCondition::Always, true));
        assert!(registry.evaluate_condition(&ChainCondition::OnSuccess, true));
        assert!(!registry.evaluate_condition(&ChainCondition::OnSuccess, false));
        assert!(registry.evaluate_condition(&ChainCondition::OnFailure, false));
    }

    #[test]
    fn test_tool_definition_debug() {
        let tool = ToolDefinition {
            name: "test".to_string(),
            description: "A test tool".to_string(),
            parameters: serde_json::json!({}),
            handler: Arc::new(|_| Box::pin(async { Ok(Value::Null) })),
        };
        let debug = format!("{:?}", tool);
        assert!(debug.contains("test"));
        assert!(debug.contains("A test tool"));
    }

    // -- Persistence integration (store wiring) ------------------------------

    fn temp_store() -> Arc<crate::SqliteAgentStore> {
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().to_string();
        std::mem::forget(file);
        let cfg = crate::store::AgentStoreConfig {
            path,
            ..Default::default()
        };
        Arc::new(crate::SqliteAgentStore::open(&cfg).unwrap())
    }

    fn sample_agent() -> AgentConfig {
        AgentConfig {
            id: "agent-run".into(),
            name: "Runner".into(),
            provider: "mock".into(),
            model: "mock".into(),
            system_prompt: "you are a tester".into(),
            tool_names: vec![],
            token_budget: 0,
        }
    }

    #[tokio::test]
    async fn execute_persists_run_and_final_step() {
        let store = temp_store();
        let agent = sample_agent();
        store.save_agent(&agent).await.unwrap();

        let engine = ExecutorEngine::new("mock").with_store(store.clone());
        engine.register_provider(Arc::new(MockProvider::new(vec![
            MockProvider::simple_response("hello world"),
        ])));

        let result = engine
            .execute(&agent.id, "say hi", &ExecutionConfig::default())
            .await
            .unwrap();

        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.result, "hello world");

        let runs = store
            .list_runs(&crate::store::RunFilter {
                agent_id: Some(agent.id.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.status, RunStatus::Completed);
        assert_eq!(run.result, "hello world");
        assert!(run.tokens_used > 0);

        let steps = store.list_steps(&run.id).await.unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_type, StepType::Final);
        assert_eq!(steps[0].content, "hello world");

        // Memory: user goal + assistant result
        let mem = store.recent_memory(&agent.id, 10).await.unwrap();
        assert_eq!(mem.len(), 2);
    }

    #[tokio::test]
    async fn execute_falls_back_to_store_for_agent_config() {
        let store = temp_store();
        // Only in the store, NOT registered in DashMap.
        let agent = AgentConfig {
            id: "only-in-store".into(),
            ..sample_agent()
        };
        store.save_agent(&agent).await.unwrap();

        let engine = ExecutorEngine::new("mock").with_store(store.clone());
        engine.register_provider(Arc::new(MockProvider::new(vec![
            MockProvider::simple_response("ok"),
        ])));

        let result = engine
            .execute(&agent.id, "hi", &ExecutionConfig::default())
            .await
            .unwrap();
        assert_eq!(result.status, RunStatus::Completed);
    }

    #[tokio::test]
    async fn execute_persists_thought_and_tool_steps() {
        let store = temp_store();
        let mut agent = sample_agent();
        agent.id = "agent-tools".into();
        agent.tool_names = vec!["echo".into()];
        store.save_agent(&agent).await.unwrap();

        let engine = ExecutorEngine::new("mock").with_store(store.clone());
        engine.register_provider(Arc::new(MockProvider::new(vec![
            MockProvider::thought_and_tool_response(
                "I will call echo",
                "echo",
                serde_json::json!({"text": "hi"}),
            ),
            MockProvider::simple_response("done"),
        ])));
        engine.register_tool(ToolDefinition {
            name: "echo".into(),
            description: "echo".into(),
            parameters: serde_json::json!({}),
            handler: Arc::new(|args| {
                Box::pin(async move { Ok(args) })
            }),
        });

        let result = engine
            .execute(&agent.id, "echo hi", &ExecutionConfig::default())
            .await
            .unwrap();
        assert_eq!(result.status, RunStatus::Completed);

        let runs = store
            .list_runs(&crate::store::RunFilter {
                agent_id: Some(agent.id.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        let run = &runs[0];
        let steps = store.list_steps(&run.id).await.unwrap();
        // thought, tool_call, tool_result, final
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0].step_type, StepType::Thought);
        assert_eq!(steps[1].step_type, StepType::ToolCall);
        assert_eq!(steps[1].tool_name, "echo");
        assert_eq!(steps[2].step_type, StepType::ToolResult);
        assert_eq!(steps[3].step_type, StepType::Final);
    }

    #[tokio::test]
    async fn execute_without_store_still_works() {
        let engine = ExecutorEngine::new("mock");
        engine.register_provider(Arc::new(MockProvider::new(vec![
            MockProvider::simple_response("no store"),
        ])));
        engine.register_agent_config(sample_agent());

        let result = engine
            .execute("agent-run", "hi", &ExecutionConfig::default())
            .await
            .unwrap();
        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.result, "no store");
    }
}
