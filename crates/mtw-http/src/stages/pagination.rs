use async_trait::async_trait;
use mtw_core::MtwError;
use std::collections::HashMap;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::{MtwResponse, PaginationInfo, ResponseBody};

/// Configuration for pagination extraction.
#[derive(Debug, Clone)]
pub struct PaginationConfig {
    /// JSON field that contains the next page cursor or URL.
    pub next_field: Option<String>,
    /// JSON field for total count.
    pub total_field: Option<String>,
    /// JSON field for has_more boolean.
    pub has_more_field: Option<String>,
    /// JSON field for current page number.
    pub page_field: Option<String>,
    /// JSON field for items per page.
    pub per_page_field: Option<String>,
}

impl Default for PaginationConfig {
    fn default() -> Self {
        Self {
            next_field: None,
            total_field: None,
            has_more_field: None,
            page_field: None,
            per_page_field: None,
        }
    }
}

/// Pipeline stage that extracts pagination info from Link headers and JSON body fields.
pub struct PaginationStage {
    config: PaginationConfig,
}

impl PaginationStage {
    pub fn new() -> Self {
        Self {
            config: PaginationConfig::default(),
        }
    }

    pub fn with_config(config: PaginationConfig) -> Self {
        Self { config }
    }

    /// Parse RFC 5988 Link headers (GitHub style: `<url>; rel="next"`).
    fn parse_link_header(header: &str) -> HashMap<String, String> {
        let mut links = HashMap::new();
        for part in header.split(',') {
            let part = part.trim();
            let mut url = None;
            let mut rel = None;

            for segment in part.split(';') {
                let segment = segment.trim();
                if segment.starts_with('<') && segment.ends_with('>') {
                    url = Some(segment[1..segment.len() - 1].to_string());
                } else if let Some(r) = segment.strip_prefix("rel=") {
                    rel = Some(r.trim_matches('"').to_string());
                }
            }

            if let (Some(url), Some(rel)) = (url, rel) {
                links.insert(rel, url);
            }
        }
        links
    }

    /// Extract pagination info from JSON body.
    fn extract_from_json(
        &self,
        json: &serde_json::Value,
    ) -> PaginationInfo {
        let mut info = PaginationInfo::default();

        // Try configured fields first, then common defaults
        let next_fields = match &self.config.next_field {
            Some(f) => vec![f.as_str()],
            None => vec!["next", "next_cursor", "next_page", "next_url"],
        };
        for field in next_fields {
            if let Some(val) = json.get(field) {
                if let Some(s) = val.as_str() {
                    if !s.is_empty() {
                        info.next = Some(s.to_string());
                        break;
                    }
                }
            }
        }

        let total_fields = match &self.config.total_field {
            Some(f) => vec![f.as_str()],
            None => vec!["total", "total_count", "count"],
        };
        for field in total_fields {
            if let Some(val) = json.get(field) {
                if let Some(n) = val.as_u64() {
                    info.total = Some(n);
                    break;
                }
            }
        }

        let has_more_fields = match &self.config.has_more_field {
            Some(f) => vec![f.as_str()],
            None => vec!["has_more", "hasMore", "has_next"],
        };
        for field in has_more_fields {
            if let Some(val) = json.get(field) {
                if let Some(b) = val.as_bool() {
                    info.has_more = b;
                    break;
                }
            }
        }

        let page_fields = match &self.config.page_field {
            Some(f) => vec![f.as_str()],
            None => vec!["page", "current_page"],
        };
        for field in page_fields {
            if let Some(val) = json.get(field) {
                if let Some(n) = val.as_u64() {
                    info.page = Some(n as u32);
                    break;
                }
            }
        }

        let per_page_fields = match &self.config.per_page_field {
            Some(f) => vec![f.as_str()],
            None => vec!["per_page", "page_size", "limit"],
        };
        for field in per_page_fields {
            if let Some(val) = json.get(field) {
                if let Some(n) = val.as_u64() {
                    info.per_page = Some(n as u32);
                    break;
                }
            }
        }

        info
    }
}

impl Default for PaginationStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for PaginationStage {
    fn name(&self) -> &str {
        "pagination"
    }

    fn priority(&self) -> i32 {
        40
    }

    async fn process(
        &self,
        mut response: MtwResponse,
        _context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let mut info = PaginationInfo::default();

        // Parse Link header if present
        if let Some(link_header) = response.headers.get("link") {
            let links = Self::parse_link_header(link_header);
            info.next = links.get("next").cloned();
            info.prev = links.get("prev").or(links.get("previous")).cloned();
        }

        // Extract from JSON body
        let json_info = match &response.body {
            ResponseBody::Json(v) => Some(self.extract_from_json(v)),
            ResponseBody::Bytes(b) => {
                serde_json::from_slice::<serde_json::Value>(b)
                    .ok()
                    .map(|v| self.extract_from_json(&v))
            }
            ResponseBody::Empty => None,
        };

        if let Some(ji) = json_info {
            // JSON fields fill in what Link header didn't provide
            if info.next.is_none() {
                info.next = ji.next;
            }
            if info.prev.is_none() {
                info.prev = ji.prev;
            }
            info.total = ji.total.or(info.total);
            info.page = ji.page.or(info.page);
            info.per_page = ji.per_page.or(info.per_page);
            info.has_more = ji.has_more || info.next.is_some();
        } else {
            info.has_more = info.next.is_some();
        }

        // Only set pagination if we found anything
        if info.next.is_some()
            || info.prev.is_some()
            || info.total.is_some()
            || info.page.is_some()
            || info.has_more
        {
            response.pagination = Some(info);
        }

        Ok(PipelineAction::Continue(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::MtwRequest;

    fn make_ctx() -> PipelineContext {
        PipelineContext::new(MtwRequest::get("http://example.com"))
    }

    #[test]
    fn test_parse_link_header() {
        let header = r#"<https://api.example.com/items?page=2>; rel="next", <https://api.example.com/items?page=5>; rel="last""#;
        let links = PaginationStage::parse_link_header(header);
        assert_eq!(
            links.get("next").unwrap(),
            "https://api.example.com/items?page=2"
        );
        assert_eq!(
            links.get("last").unwrap(),
            "https://api.example.com/items?page=5"
        );
    }

    #[tokio::test]
    async fn test_link_header_pagination() {
        let stage = PaginationStage::new();
        let resp = MtwResponse::new(200)
            .with_header(
                "link",
                r#"<https://api.example.com/items?page=3>; rel="next", <https://api.example.com/items?page=1>; rel="prev""#,
            )
            .with_body(b"[]".to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let pg = resp.pagination.unwrap();
            assert_eq!(
                pg.next.as_deref(),
                Some("https://api.example.com/items?page=3")
            );
            assert_eq!(
                pg.prev.as_deref(),
                Some("https://api.example.com/items?page=1")
            );
            assert!(pg.has_more);
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_json_body_pagination() {
        let stage = PaginationStage::new();
        let json_body = serde_json::json!({
            "items": [1, 2, 3],
            "total": 100,
            "page": 2,
            "per_page": 10,
            "has_more": true,
            "next": "https://api.example.com/items?page=3"
        });
        let resp = MtwResponse::new(200).with_json(json_body);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let pg = resp.pagination.unwrap();
            assert_eq!(pg.total, Some(100));
            assert_eq!(pg.page, Some(2));
            assert_eq!(pg.per_page, Some(10));
            assert!(pg.has_more);
            assert!(pg.next.is_some());
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_no_pagination_info() {
        let stage = PaginationStage::new();
        let resp = MtwResponse::new(200).with_json(serde_json::json!({"data": "hello"}));
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert!(resp.pagination.is_none());
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_custom_field_config() {
        let config = PaginationConfig {
            next_field: Some("cursor_next".into()),
            total_field: Some("result_count".into()),
            ..Default::default()
        };
        let stage = PaginationStage::with_config(config);
        let json_body = serde_json::json!({
            "cursor_next": "abc123",
            "result_count": 42
        });
        let resp = MtwResponse::new(200).with_json(json_body);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let pg = resp.pagination.unwrap();
            assert_eq!(pg.next.as_deref(), Some("abc123"));
            assert_eq!(pg.total, Some(42));
        } else {
            panic!("expected Continue");
        }
    }
}
