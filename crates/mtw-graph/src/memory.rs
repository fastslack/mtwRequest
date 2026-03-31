use async_trait::async_trait;
use mtw_core::MtwError;
use std::collections::HashMap;
use std::sync::RwLock;
use crate::client::{GraphClient, GraphConfig, GraphRecord};
use crate::types::{GraphNode, GraphRelation};

/// In-memory graph for testing and small datasets
pub struct InMemoryGraph {
    nodes: RwLock<HashMap<String, GraphNode>>,
    relations: RwLock<Vec<GraphRelation>>,
    available: std::sync::atomic::AtomicBool,
}

impl InMemoryGraph {
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            relations: RwLock::new(Vec::new()),
            available: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn add_node(&self, node: GraphNode) { self.nodes.write().unwrap().insert(node.id.clone(), node); }
    pub fn add_relation(&self, rel: GraphRelation) { self.relations.write().unwrap().push(rel); }
    pub fn node_count(&self) -> usize { self.nodes.read().unwrap().len() }
    pub fn relation_count(&self) -> usize { self.relations.read().unwrap().len() }

    pub fn get_node(&self, id: &str) -> Option<GraphNode> { self.nodes.read().unwrap().get(id).cloned() }

    pub fn get_neighbors(&self, node_id: &str) -> Vec<String> {
        self.relations.read().unwrap().iter()
            .filter_map(|r| {
                if r.start_node_id == node_id { Some(r.end_node_id.clone()) }
                else if r.end_node_id == node_id { Some(r.start_node_id.clone()) }
                else { None }
            }).collect()
    }
}

impl Default for InMemoryGraph { fn default() -> Self { Self::new() } }

#[async_trait]
impl GraphClient for InMemoryGraph {
    async fn connect(&self, _config: &GraphConfig) -> Result<(), MtwError> {
        self.available.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
    async fn run(&self, _cypher: &str, _params: Option<HashMap<String, serde_json::Value>>) -> Result<Vec<GraphRecord>, MtwError> {
        Ok(Vec::new())
    }
    async fn run_single(&self, cypher: &str, params: Option<HashMap<String, serde_json::Value>>) -> Result<Option<GraphRecord>, MtwError> {
        Ok(self.run(cypher, params).await?.into_iter().next())
    }
    fn is_available(&self) -> bool { self.available.load(std::sync::atomic::Ordering::Relaxed) }
    async fn close(&self) -> Result<(), MtwError> {
        self.available.store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_graph() {
        let g = InMemoryGraph::new();
        g.add_node(GraphNode { id: "n1".into(), labels: vec!["Person".into()], properties: HashMap::new() });
        g.add_node(GraphNode { id: "n2".into(), labels: vec!["Person".into()], properties: HashMap::new() });
        g.add_relation(GraphRelation { id: "r1".into(), rel_type: "KNOWS".into(), start_node_id: "n1".into(), end_node_id: "n2".into(), properties: HashMap::new() });

        assert_eq!(g.node_count(), 2);
        assert_eq!(g.relation_count(), 1);
        assert_eq!(g.get_neighbors("n1"), vec!["n2"]);
    }

    #[tokio::test]
    async fn test_connect_disconnect() {
        let g = InMemoryGraph::new();
        assert!(!g.is_available());
        g.connect(&GraphConfig::default()).await.unwrap();
        assert!(g.is_available());
        g.close().await.unwrap();
        assert!(!g.is_available());
    }
}
