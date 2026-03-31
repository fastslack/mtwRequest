use async_trait::async_trait;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use crate::types::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentResult { pub nodes_enriched: u32, pub relationships_created: u32, pub duration_ms: u64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GdsResult { pub communities_detected: u32, pub page_rank_computed: bool, pub betweenness_computed: bool, pub duration_ms: u64 }

#[async_trait]
pub trait MtwGraphAnalytics: Send + Sync {
    async fn enrich_graph(&self) -> Result<EnrichmentResult, MtwError>;
    async fn run_gds_analytics(&self) -> Result<GdsResult, MtwError>;
    async fn get_network_report(&self) -> Result<NetworkReport, MtwError>;
    async fn find_shortest_path(&self, from: &str, to: &str) -> Result<Option<GraphPath>, MtwError>;
    async fn get_communities(&self) -> Result<Vec<CommunityInfo>, MtwError>;
    async fn get_influencers(&self, limit: usize) -> Result<Vec<InfluencerInfo>, MtwError>;
    async fn get_bridges(&self, limit: usize) -> Result<Vec<BridgeContact>, MtwError>;
}
