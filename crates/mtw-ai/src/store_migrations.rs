//! Schema migrations for the agent SQLite store.
//!
//! Migrations are applied in-order and tracked in `schema_migrations`.
//! Never edit a migration once it has shipped — add a new one instead.

/// A single schema change, identified by a monotonically-increasing version.
pub struct Migration {
    pub version: u32,
    pub sql: &'static str,
}

/// The full migration set. Append new migrations to the end.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: r#"
            CREATE TABLE IF NOT EXISTS agents (
                id              TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                provider        TEXT NOT NULL DEFAULT '',
                model           TEXT NOT NULL DEFAULT '',
                system_prompt   TEXT NOT NULL DEFAULT '',
                tool_names      TEXT NOT NULL DEFAULT '[]',
                token_budget    INTEGER NOT NULL DEFAULT 0,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS agent_runs (
                id              TEXT PRIMARY KEY,
                agent_id        TEXT NOT NULL,
                trigger_type    TEXT NOT NULL DEFAULT 'manual',
                trigger_payload TEXT NOT NULL DEFAULT '{}',
                goal            TEXT NOT NULL DEFAULT '',
                status          TEXT NOT NULL DEFAULT 'pending',
                result          TEXT NOT NULL DEFAULT '',
                error           TEXT NOT NULL DEFAULT '',
                steps_count     INTEGER NOT NULL DEFAULT 0,
                tokens_used     INTEGER NOT NULL DEFAULT 0,
                started_at      TEXT,
                completed_at    TEXT,
                created_at      TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_agent_runs_agent
                ON agent_runs(agent_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_agent_runs_status
                ON agent_runs(status);

            CREATE TABLE IF NOT EXISTS agent_steps (
                id          TEXT PRIMARY KEY,
                run_id      TEXT NOT NULL,
                step_number INTEGER NOT NULL,
                step_type   TEXT NOT NULL,
                content     TEXT NOT NULL DEFAULT '',
                tool_name   TEXT NOT NULL DEFAULT '',
                tool_input  TEXT NOT NULL DEFAULT '{}',
                tool_output TEXT NOT NULL DEFAULT '',
                tokens      INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_agent_steps_run
                ON agent_steps(run_id, step_number);

            CREATE TABLE IF NOT EXISTS agent_memory (
                id          TEXT PRIMARY KEY,
                agent_id    TEXT NOT NULL,
                role        TEXT NOT NULL DEFAULT 'user',
                content     TEXT NOT NULL DEFAULT '',
                run_id      TEXT NOT NULL DEFAULT '',
                created_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_agent_memory_agent
                ON agent_memory(agent_id, created_at DESC);
        "#,
    },
];

/// Apply all pending migrations to a connection.
///
/// Creates `schema_migrations` if missing, then runs every entry in
/// [`MIGRATIONS`] whose version is not yet recorded. Idempotent.
pub fn apply(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version    INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
         );",
    )?;

    let mut applied: std::collections::HashSet<u32> = std::collections::HashSet::new();
    {
        let mut stmt = conn.prepare("SELECT version FROM schema_migrations")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        for row in rows {
            applied.insert(row? as u32);
        }
    }

    for m in MIGRATIONS {
        if applied.contains(&m.version) {
            continue;
        }
        conn.execute_batch(m.sql)?;
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![m.version as i64, now_iso()],
        )?;
        tracing::info!(version = m.version, "agent store migration applied");
    }

    Ok(())
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}
