use async_trait::async_trait;
use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
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
use crate::trigger::TriggerType;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Status of an agent execution run
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Type of a single step within a run
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    Thought,
    ToolCall,
    ToolResult,
    Error,
    Final,
}

/// Record of a full agent execution run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: String,
    pub agent_id: String,
    pub trigger_type: TriggerType,
    pub trigger_payload: Value,
    pub goal: String,
    pub status: RunStatus,
    pub result: String,
    pub error: String,
    pub steps_count: u32,
    pub tokens_used: u32,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
}

/// A single step within an agent run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStep {
    pub id: String,
    pub run_id: String,
    pub step_number: u32,
    pub step_type: StepType,
    pub content: String,
    pub tool_name: String,
    pub tool_input: Value,
    pub tool_output: String,
    pub tokens: u32,
    pub created_at: String,
}

/// Configuration for a single execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub max_iterations: u32,
    pub timeout_ms: u64,
    pub max_chain_depth: u32,
    pub max_errors: u32,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_iterations: 15,
            timeout_ms: 300_000,
            max_chain_depth: 5,
            max_errors: 3,
        }
    }
}

/// Result returned after an execution completes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub status: RunStatus,
    pub result: String,
    pub error: String,
    pub steps_count: u32,
    pub tokens_used: u32,
}

/// Trait for agent executors
#[async_trait]
pub trait MtwAgentExecutor: Send + Sync {
    /// Execute an agent with the given goal
    async fn execute(
        &self,
        agent_id: &str,
        goal: &str,
        config: &ExecutionConfig,
    ) -> Result<ExecutionResult, MtwError>;

    /// Cancel an active run; returns true if successfully cancelled
    fn cancel_run(&self, run_id: &str) -> bool;

    /// List IDs of all currently active runs
    fn active_runs(&self) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// ToolDefinition -- callable tool with async handler
// ---------------------------------------------------------------------------

/// A tool that can be invoked by the executor during a run.
///
/// The `handler` is an async function that receives the tool arguments as
/// [`serde_json::Value`] and returns a result value or an error.
pub struct ToolDefinition {
    /// Tool name (must be unique within the executor)
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// JSON Schema describing the tool parameters
    pub parameters: Value,
    /// Async handler invoked when the LLM requests this tool
    pub handler:
        Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, MtwError>> + Send>> + Send + Sync>,
}

impl std::fmt::Debug for ToolDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolDefinition")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// AgentConfig -- lightweight config passed into the executor
// ---------------------------------------------------------------------------

/// Minimal agent configuration needed by the executor to set up the LLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique agent identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Provider key (must match a registered MtwAIProvider)
    pub provider: String,
    /// Model identifier passed to the provider
    pub model: String,
    /// System prompt template (supports `{{variable}}` interpolation)
    pub system_prompt: String,
    /// List of tool names this agent is allowed to use
    pub tool_names: Vec<String>,
    /// Optional token budget (0 = unlimited)
    #[serde(default)]
    pub token_budget: u32,
}

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

        // Collect tool definitions
        let tool_defs = self.collect_tool_defs(agent_config);

        // Initialize messages
        let mut messages: Vec<Message> = vec![
            Message::system(&system_prompt),
            Message::user(goal),
        ];

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

            // Build completion request
            let req = CompletionRequest {
                model: agent_config.model.clone(),
                messages: messages.clone(),
                tools: if tool_defs.is_empty() {
                    None
                } else {
                    Some(tool_defs.clone())
                },
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
                    steps.push(Self::make_step(
                        run_id,
                        step_number,
                        StepType::Error,
                        &format!("LLM call failed: {}", e),
                        "",
                        Value::Null,
                        "",
                        0,
                    ));
                    continue;
                }
            };

            tokens_used += response.usage.total_tokens;

            // No tool calls -- final answer
            if response.tool_calls.is_empty() {
                step_number += 1;
                steps.push(Self::make_step(
                    run_id,
                    step_number,
                    StepType::Final,
                    &response.content,
                    "",
                    Value::Null,
                    "",
                    response.usage.total_tokens,
                ));

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
                steps.push(Self::make_step(
                    run_id,
                    step_number,
                    StepType::Thought,
                    &response.content,
                    "",
                    Value::Null,
                    "",
                    0,
                ));
                messages.push(Message::assistant(&response.content));
            }

            // Process each tool call
            let mut tool_results: Vec<ToolResult> = Vec::new();

            for tc in &response.tool_calls {
                let (result_value, is_error) = self.invoke_tool(tc).await;

                let result_str = match serde_json::to_string(&result_value) {
                    Ok(s) => s,
                    Err(_) => result_value.to_string(),
                };

                // Record tool_call step
                step_number += 1;
                steps.push(Self::make_step(
                    run_id,
                    step_number,
                    StepType::ToolCall,
                    "",
                    &tc.name,
                    tc.arguments.clone(),
                    "",
                    0,
                ));

                // Record tool_result step
                step_number += 1;
                steps.push(Self::make_step(
                    run_id,
                    step_number,
                    StepType::ToolResult,
                    "",
                    &tc.name,
                    Value::Null,
                    &result_str,
                    0,
                ));

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
        // Look up agent config
        let agent_config = self
            .agent_configs
            .get(agent_id)
            .map(|r| r.value().clone())
            .ok_or_else(|| {
                MtwError::Agent(format!("agent config not found: {}", agent_id))
            })?;

        let run_id = ulid::Ulid::new().to_string();

        tracing::info!(
            run_id = %run_id,
            agent_id = %agent_id,
            goal = %goal,
            "starting agent execution"
        );

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
}
