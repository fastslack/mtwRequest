//! SQLite-backed [`AgentStore`].
//!
//! Uses `r2d2` for connection pooling and runs every rusqlite call inside
//! `tokio::task::spawn_blocking` so the async executor stays responsive.
//!
//! The schema is owned by [`crate::store_migrations`]; this module only
//! persists and reads back records through it.

use async_trait::async_trait;
use mtw_core::MtwError;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, OpenFlags};

use crate::executor::{AgentConfig, AgentRun, AgentStep, RunStatus, StepType};
use crate::store::{
    AgentMemoryRecord, AgentStore, AgentStoreConfig, MemoryRole, RunFilter, RunUpdate,
};
use crate::store_migrations;

/// SQLite-backed implementation of [`AgentStore`].
pub struct SqliteAgentStore {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteAgentStore {
    /// Open (or create) the agents database and apply pending migrations.
    pub fn open(config: &AgentStoreConfig) -> Result<Self, MtwError> {
        if config.path.is_empty() {
            return Err(MtwError::Config(
                "agent store path is empty (set [agents.store].path or MTW_AGENTS_DB)".into(),
            ));
        }

        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;

        let manager = SqliteConnectionManager::file(&config.path).with_flags(flags);
        let pool = Pool::builder()
            .max_size(config.pool_size)
            .build(manager)
            .map_err(|e| MtwError::Config(format!("agent store pool: {}", e)))?;

        let conn = pool
            .get()
            .map_err(|e| MtwError::Config(format!("agent store conn: {}", e)))?;

        let pragmas = format!(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = {timeout};
             PRAGMA temp_store = MEMORY;",
            timeout = config.busy_timeout_ms,
        );
        conn.execute_batch(&pragmas)
            .map_err(|e| MtwError::Config(format!("agent store pragmas: {}", e)))?;

        store_migrations::apply(&conn)
            .map_err(|e| MtwError::Config(format!("agent store migrate: {}", e)))?;

        tracing::info!(path = %config.path, "agent store opened");

        Ok(Self { pool })
    }
}

// ── serde helpers for enum <-> TEXT column ────────────────────────────

fn status_to_str(s: &RunStatus) -> &'static str {
    match s {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
    }
}

fn str_to_status(s: &str) -> RunStatus {
    match s {
        "running" => RunStatus::Running,
        "completed" => RunStatus::Completed,
        "failed" => RunStatus::Failed,
        "cancelled" => RunStatus::Cancelled,
        _ => RunStatus::Pending,
    }
}

fn step_type_to_str(s: &StepType) -> &'static str {
    match s {
        StepType::Thought => "thought",
        StepType::ToolCall => "tool_call",
        StepType::ToolResult => "tool_result",
        StepType::Error => "error",
        StepType::Final => "final",
    }
}

fn str_to_step_type(s: &str) -> StepType {
    match s {
        "tool_call" => StepType::ToolCall,
        "tool_result" => StepType::ToolResult,
        "error" => StepType::Error,
        "final" => StepType::Final,
        _ => StepType::Thought,
    }
}

fn role_to_str(r: &MemoryRole) -> &'static str {
    match r {
        MemoryRole::User => "user",
        MemoryRole::Assistant => "assistant",
        MemoryRole::System => "system",
    }
}

fn str_to_role(s: &str) -> MemoryRole {
    match s {
        "assistant" => MemoryRole::Assistant,
        "system" => MemoryRole::System,
        _ => MemoryRole::User,
    }
}

fn trigger_type_to_str(t: &crate::trigger::TriggerType) -> String {
    // TriggerType is serde snake_case — round-trip through JSON for stability.
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "manual".to_string())
}

fn str_to_trigger_type(s: &str) -> crate::trigger::TriggerType {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .unwrap_or(crate::trigger::TriggerType::Manual)
}

fn now_iso() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

// ── AgentStore impl ──────────────────────────────────────────────────

#[async_trait]
impl AgentStore for SqliteAgentStore {
    async fn save_agent(&self, agent: &AgentConfig) -> Result<(), MtwError> {
        let pool = self.pool.clone();
        let agent = agent.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            let now = now_iso();
            let tool_names = serde_json::to_string(&agent.tool_names)
                .unwrap_or_else(|_| "[]".to_string());

            conn.execute(
                "INSERT INTO agents
                   (id, name, provider, model, system_prompt, tool_names, token_budget,
                    created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    provider = excluded.provider,
                    model = excluded.model,
                    system_prompt = excluded.system_prompt,
                    tool_names = excluded.tool_names,
                    token_budget = excluded.token_budget,
                    updated_at = excluded.updated_at",
                params![
                    agent.id,
                    agent.name,
                    agent.provider,
                    agent.model,
                    agent.system_prompt,
                    tool_names,
                    agent.token_budget as i64,
                    now,
                ],
            )
            .map_err(|e| MtwError::Internal(format!("save_agent: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn get_agent(&self, agent_id: &str) -> Result<Option<AgentConfig>, MtwError> {
        let pool = self.pool.clone();
        let id = agent_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, provider, model, system_prompt, tool_names, token_budget
                     FROM agents WHERE id = ?1",
                )
                .map_err(|e| MtwError::Internal(format!("prepare: {}", e)))?;

            let mut rows = stmt
                .query(params![id])
                .map_err(|e| MtwError::Internal(format!("query: {}", e)))?;

            if let Some(row) = rows
                .next()
                .map_err(|e| MtwError::Internal(format!("row: {}", e)))?
            {
                Ok(Some(row_to_agent(row)?))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn list_agents(&self) -> Result<Vec<AgentConfig>, MtwError> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, provider, model, system_prompt, tool_names, token_budget
                     FROM agents ORDER BY name",
                )
                .map_err(|e| MtwError::Internal(format!("prepare: {}", e)))?;

            let rows = stmt
                .query_map([], |row| {
                    row_to_agent(row).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                e.to_string(),
                            )),
                        )
                    })
                })
                .map_err(|e| MtwError::Internal(format!("query_map: {}", e)))?;

            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| MtwError::Internal(format!("row: {}", e)))?);
            }
            Ok(out)
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn delete_agent(&self, agent_id: &str) -> Result<(), MtwError> {
        let pool = self.pool.clone();
        let id = agent_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| MtwError::Internal(format!("tx: {}", e)))?;

            tx.execute(
                "DELETE FROM agent_steps WHERE run_id IN (SELECT id FROM agent_runs WHERE agent_id = ?1)",
                params![id],
            )
            .map_err(|e| MtwError::Internal(format!("delete steps: {}", e)))?;
            tx.execute("DELETE FROM agent_runs WHERE agent_id = ?1", params![id])
                .map_err(|e| MtwError::Internal(format!("delete runs: {}", e)))?;
            tx.execute("DELETE FROM agent_memory WHERE agent_id = ?1", params![id])
                .map_err(|e| MtwError::Internal(format!("delete memory: {}", e)))?;
            tx.execute("DELETE FROM agents WHERE id = ?1", params![id])
                .map_err(|e| MtwError::Internal(format!("delete agent: {}", e)))?;

            tx.commit()
                .map_err(|e| MtwError::Internal(format!("commit: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn create_run(&self, run: &AgentRun) -> Result<(), MtwError> {
        let pool = self.pool.clone();
        let run = run.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            let trigger_payload = serde_json::to_string(&run.trigger_payload)
                .unwrap_or_else(|_| "{}".to_string());

            conn.execute(
                "INSERT INTO agent_runs
                   (id, agent_id, trigger_type, trigger_payload, goal, status, result, error,
                    steps_count, tokens_used, started_at, completed_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    run.id,
                    run.agent_id,
                    trigger_type_to_str(&run.trigger_type),
                    trigger_payload,
                    run.goal,
                    status_to_str(&run.status),
                    run.result,
                    run.error,
                    run.steps_count as i64,
                    run.tokens_used as i64,
                    run.started_at,
                    run.completed_at,
                    run.created_at,
                ],
            )
            .map_err(|e| MtwError::Internal(format!("create_run: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn update_run(&self, run_id: &str, update: &RunUpdate) -> Result<(), MtwError> {
        let pool = self.pool.clone();
        let id = run_id.to_string();
        let update = update.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;

            // Build dynamic SET clause; only update fields the caller set.
            let mut sets: Vec<&str> = Vec::new();
            let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(s) = &update.status {
                sets.push("status = ?");
                vals.push(Box::new(status_to_str(s).to_string()));
            }
            if let Some(v) = &update.result {
                sets.push("result = ?");
                vals.push(Box::new(v.clone()));
            }
            if let Some(v) = &update.error {
                sets.push("error = ?");
                vals.push(Box::new(v.clone()));
            }
            if let Some(v) = update.steps_count {
                sets.push("steps_count = ?");
                vals.push(Box::new(v as i64));
            }
            if let Some(v) = update.tokens_used {
                sets.push("tokens_used = ?");
                vals.push(Box::new(v as i64));
            }
            if let Some(v) = &update.started_at {
                sets.push("started_at = ?");
                vals.push(Box::new(v.clone()));
            }
            if let Some(v) = &update.completed_at {
                sets.push("completed_at = ?");
                vals.push(Box::new(v.clone()));
            }

            if sets.is_empty() {
                return Ok(());
            }

            let sql = format!("UPDATE agent_runs SET {} WHERE id = ?", sets.join(", "));
            vals.push(Box::new(id));

            let refs: Vec<&dyn rusqlite::types::ToSql> =
                vals.iter().map(|b| b.as_ref()).collect();

            conn.execute(&sql, refs.as_slice())
                .map_err(|e| MtwError::Internal(format!("update_run: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<AgentRun>, MtwError> {
        let pool = self.pool.clone();
        let id = run_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, agent_id, trigger_type, trigger_payload, goal, status, result,
                            error, steps_count, tokens_used, started_at, completed_at, created_at
                     FROM agent_runs WHERE id = ?1",
                )
                .map_err(|e| MtwError::Internal(format!("prepare: {}", e)))?;

            let mut rows = stmt
                .query(params![id])
                .map_err(|e| MtwError::Internal(format!("query: {}", e)))?;

            if let Some(row) = rows
                .next()
                .map_err(|e| MtwError::Internal(format!("row: {}", e)))?
            {
                Ok(Some(row_to_run(row)?))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn list_runs(&self, filter: &RunFilter) -> Result<Vec<AgentRun>, MtwError> {
        let pool = self.pool.clone();
        let filter = filter.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;

            let mut sql = String::from(
                "SELECT id, agent_id, trigger_type, trigger_payload, goal, status, result,
                        error, steps_count, tokens_used, started_at, completed_at, created_at
                 FROM agent_runs",
            );
            let mut clauses: Vec<&str> = Vec::new();
            let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(aid) = &filter.agent_id {
                clauses.push("agent_id = ?");
                vals.push(Box::new(aid.clone()));
            }
            if let Some(s) = &filter.status {
                clauses.push("status = ?");
                vals.push(Box::new(status_to_str(s).to_string()));
            }
            if !clauses.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&clauses.join(" AND "));
            }
            sql.push_str(" ORDER BY created_at DESC");
            if let Some(lim) = filter.limit {
                sql.push_str(" LIMIT ?");
                vals.push(Box::new(lim as i64));
            }

            let refs: Vec<&dyn rusqlite::types::ToSql> =
                vals.iter().map(|b| b.as_ref()).collect();

            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| MtwError::Internal(format!("prepare: {}", e)))?;

            let rows = stmt
                .query_map(refs.as_slice(), |row| {
                    row_to_run(row).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                e.to_string(),
                            )),
                        )
                    })
                })
                .map_err(|e| MtwError::Internal(format!("query_map: {}", e)))?;

            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| MtwError::Internal(format!("row: {}", e)))?);
            }
            Ok(out)
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn add_step(&self, step: &AgentStep) -> Result<(), MtwError> {
        let pool = self.pool.clone();
        let step = step.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            let tool_input = serde_json::to_string(&step.tool_input)
                .unwrap_or_else(|_| "{}".to_string());

            conn.execute(
                "INSERT INTO agent_steps
                   (id, run_id, step_number, step_type, content, tool_name, tool_input,
                    tool_output, tokens, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    step.id,
                    step.run_id,
                    step.step_number as i64,
                    step_type_to_str(&step.step_type),
                    step.content,
                    step.tool_name,
                    tool_input,
                    step.tool_output,
                    step.tokens as i64,
                    step.created_at,
                ],
            )
            .map_err(|e| MtwError::Internal(format!("add_step: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn list_steps(&self, run_id: &str) -> Result<Vec<AgentStep>, MtwError> {
        let pool = self.pool.clone();
        let id = run_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, run_id, step_number, step_type, content, tool_name, tool_input,
                            tool_output, tokens, created_at
                     FROM agent_steps
                     WHERE run_id = ?1 ORDER BY step_number",
                )
                .map_err(|e| MtwError::Internal(format!("prepare: {}", e)))?;

            let rows = stmt
                .query_map(params![id], |row| {
                    row_to_step(row).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                e.to_string(),
                            )),
                        )
                    })
                })
                .map_err(|e| MtwError::Internal(format!("query_map: {}", e)))?;

            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| MtwError::Internal(format!("row: {}", e)))?);
            }
            Ok(out)
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn add_memory(&self, memory: &AgentMemoryRecord) -> Result<(), MtwError> {
        let pool = self.pool.clone();
        let m = memory.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            conn.execute(
                "INSERT INTO agent_memory
                   (id, agent_id, role, content, run_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    m.id,
                    m.agent_id,
                    role_to_str(&m.role),
                    m.content,
                    m.run_id,
                    m.created_at,
                ],
            )
            .map_err(|e| MtwError::Internal(format!("add_memory: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn recent_memory(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> Result<Vec<AgentMemoryRecord>, MtwError> {
        let pool = self.pool.clone();
        let id = agent_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, agent_id, role, content, run_id, created_at
                     FROM agent_memory
                     WHERE agent_id = ?1
                     ORDER BY created_at DESC LIMIT ?2",
                )
                .map_err(|e| MtwError::Internal(format!("prepare: {}", e)))?;

            let rows = stmt
                .query_map(params![id, limit as i64], |row| {
                    Ok(AgentMemoryRecord {
                        id: row.get(0)?,
                        agent_id: row.get(1)?,
                        role: str_to_role(&row.get::<_, String>(2)?),
                        content: row.get(3)?,
                        run_id: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })
                .map_err(|e| MtwError::Internal(format!("query_map: {}", e)))?;

            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| MtwError::Internal(format!("row: {}", e)))?);
            }
            Ok(out)
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }
}

// ── row -> struct helpers ─────────────────────────────────────────────

fn row_to_agent(row: &rusqlite::Row<'_>) -> Result<AgentConfig, MtwError> {
    let tool_names_json: String = row
        .get(5)
        .map_err(|e| MtwError::Internal(format!("tool_names: {}", e)))?;
    let tool_names: Vec<String> =
        serde_json::from_str(&tool_names_json).unwrap_or_default();
    Ok(AgentConfig {
        id: row.get(0).map_err(|e| MtwError::Internal(format!("id: {}", e)))?,
        name: row.get(1).map_err(|e| MtwError::Internal(format!("name: {}", e)))?,
        provider: row.get(2).map_err(|e| MtwError::Internal(format!("provider: {}", e)))?,
        model: row.get(3).map_err(|e| MtwError::Internal(format!("model: {}", e)))?,
        system_prompt: row
            .get(4)
            .map_err(|e| MtwError::Internal(format!("system_prompt: {}", e)))?,
        tool_names,
        token_budget: row
            .get::<_, i64>(6)
            .map_err(|e| MtwError::Internal(format!("token_budget: {}", e)))? as u32,
    })
}

fn row_to_run(row: &rusqlite::Row<'_>) -> Result<AgentRun, MtwError> {
    let trigger_type_str: String = row
        .get(2)
        .map_err(|e| MtwError::Internal(format!("trigger_type: {}", e)))?;
    let trigger_payload_str: String = row
        .get(3)
        .map_err(|e| MtwError::Internal(format!("trigger_payload: {}", e)))?;
    let status_str: String = row
        .get(5)
        .map_err(|e| MtwError::Internal(format!("status: {}", e)))?;

    Ok(AgentRun {
        id: row.get(0).map_err(|e| MtwError::Internal(format!("id: {}", e)))?,
        agent_id: row.get(1).map_err(|e| MtwError::Internal(format!("agent_id: {}", e)))?,
        trigger_type: str_to_trigger_type(&trigger_type_str),
        trigger_payload: serde_json::from_str(&trigger_payload_str)
            .unwrap_or(serde_json::Value::Null),
        goal: row.get(4).map_err(|e| MtwError::Internal(format!("goal: {}", e)))?,
        status: str_to_status(&status_str),
        result: row.get(6).map_err(|e| MtwError::Internal(format!("result: {}", e)))?,
        error: row.get(7).map_err(|e| MtwError::Internal(format!("error: {}", e)))?,
        steps_count: row
            .get::<_, i64>(8)
            .map_err(|e| MtwError::Internal(format!("steps_count: {}", e)))? as u32,
        tokens_used: row
            .get::<_, i64>(9)
            .map_err(|e| MtwError::Internal(format!("tokens_used: {}", e)))? as u32,
        started_at: row.get(10).ok(),
        completed_at: row.get(11).ok(),
        created_at: row
            .get(12)
            .map_err(|e| MtwError::Internal(format!("created_at: {}", e)))?,
    })
}

fn row_to_step(row: &rusqlite::Row<'_>) -> Result<AgentStep, MtwError> {
    let step_type_str: String = row
        .get(3)
        .map_err(|e| MtwError::Internal(format!("step_type: {}", e)))?;
    let tool_input_str: String = row
        .get(6)
        .map_err(|e| MtwError::Internal(format!("tool_input: {}", e)))?;
    Ok(AgentStep {
        id: row.get(0).map_err(|e| MtwError::Internal(format!("id: {}", e)))?,
        run_id: row.get(1).map_err(|e| MtwError::Internal(format!("run_id: {}", e)))?,
        step_number: row
            .get::<_, i64>(2)
            .map_err(|e| MtwError::Internal(format!("step_number: {}", e)))? as u32,
        step_type: str_to_step_type(&step_type_str),
        content: row.get(4).map_err(|e| MtwError::Internal(format!("content: {}", e)))?,
        tool_name: row.get(5).map_err(|e| MtwError::Internal(format!("tool_name: {}", e)))?,
        tool_input: serde_json::from_str(&tool_input_str).unwrap_or(serde_json::Value::Null),
        tool_output: row.get(7).map_err(|e| MtwError::Internal(format!("tool_output: {}", e)))?,
        tokens: row
            .get::<_, i64>(8)
            .map_err(|e| MtwError::Internal(format!("tokens: {}", e)))? as u32,
        created_at: row
            .get(9)
            .map_err(|e| MtwError::Internal(format!("created_at: {}", e)))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger::TriggerType;
    use tempfile::NamedTempFile;

    fn temp_store() -> SqliteAgentStore {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().to_string();
        // Keep the file alive for the duration of the test by leaking its handle.
        std::mem::forget(file);
        let cfg = AgentStoreConfig {
            path,
            ..Default::default()
        };
        SqliteAgentStore::open(&cfg).expect("open store")
    }

    fn sample_agent() -> AgentConfig {
        AgentConfig {
            id: "agent-1".into(),
            name: "Test".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4".into(),
            system_prompt: "be helpful".into(),
            tool_names: vec!["search".into(), "fetch".into()],
            token_budget: 10_000,
        }
    }

    #[tokio::test]
    async fn open_is_idempotent() {
        let store = temp_store();
        // Reopening the same file (via recent_memory) should not fail.
        let _ = store.recent_memory("nope", 5).await.unwrap();
    }

    #[tokio::test]
    async fn save_and_fetch_agent() {
        let store = temp_store();
        let a = sample_agent();
        store.save_agent(&a).await.unwrap();

        let got = store.get_agent("agent-1").await.unwrap().unwrap();
        assert_eq!(got.id, "agent-1");
        assert_eq!(got.name, "Test");
        assert_eq!(got.tool_names, vec!["search".to_string(), "fetch".into()]);
        assert_eq!(got.token_budget, 10_000);
    }

    #[tokio::test]
    async fn save_agent_is_upsert() {
        let store = temp_store();
        let mut a = sample_agent();
        store.save_agent(&a).await.unwrap();
        a.name = "Renamed".into();
        store.save_agent(&a).await.unwrap();

        let got = store.get_agent("agent-1").await.unwrap().unwrap();
        assert_eq!(got.name, "Renamed");
    }

    #[tokio::test]
    async fn list_agents_sorted() {
        let store = temp_store();
        store
            .save_agent(&AgentConfig {
                id: "b".into(),
                name: "Beta".into(),
                ..sample_agent()
            })
            .await
            .unwrap();
        store
            .save_agent(&AgentConfig {
                id: "a".into(),
                name: "Alpha".into(),
                ..sample_agent()
            })
            .await
            .unwrap();
        let xs = store.list_agents().await.unwrap();
        assert_eq!(xs.len(), 2);
        assert_eq!(xs[0].name, "Alpha");
    }

    #[tokio::test]
    async fn run_lifecycle() {
        let store = temp_store();
        store.save_agent(&sample_agent()).await.unwrap();

        let run = AgentRun {
            id: "run-1".into(),
            agent_id: "agent-1".into(),
            trigger_type: TriggerType::Manual,
            trigger_payload: serde_json::json!({}),
            goal: "do the thing".into(),
            status: RunStatus::Pending,
            result: String::new(),
            error: String::new(),
            steps_count: 0,
            tokens_used: 0,
            started_at: None,
            completed_at: None,
            created_at: now_iso(),
        };
        store.create_run(&run).await.unwrap();

        store
            .update_run(
                "run-1",
                &RunUpdate {
                    status: Some(RunStatus::Running),
                    started_at: Some("t1".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let got = store.get_run("run-1").await.unwrap().unwrap();
        assert_eq!(got.status, RunStatus::Running);
        assert_eq!(got.started_at.as_deref(), Some("t1"));

        store
            .update_run(
                "run-1",
                &RunUpdate {
                    status: Some(RunStatus::Completed),
                    result: Some("done".into()),
                    steps_count: Some(3),
                    tokens_used: Some(512),
                    completed_at: Some("t2".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let got = store.get_run("run-1").await.unwrap().unwrap();
        assert_eq!(got.status, RunStatus::Completed);
        assert_eq!(got.result, "done");
        assert_eq!(got.steps_count, 3);
        assert_eq!(got.tokens_used, 512);
    }

    #[tokio::test]
    async fn steps_and_memory() {
        let store = temp_store();
        store.save_agent(&sample_agent()).await.unwrap();

        let run = AgentRun {
            id: "run-2".into(),
            agent_id: "agent-1".into(),
            trigger_type: TriggerType::Manual,
            trigger_payload: serde_json::json!({}),
            goal: "g".into(),
            status: RunStatus::Running,
            result: String::new(),
            error: String::new(),
            steps_count: 0,
            tokens_used: 0,
            started_at: Some("t0".into()),
            completed_at: None,
            created_at: now_iso(),
        };
        store.create_run(&run).await.unwrap();

        for i in 1..=3 {
            store
                .add_step(&AgentStep {
                    id: format!("step-{}", i),
                    run_id: "run-2".into(),
                    step_number: i,
                    step_type: StepType::Thought,
                    content: format!("thought {}", i),
                    tool_name: String::new(),
                    tool_input: serde_json::json!({}),
                    tool_output: String::new(),
                    tokens: 10,
                    created_at: now_iso(),
                })
                .await
                .unwrap();
        }
        let steps = store.list_steps("run-2").await.unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].step_number, 1);
        assert_eq!(steps[2].content, "thought 3");

        store
            .add_memory(&AgentMemoryRecord {
                id: "m1".into(),
                agent_id: "agent-1".into(),
                role: MemoryRole::User,
                content: "hola".into(),
                run_id: "run-2".into(),
                created_at: now_iso(),
            })
            .await
            .unwrap();
        store
            .add_memory(&AgentMemoryRecord {
                id: "m2".into(),
                agent_id: "agent-1".into(),
                role: MemoryRole::Assistant,
                content: "chau".into(),
                run_id: "run-2".into(),
                created_at: now_iso(),
            })
            .await
            .unwrap();
        let mem = store.recent_memory("agent-1", 5).await.unwrap();
        assert_eq!(mem.len(), 2);
    }

    #[tokio::test]
    async fn delete_agent_cascades() {
        let store = temp_store();
        store.save_agent(&sample_agent()).await.unwrap();
        store
            .create_run(&AgentRun {
                id: "r".into(),
                agent_id: "agent-1".into(),
                trigger_type: TriggerType::Manual,
                trigger_payload: serde_json::json!({}),
                goal: "g".into(),
                status: RunStatus::Completed,
                result: String::new(),
                error: String::new(),
                steps_count: 0,
                tokens_used: 0,
                started_at: None,
                completed_at: None,
                created_at: now_iso(),
            })
            .await
            .unwrap();

        store.delete_agent("agent-1").await.unwrap();
        assert!(store.get_agent("agent-1").await.unwrap().is_none());
        assert!(store.get_run("r").await.unwrap().is_none());
    }
}
