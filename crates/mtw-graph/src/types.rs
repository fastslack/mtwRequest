use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String, pub labels: Vec<String>,
    pub properties: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRelation {
    pub id: String, pub rel_type: String,
    pub start_node_id: String, pub end_node_id: String,
    pub properties: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPath {
    pub nodes: Vec<GraphNode>, pub relationships: Vec<GraphRelation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityInfo {
    pub community_id: u32, pub size: usize,
    pub dominant_domain: Option<String>, pub dominant_company: Option<String>,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfluencerInfo { pub name: String, pub company: Option<String>, pub page_rank: f64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeContact { pub name: String, pub company: Option<String>, pub betweenness: f64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkReport {
    pub components: u32, pub communities: u32,
    pub total_nodes: u64, pub total_relationships: u64,
    pub top_influencers: Vec<InfluencerInfo>,
    pub bridge_contacts: Vec<BridgeContact>,
    pub top_communities: Vec<CommunityInfo>,
}
