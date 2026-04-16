//! Wiring between the MCP server and the real mtw-ai engine.
//!
//! `AgentsCtx` owns every piece of state that the `mtw_agents_*` tools
//! read or mutate: the `ExecutorEngine`, a persistent `AgentStore`, and
//! the in-memory registries for chains, schedules, triggers, and flows.
//!
//! Providers are registered conditionally from environment variables so
//! the MCP server starts cleanly even when only some credentials are
//! available (e.g. laptop dev with only `ANTHROPIC_API_KEY` set).

use std::path::Path;
use std::sync::Arc;

use mtw_ai::chain::ChainRegistry;
use mtw_ai::executor::ExecutorEngine;
use mtw_ai::flow::FlowManager;
use mtw_ai::providers::anthropic::{AnthropicConfig, AnthropicProvider};
use mtw_ai::providers::ollama::{OllamaConfig, OllamaProvider};
use mtw_ai::providers::openai::{OpenAIConfig, OpenAIProvider};
use mtw_ai::schedule::ScheduleManager;
use mtw_ai::store::{AgentStore, AgentStoreConfig};
use mtw_ai::store_sqlite::SqliteAgentStore;
use mtw_ai::trigger::TriggerRegistry;
use mtw_core::MtwError;

/// Shared state for all `mtw_agents_*` MCP tool handlers.
///
/// Cloned into every handler closure behind `Arc` — all fields are
/// themselves `Arc<...>` so cloning the context is a refcount bump,
/// not a deep copy.
#[derive(Clone)]
pub struct AgentsCtx {
    pub engine: Arc<ExecutorEngine>,
    pub store: Arc<dyn AgentStore>,
    pub chains: Arc<ChainRegistry>,
    pub schedules: Arc<ScheduleManager>,
    pub triggers: Arc<TriggerRegistry>,
    pub flows: Arc<FlowManager>,
}

impl AgentsCtx {
    /// Bootstrap from environment:
    ///   * `MTW_AGENTS_DB`    — SQLite path (default: `./data/agents.db`)
    ///   * `MTW_DEFAULT_PROVIDER` — default provider name (default: `anthropic`)
    ///   * `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` — enable remote providers
    ///   * `OLLAMA_BASE_URL`  — register Ollama (default: `http://localhost:11434`)
    ///
    /// Providers without credentials simply aren't registered; calls to
    /// `mtw_agents_run` against an unregistered provider will fail with
    /// a clear "provider not found" error.
    pub fn bootstrap() -> Result<Self, MtwError> {
        let path = std::env::var("MTW_AGENTS_DB")
            .unwrap_or_else(|_| "./data/agents.db".to_string());

        if let Some(parent) = Path::new(&path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        let store_cfg = AgentStoreConfig {
            path,
            ..Default::default()
        }
        .with_env_override();

        let store: Arc<dyn AgentStore> = Arc::new(SqliteAgentStore::open(&store_cfg)?);

        let chains = Arc::new(ChainRegistry::new());
        let schedules = Arc::new(ScheduleManager::new());
        let triggers = Arc::new(TriggerRegistry::new());
        let flows = Arc::new(FlowManager::new());

        let default_provider =
            std::env::var("MTW_DEFAULT_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());

        let engine = ExecutorEngine::new(default_provider)
            .with_store(store.clone())
            .with_chain_registry(chains.clone());

        // Register providers conditionally on credentials/availability.
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                engine.register_provider(Arc::new(AnthropicProvider::new(
                    AnthropicConfig::new(key),
                )));
            }
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if !key.is_empty() {
                engine.register_provider(Arc::new(OpenAIProvider::new(
                    OpenAIConfig::new(key),
                )));
            }
        }
        let ollama_cfg = if let Ok(url) = std::env::var("OLLAMA_BASE_URL") {
            OllamaConfig::default().with_base_url(url)
        } else {
            OllamaConfig::default()
        };
        engine.register_provider(Arc::new(OllamaProvider::new(ollama_cfg)));

        Ok(Self {
            engine: Arc::new(engine),
            store,
            chains,
            schedules,
            triggers,
            flows,
        })
    }
}
