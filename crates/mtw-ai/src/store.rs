//! Agent persistence trait.
//!
//! `AgentStore` is the backend-agnostic contract for saving agents, runs,
//! steps, and conversational memory. The executor calls it through
//! `Arc<dyn AgentStore>` so the same engine can run on SQLite today and
//! any other backend (Postgres, Redis, in-memory) later.
//!
//! All methods return `MtwError` on failure. Implementations must be
//! `Send + Sync` since the executor is shared across Tokio tasks.

use async_trait::async_trait;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};

use crate::executor::{AgentConfig, AgentRun, AgentStep, RunStatus};

/// Configuration for a SQLite-backed `AgentStore`.
///
/// Minimal form (just a path) works from `mtw.toml`:
/// ```toml
/// [agents.store]
/// path = "./data/agents.db"
/// ```
///
/// The path can be overridden at runtime via the `MTW_AGENTS_DB`
/// environment variable, which takes precedence over the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStoreConfig {
    /// Path to the SQLite database file. Parent directories must exist
    /// (the file itself is auto-created on first open).
    pub path: String,

    /// Connection pool size. Default: 4.
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Busy timeout in milliseconds. Default: 5000.
    #[serde(default = "default_busy_timeout")]
    pub busy_timeout_ms: u64,
}

fn default_pool_size() -> u32 {
    4
}
fn default_busy_timeout() -> u64 {
    5000
}

impl Default for AgentStoreConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            pool_size: default_pool_size(),
            busy_timeout_ms: default_busy_timeout(),
        }
    }
}

impl AgentStoreConfig {
    /// Build a config from a path, with env var override.
    ///
    /// If `MTW_AGENTS_DB` is set in the environment, its value replaces
    /// the supplied path — useful for overriding the `mtw.toml` setting
    /// without editing the file (CI, Docker, dev loops).
    pub fn with_env_override(mut self) -> Self {
        if let Ok(p) = std::env::var("MTW_AGENTS_DB") {
            if !p.is_empty() {
                self.path = p;
            }
        }
        self
    }
}

/// Role of a memory entry (conversation turn the agent should remember).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRole {
    User,
    Assistant,
    System,
}

/// A single memory entry tied to an agent (and optionally a run).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMemoryRecord {
    pub id: String,
    pub agent_id: String,
    pub role: MemoryRole,
    pub content: String,
    pub run_id: String,
    pub created_at: String,
}

/// Partial update for an in-progress or completed run.
///
/// Fields set to `Some` are written; fields left `None` are untouched.
/// This lets the executor update status independently of counts or results
/// without clobbering concurrent writes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunUpdate {
    pub status: Option<RunStatus>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub steps_count: Option<u32>,
    pub tokens_used: Option<u32>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// Filter for `list_runs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunFilter {
    pub agent_id: Option<String>,
    pub status: Option<RunStatus>,
    pub limit: Option<u32>,
}

/// Persistent backend for agents, runs, steps, and memory.
///
/// Implementations must be cheap to clone behind `Arc` and safe to call
/// concurrently from multiple tokio tasks.
#[async_trait]
pub trait AgentStore: Send + Sync {
    // -- agents -------------------------------------------------------------

    /// Upsert an agent configuration. Insert on first write, update otherwise.
    async fn save_agent(&self, agent: &AgentConfig) -> Result<(), MtwError>;

    /// Fetch an agent by ID. Returns `None` if not found.
    async fn get_agent(&self, agent_id: &str) -> Result<Option<AgentConfig>, MtwError>;

    /// List all agents.
    async fn list_agents(&self) -> Result<Vec<AgentConfig>, MtwError>;

    /// Delete an agent and cascade to its runs/steps/memory.
    async fn delete_agent(&self, agent_id: &str) -> Result<(), MtwError>;

    // -- runs ---------------------------------------------------------------

    /// Persist a newly-created run.
    async fn create_run(&self, run: &AgentRun) -> Result<(), MtwError>;

    /// Apply a partial update to an existing run.
    async fn update_run(&self, run_id: &str, update: &RunUpdate) -> Result<(), MtwError>;

    /// Fetch a run by ID.
    async fn get_run(&self, run_id: &str) -> Result<Option<AgentRun>, MtwError>;

    /// List runs matching the filter, newest first.
    async fn list_runs(&self, filter: &RunFilter) -> Result<Vec<AgentRun>, MtwError>;

    // -- steps --------------------------------------------------------------

    /// Append a step to a run.
    async fn add_step(&self, step: &AgentStep) -> Result<(), MtwError>;

    /// List steps for a run in order.
    async fn list_steps(&self, run_id: &str) -> Result<Vec<AgentStep>, MtwError>;

    // -- memory -------------------------------------------------------------

    /// Append a memory entry.
    async fn add_memory(&self, memory: &AgentMemoryRecord) -> Result<(), MtwError>;

    /// Get the N most recent memory entries for an agent.
    /// Ordering is newest first; callers typically reverse before injection.
    async fn recent_memory(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> Result<Vec<AgentMemoryRecord>, MtwError>;
}
