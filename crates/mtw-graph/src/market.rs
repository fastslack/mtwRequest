use async_trait::async_trait;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetNode { pub symbol: String, pub name: String, pub asset_class: String, pub sector: Option<String>, pub market_cap: Option<f64> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarAsset { pub symbol: String, pub similarity: f64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketLeader { pub symbol: String, pub centrality: f64, pub correlation_count: u32 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationCluster { pub id: u32, pub assets: Vec<String>, pub avg_correlation: f64 }

#[async_trait]
pub trait MarketGraphService: Send + Sync {
    async fn sync_assets(&self, assets: &[AssetNode]) -> Result<u32, MtwError>;
    async fn compute_correlations(&self, window: usize) -> Result<u32, MtwError>;
    async fn find_similar_assets(&self, symbol: &str, k: usize) -> Result<Vec<SimilarAsset>, MtwError>;
    async fn get_market_leaders(&self, limit: usize) -> Result<Vec<MarketLeader>, MtwError>;
    async fn get_correlation_clusters(&self) -> Result<Vec<CorrelationCluster>, MtwError>;
}
