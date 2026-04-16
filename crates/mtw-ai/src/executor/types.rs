use async_trait::async_trait;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::trigger::TriggerType;

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

/// A tool that can be invoked by the executor during a run.
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
