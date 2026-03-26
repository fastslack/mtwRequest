//! RSS and Atom feed support.
//!
//! Provides config and data structures for consuming RSS/Atom feeds.
//! Actual feed fetching and parsing will be implemented in a future phase.

use serde::{Deserialize, Serialize};

/// RSS feed configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssConfig {
    /// Feed URL to fetch.
    pub feed_url: String,
    /// Polling interval in seconds.
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    /// Maximum number of items to keep.
    #[serde(default = "default_max_items")]
    pub max_items: usize,
    /// Optional HTTP headers for authenticated feeds.
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_poll_interval() -> u64 {
    300 // 5 minutes
}

fn default_max_items() -> usize {
    50
}

fn default_timeout() -> u64 {
    30
}

/// Feed format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedFormat {
    Rss,
    Atom,
    Unknown,
}

/// A parsed RSS/Atom feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssFeed {
    /// Feed title.
    pub title: String,
    /// Feed link/URL.
    pub link: String,
    /// Feed description.
    pub description: String,
    /// Detected feed format.
    pub format: FeedFormat,
    /// Language code (e.g. "en-us").
    pub language: Option<String>,
    /// Last build date (ISO 8601 string).
    pub last_build_date: Option<String>,
    /// Feed items.
    pub items: Vec<RssItem>,
}

/// A single RSS/Atom feed item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssItem {
    /// Item title.
    pub title: Option<String>,
    /// Item link/URL.
    pub link: Option<String>,
    /// Item description or summary.
    pub description: Option<String>,
    /// Full content (if available).
    pub content: Option<String>,
    /// Publication date (ISO 8601 string).
    pub pub_date: Option<String>,
    /// Globally unique identifier.
    pub guid: Option<String>,
    /// Author name.
    pub author: Option<String>,
    /// Categories/tags.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Enclosure/media attachment URL.
    pub enclosure_url: Option<String>,
    /// Enclosure MIME type.
    pub enclosure_type: Option<String>,
}

/// RSS feed reader client (stub).
pub struct RssReader {
    config: RssConfig,
}

impl RssReader {
    pub fn new(config: RssConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &RssConfig {
        &self.config
    }

    /// Fetch and parse the feed.
    ///
    /// Stub: returns an error indicating this is not yet implemented.
    pub async fn fetch(&self) -> Result<RssFeed, String> {
        // TODO: implement actual HTTP fetch + XML parsing
        Err(format!(
            "RSS fetch not yet implemented for {}",
            self.config.feed_url
        ))
    }

    /// Parse raw XML content into an RssFeed.
    ///
    /// Stub: returns an error indicating this is not yet implemented.
    pub fn parse(_xml: &str) -> Result<RssFeed, String> {
        // TODO: implement XML parsing for RSS 2.0 and Atom feeds
        Err("RSS parse not yet implemented".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let json = r#"{"feed_url": "https://example.com/feed.xml"}"#;
        let config: RssConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.feed_url, "https://example.com/feed.xml");
        assert_eq!(config.poll_interval_secs, 300);
        assert_eq!(config.max_items, 50);
        assert_eq!(config.timeout_secs, 30);
    }

    #[test]
    fn test_rss_item_serialization() {
        let item = RssItem {
            title: Some("Test Post".to_string()),
            link: Some("https://example.com/post/1".to_string()),
            description: Some("A test post".to_string()),
            content: None,
            pub_date: Some("2026-01-15T10:00:00Z".to_string()),
            guid: Some("post-1".to_string()),
            author: Some("author@example.com".to_string()),
            categories: vec!["tech".to_string(), "rust".to_string()],
            enclosure_url: None,
            enclosure_type: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        let parsed: RssItem = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, Some("Test Post".to_string()));
        assert_eq!(parsed.categories.len(), 2);
    }

    #[test]
    fn test_rss_feed_serialization() {
        let feed = RssFeed {
            title: "My Blog".to_string(),
            link: "https://example.com".to_string(),
            description: "A blog about things".to_string(),
            format: FeedFormat::Rss,
            language: Some("en-us".to_string()),
            last_build_date: None,
            items: vec![],
        };
        let json = serde_json::to_string(&feed).unwrap();
        let parsed: RssFeed = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "My Blog");
        assert_eq!(parsed.format, FeedFormat::Rss);
    }

    #[test]
    fn test_reader_creation() {
        let reader = RssReader::new(RssConfig {
            feed_url: "https://example.com/rss".to_string(),
            poll_interval_secs: 60,
            max_items: 10,
            headers: Default::default(),
            timeout_secs: 15,
        });
        assert_eq!(reader.config().feed_url, "https://example.com/rss");
        assert_eq!(reader.config().max_items, 10);
    }
}
