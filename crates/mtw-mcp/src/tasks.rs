//! Async task primitive (A2A communication).
//!
//! Implements the experimental MCP `tasks/*` methods on top of the existing
//! `ExecutorEngine`. Whereas `mtw_agents_run` blocks the JSON-RPC call until
//! the agent finishes, a task is fire-and-forget: `tasks/create` returns a
//! `task_id` immediately, and the client polls `tasks/get` (or eventually
//! receives a notification over SSE) for the result.
//!
//! This is the same shape as the kernel's `/api/agents/run` (non-blocking
//! with a run_id) — we lift it into the MCP protocol so any client can use
//! it without going around the protocol.
//!
//! State lives in-memory (DashMap) for the lifetime of the MCP process.
//! When the protocol team finalises a persistence story we'll move this to
//! the SQLite store; until then, restarting the server clears tasks just
//! like it clears schedules/chains/triggers/flows.

use crate::agents_ctx::AgentsCtx;
use crate::protocol::TaskProvider;
use async_trait::async_trait;
use dashmap::DashMap;
use mtw_ai::executor::{ExecutionConfig, RunStatus};
use mtw_ai::MtwAgentExecutor;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskRecord {
    pub task_id: String,
    pub kind: String,
    pub agent_id: Option<String>,
    pub goal: Option<String>,
    pub status: TaskStatus,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<String>,
}

pub struct TaskRegistry {
    ctx: AgentsCtx,
    tasks: Arc<DashMap<String, Arc<Mutex<TaskRecord>>>>,
    handles: Arc<DashMap<String, tokio::task::JoinHandle<()>>>,
}

impl TaskRegistry {
    pub fn new(ctx: AgentsCtx) -> Self {
        Self {
            ctx,
            tasks: Arc::new(DashMap::new()),
            handles: Arc::new(DashMap::new()),
        }
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn snapshot(record: &TaskRecord) -> Value {
        serde_json::to_value(record).unwrap_or(json!({}))
    }
}

#[async_trait]
impl TaskProvider for TaskRegistry {
    /// Spawn an async agent run. Params:
    ///   * `kind` — currently only `"agent_run"`. Reserved for future
    ///     task types (skill_run, prompt_run, etc.).
    ///   * `agent_id`, `goal`, plus the same execution knobs as
    ///     `mtw_agents_run`.
    async fn create(&self, params: Value) -> Result<Value, String> {
        let kind = params
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("agent_run")
            .to_string();

        if kind != "agent_run" {
            return Err(format!("unsupported task kind: {}", kind));
        }

        let agent_id = params
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or("missing 'agent_id'")?
            .to_string();
        let goal = params
            .get("goal")
            .and_then(|v| v.as_str())
            .ok_or("missing 'goal'")?
            .to_string();

        let mut config = ExecutionConfig::default();
        if let Some(v) = params.get("max_iterations").and_then(|v| v.as_u64()) {
            config.max_iterations = v as u32;
        }
        if let Some(v) = params.get("timeout_ms").and_then(|v| v.as_u64()) {
            config.timeout_ms = v;
        }
        if let Some(v) = params.get("max_errors").and_then(|v| v.as_u64()) {
            config.max_errors = v as u32;
        }

        let task_id = ulid::Ulid::new().to_string();
        let record = Arc::new(Mutex::new(TaskRecord {
            task_id: task_id.clone(),
            kind,
            agent_id: Some(agent_id.clone()),
            goal: Some(goal.clone()),
            status: TaskStatus::Pending,
            created_at: Self::now(),
            started_at: None,
            finished_at: None,
            result: None,
            error: None,
        }));
        self.tasks.insert(task_id.clone(), record.clone());

        let engine = self.ctx.engine.clone();
        let tasks_map = self.tasks.clone();
        let handles_map = self.handles.clone();
        let tid = task_id.clone();

        let handle = tokio::spawn(async move {
            {
                let mut r = record.lock().await;
                r.status = TaskStatus::Running;
                r.started_at = Some(Self::now());
            }

            let outcome = engine.execute(&agent_id, &goal, &config).await;

            let mut r = record.lock().await;
            r.finished_at = Some(Self::now());
            match outcome {
                Ok(result) => {
                    r.status = match result.status {
                        RunStatus::Failed => TaskStatus::Failed,
                        RunStatus::Cancelled => TaskStatus::Cancelled,
                        _ => TaskStatus::Completed,
                    };
                    r.result = Some(json!({
                        "status": result.status,
                        "result": result.result,
                        "error": result.error,
                        "steps_count": result.steps_count,
                        "tokens_used": result.tokens_used,
                    }));
                }
                Err(e) => {
                    r.status = TaskStatus::Failed;
                    r.error = Some(e.to_string());
                }
            }
            // Detach the join handle once we're done.
            handles_map.remove(&tid);
            // Keep the task record around for `tasks/get`.
            drop(tasks_map);
        });

        self.handles.insert(task_id.clone(), handle);

        let r = self.tasks.get(&task_id).unwrap();
        let snapshot = Self::snapshot(&*r.value().lock().await);
        Ok(json!({ "task": snapshot }))
    }

    async fn get(&self, task_id: &str) -> Result<Value, String> {
        let entry = self
            .tasks
            .get(task_id)
            .ok_or_else(|| format!("task not found: {}", task_id))?;
        let record = entry.value().lock().await;
        Ok(json!({ "task": Self::snapshot(&*record) }))
    }

    async fn list(&self, filter: Value) -> Result<Value, String> {
        let limit = filter
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(50);
        let status_filter = filter
            .get("status")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_value::<TaskStatus>(json!(s)).ok());

        let mut out: Vec<Value> = Vec::new();
        for entry in self.tasks.iter() {
            let r = entry.value().lock().await;
            if let Some(s) = &status_filter {
                if r.status != *s {
                    continue;
                }
            }
            out.push(Self::snapshot(&*r));
            if out.len() >= limit {
                break;
            }
        }
        Ok(json!({ "tasks": out, "total": out.len() }))
    }

    async fn cancel(&self, task_id: &str) -> Result<Value, String> {
        let entry = self
            .tasks
            .get(task_id)
            .ok_or_else(|| format!("task not found: {}", task_id))?;
        // Abort the join handle, then mark cancelled. We don't have
        // co-operative cancellation in ExecutorEngine yet, so this is
        // best-effort: the agent thread sees its task aborted.
        if let Some((_, handle)) = self.handles.remove(task_id) {
            handle.abort();
        }
        let mut r = entry.value().lock().await;
        if matches!(r.status, TaskStatus::Pending | TaskStatus::Running) {
            r.status = TaskStatus::Cancelled;
            r.finished_at = Some(Self::now());
        }
        Ok(json!({ "task": Self::snapshot(&*r) }))
    }
}
