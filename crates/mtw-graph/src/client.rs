use async_trait::async_trait;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphConfig {
    pub uri: String, pub user: String, pub password: String,
    #[serde(default = "default_max_conn")] pub max_connections: u32,
    #[serde(default = "default_timeout")] pub connection_timeout_ms: u64,
}
fn default_max_conn() -> u32 { 10 }
fn default_timeout() -> u64 { 5000 }

impl Default for GraphConfig {
    fn default() -> Self {
        Self { uri: "bolt://localhost:7687".into(), user: "neo4j".into(), password: "".into(),
            max_connections: 10, connection_timeout_ms: 5000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRecord { pub fields: HashMap<String, serde_json::Value> }

impl GraphRecord {
    pub fn get_string(&self, key: &str) -> Option<&str> { self.fields.get(key)?.as_str() }
    pub fn get_i64(&self, key: &str) -> Option<i64> { self.fields.get(key)?.as_i64() }
    pub fn get_f64(&self, key: &str) -> Option<f64> { self.fields.get(key)?.as_f64() }
    pub fn get_bool(&self, key: &str) -> Option<bool> { self.fields.get(key)?.as_bool() }
    pub fn get_json(&self, key: &str) -> Option<&serde_json::Value> { self.fields.get(key) }
}

#[async_trait]
pub trait GraphClient: Send + Sync {
    async fn connect(&self, config: &GraphConfig) -> Result<(), MtwError>;
    async fn run(&self, cypher: &str, params: Option<HashMap<String, serde_json::Value>>) -> Result<Vec<GraphRecord>, MtwError>;
    async fn run_single(&self, cypher: &str, params: Option<HashMap<String, serde_json::Value>>) -> Result<Option<GraphRecord>, MtwError>;
    fn is_available(&self) -> bool;
    async fn close(&self) -> Result<(), MtwError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_graph_record() {
        let mut fields = HashMap::new();
        fields.insert("name".into(), serde_json::json!("Alice"));
        fields.insert("age".into(), serde_json::json!(30));
        fields.insert("score".into(), serde_json::json!(9.5));
        fields.insert("active".into(), serde_json::json!(true));
        let r = GraphRecord { fields };
        assert_eq!(r.get_string("name"), Some("Alice"));
        assert_eq!(r.get_i64("age"), Some(30));
        assert_eq!(r.get_f64("score"), Some(9.5));
        assert_eq!(r.get_bool("active"), Some(true));
    }
}
