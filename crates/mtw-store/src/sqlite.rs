//! SQLite store backend — read-only connection pool with WAL mode.
//!
//! 100% schema-agnostic. No assumptions about table names, columns,
//! or conventions. Works with any SQLite database.
//!
//! Uses r2d2 connection pooling for concurrent reads.
//! SQLite WAL mode allows unlimited concurrent readers + 1 writer.
//! By default opens in read-only mode (PRAGMA query_only = ON).

use async_trait::async_trait;
use mtw_core::MtwError;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OpenFlags;

use crate::{MtwStore, StoreConfig, StoreHealth, StoreResult};

/// SQLite store with read-only connection pool.
/// Completely agnostic — no knowledge of any project's schema.
pub struct SqliteStore {
    pool: Pool<SqliteConnectionManager>,
    config: StoreConfig,
}

impl SqliteStore {
    /// Open a SQLite store from config
    pub fn open(config: &StoreConfig) -> Result<Self, MtwError> {
        let path = std::path::Path::new(&config.path);
        if !path.exists() {
            return Err(MtwError::Config(format!(
                "store path not found: {} (create the database first or check the path)",
                config.path
            )));
        }

        let flags = if config.readonly {
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI
        };

        let manager = SqliteConnectionManager::file(&config.path).with_flags(flags);

        let pool = Pool::builder()
            .max_size(config.pool_size)
            .build(manager)
            .map_err(|e| MtwError::Config(format!("failed to create connection pool: {}", e)))?;

        // Initialize with performance pragmas
        let conn = pool
            .get()
            .map_err(|e| MtwError::Config(format!("failed to get connection: {}", e)))?;

        let pragmas = format!(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -{cache_kb};
             PRAGMA mmap_size = {mmap_bytes};
             PRAGMA busy_timeout = {timeout};
             PRAGMA temp_store = MEMORY;
             {query_only}",
            cache_kb = config.cache_mb * 1024,
            mmap_bytes = (config.mmap_mb as u64) * 1024 * 1024,
            timeout = config.busy_timeout_ms,
            query_only = if config.readonly {
                "PRAGMA query_only = ON;"
            } else {
                ""
            }
        );

        conn.execute_batch(&pragmas)
            .map_err(|e| MtwError::Config(format!("failed to set pragmas: {}", e)))?;

        tracing::info!(
            path = %config.path,
            pool_size = config.pool_size,
            readonly = config.readonly,
            cache_mb = config.cache_mb,
            "store opened"
        );

        Ok(Self {
            pool,
            config: config.clone(),
        })
    }

    /// Get a connection from the pool (for custom queries in modules)
    pub fn connection(
        &self,
    ) -> Result<r2d2::PooledConnection<SqliteConnectionManager>, MtwError> {
        self.pool
            .get()
            .map_err(|e| MtwError::Internal(format!("pool exhausted: {}", e)))
    }

    /// Execute a SQL query synchronously and return rows as JSON array.
    /// The caller provides the full SQL — no schema assumptions.
    pub fn exec_query_sync(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> Result<serde_json::Value, MtwError> {
        let conn = self.connection()?;
        execute_sql(&conn, sql, params)
    }
}

#[async_trait]
impl MtwStore for SqliteStore {
    /// Query a table by name. Returns ALL rows (no filtering).
    /// Callers who need filtering should use `query_raw()` instead.
    async fn query(&self, table: &str, _params: serde_json::Value) -> StoreResult {
        let sql = format!(
            "SELECT * FROM {} LIMIT 1000",
            sanitize_identifier(table)
        );
        let pool = self.pool.clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            execute_sql(&conn, &sql, &[])
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    /// Execute a raw SQL query. The caller provides everything.
    /// The store makes zero assumptions about the schema.
    async fn query_raw(&self, sql: &str, params: &[serde_json::Value]) -> StoreResult {
        let sql = sql.to_string();
        let params = params.to_vec();
        let pool = self.pool.clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;
            execute_sql(&conn, &sql, &params)
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }

    async fn health(&self) -> Result<StoreHealth, MtwError> {
        let state = self.pool.state();
        Ok(StoreHealth {
            available: true,
            driver: "sqlite".into(),
            path: self.config.path.clone(),
            pool_size: state.connections,
            pool_idle: state.idle_connections,
        })
    }

    async fn info(&self) -> Result<serde_json::Value, MtwError> {
        let pool = self.pool.clone();
        let path = self.config.path.clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| MtwError::Internal(format!("pool: {}", e)))?;

            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type='table' \
                     AND name NOT LIKE '\\_%' ESCAPE '\\' \
                     AND name NOT LIKE 'sqlite_%' \
                     ORDER BY name",
                )
                .map_err(|e| MtwError::Internal(format!("prepare: {}", e)))?;

            let tables: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| MtwError::Internal(format!("query: {}", e)))?
                .filter_map(|r| r.ok())
                .collect();

            let size_bytes = std::fs::metadata(&path)
                .map(|m| m.len())
                .unwrap_or(0);

            Ok(serde_json::json!({
                "driver": "sqlite",
                "path": path,
                "tables": tables,
                "table_count": tables.len(),
                "size_bytes": size_bytes,
                "size_mb": format!("{:.1}", size_bytes as f64 / 1_048_576.0),
            }))
        })
        .await
        .map_err(|e| MtwError::Internal(format!("spawn: {}", e)))?
    }
}

/// Execute SQL on a connection and return rows as JSON array.
/// Shared between sync and async paths. No schema assumptions.
fn execute_sql(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<serde_json::Value, MtwError> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MtwError::Internal(format!("prepare: {}", e)))?;

    let rusqlite_params: Vec<Box<dyn rusqlite::types::ToSql>> =
        params.iter().map(|v| json_to_sql(v)).collect();
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        rusqlite_params.iter().map(|b| b.as_ref()).collect();

    let column_names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            let mut obj = serde_json::Map::new();
            for (i, col) in column_names.iter().enumerate() {
                let val: rusqlite::types::Value = row.get(i)?;
                obj.insert(col.clone(), sqlite_to_json(val));
            }
            Ok(serde_json::Value::Object(obj))
        })
        .map_err(|e| MtwError::Internal(format!("query: {}", e)))?;

    let mut results = Vec::new();
    for row in rows {
        results
            .push(row.map_err(|e| MtwError::Internal(format!("row: {}", e)))?);
    }

    Ok(serde_json::Value::Array(results))
}

/// Convert a serde_json::Value to a boxed rusqlite ToSql
fn json_to_sql(val: &serde_json::Value) -> Box<dyn rusqlite::types::ToSql> {
    match val {
        serde_json::Value::Null => Box::new(rusqlite::types::Null),
        serde_json::Value::Bool(b) => Box::new(*b as i32),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        serde_json::Value::String(s) => Box::new(s.clone()),
        other => Box::new(other.to_string()),
    }
}

/// Convert a rusqlite Value to serde_json::Value
fn sqlite_to_json(val: rusqlite::types::Value) -> serde_json::Value {
    match val {
        rusqlite::types::Value::Null => serde_json::Value::Null,
        rusqlite::types::Value::Integer(i) => serde_json::json!(i),
        rusqlite::types::Value::Real(f) => serde_json::json!(f),
        rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
        rusqlite::types::Value::Blob(b) => {
            use base64::Engine;
            serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(b))
        }
    }
}

/// Sanitize a SQL identifier to prevent injection
fn sanitize_identifier(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a generic test database — no project-specific schema
    fn make_test_db() -> (tempfile::TempDir, StoreConfig) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;

            CREATE TABLE products (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                price_cents INTEGER NOT NULL DEFAULT 0,
                in_stock INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            INSERT INTO products (id, name, price_cents) VALUES ('p1', 'Widget', 999);
            INSERT INTO products (id, name, price_cents) VALUES ('p2', 'Gadget', 1999);
            INSERT INTO products (id, name, price_cents, in_stock) VALUES ('p3', 'Gizmo', 499, 0);

            CREATE TABLE orders (
                id TEXT PRIMARY KEY,
                product_id TEXT NOT NULL,
                quantity INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL DEFAULT 'pending'
            );
            INSERT INTO orders (id, product_id, quantity) VALUES ('o1', 'p1', 3);
            ",
        )
        .unwrap();

        let config = StoreConfig {
            path: db_path.to_string_lossy().to_string(),
            readonly: true,
            pool_size: 2,
            ..Default::default()
        };

        (dir, config)
    }

    #[tokio::test]
    async fn test_open_store() {
        let (_dir, config) = make_test_db();
        let store = SqliteStore::open(&config).unwrap();
        let health = store.health().await.unwrap();
        assert!(health.available);
        assert_eq!(health.driver, "sqlite");
    }

    #[tokio::test]
    async fn test_query_table_returns_all_rows() {
        let (_dir, config) = make_test_db();
        let store = SqliteStore::open(&config).unwrap();

        let result = store
            .query("products", serde_json::json!({}))
            .await
            .unwrap();
        let arr = result.as_array().unwrap();

        // Returns ALL 3 rows — no filtering, fully agnostic
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["name"], "Widget");
        assert_eq!(arr[0]["price_cents"], 999);
    }

    #[tokio::test]
    async fn test_query_raw_count() {
        let (_dir, config) = make_test_db();
        let store = SqliteStore::open(&config).unwrap();

        let result = store
            .query_raw("SELECT COUNT(*) as total FROM products", &[])
            .await
            .unwrap();

        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["total"], 3);
    }

    #[tokio::test]
    async fn test_query_raw_with_params() {
        let (_dir, config) = make_test_db();
        let store = SqliteStore::open(&config).unwrap();

        let result = store
            .query_raw(
                "SELECT * FROM products WHERE in_stock = ? ORDER BY name",
                &[serde_json::json!(1)],
            )
            .await
            .unwrap();

        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "Gadget");
        assert_eq!(arr[1]["name"], "Widget");
    }

    #[tokio::test]
    async fn test_query_raw_join() {
        let (_dir, config) = make_test_db();
        let store = SqliteStore::open(&config).unwrap();

        let result = store
            .query_raw(
                "SELECT o.id, p.name, o.quantity \
                 FROM orders o JOIN products p ON o.product_id = p.id",
                &[],
            )
            .await
            .unwrap();

        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "Widget");
        assert_eq!(arr[0]["quantity"], 3);
    }

    #[tokio::test]
    async fn test_store_info() {
        let (_dir, config) = make_test_db();
        let store = SqliteStore::open(&config).unwrap();

        let info = store.info().await.unwrap();
        assert_eq!(info["driver"], "sqlite");
        assert_eq!(info["table_count"], 2);

        let tables = info["tables"].as_array().unwrap();
        assert!(tables.contains(&serde_json::json!("products")));
        assert!(tables.contains(&serde_json::json!("orders")));
    }

    #[tokio::test]
    async fn test_nonexistent_path() {
        let config = StoreConfig {
            path: "/tmp/nonexistent_mtw_test_12345.db".into(),
            ..Default::default()
        };
        let result = SqliteStore::open(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_identifier() {
        assert_eq!(sanitize_identifier("users"), "users");
        assert_eq!(sanitize_identifier("order_items"), "order_items");
        assert_eq!(
            sanitize_identifier("users; DROP TABLE data;--"),
            "usersDROPTABLEdata"
        );
    }

    #[test]
    fn test_json_to_sql_conversions() {
        json_to_sql(&serde_json::json!(null));
        json_to_sql(&serde_json::json!(true));
        json_to_sql(&serde_json::json!(42));
        json_to_sql(&serde_json::json!(3.14));
        json_to_sql(&serde_json::json!("hello"));
        json_to_sql(&serde_json::json!({"key": "value"}));
    }
}
