use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemType { Extension, Agent, Flow, Theme, Template, Channel }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus { Available, Installed, Active, Disabled }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType { Bundled, Local, Import, Community }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceItem {
    pub id: String, pub item_type: ItemType, pub slug: String,
    pub name: String, pub description: String, pub long_description: String,
    pub version: String, pub author: String, pub author_url: String,
    pub icon: String, pub category: String, pub tags: Vec<String>,
    pub license: String, pub price_cents: i64, pub currency: String,
    pub source_type: SourceType, pub source_ref: String,
    pub package_data: serde_json::Value,
    pub min_kernel_version: String, pub dependencies: Vec<String>,
    pub install_count: u64, pub avg_rating: f64, pub review_count: u32,
    pub status: ItemStatus, pub installed_at: Option<String>,
    pub installed_version: String, pub featured: bool, pub verified: bool,
    pub created_at: String, pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceReview {
    pub id: String, pub item_id: String, pub rating: u8,
    pub title: String, pub body: String, pub author: String,
    pub created_at: String, pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_item_type_serialization() {
        assert_eq!(serde_json::to_string(&ItemType::Agent).unwrap(), "\"agent\"");
        assert_eq!(serde_json::to_string(&SourceType::Bundled).unwrap(), "\"bundled\"");
    }
}
