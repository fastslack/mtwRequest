use async_trait::async_trait;
use mtw_core::MtwError;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::MtwResponse;

/// Configuration for header extraction.
#[derive(Debug, Clone)]
pub struct HeaderExtractionConfig {
    /// Headers to extract into response metadata.
    pub headers: Vec<String>,
    /// Auto-extract common headers (x-request-id, x-correlation-id, content-type).
    pub auto_extract: bool,
}

impl Default for HeaderExtractionConfig {
    fn default() -> Self {
        Self {
            headers: Vec::new(),
            auto_extract: true,
        }
    }
}

/// Auto-extracted common headers.
const AUTO_HEADERS: &[&str] = &[
    "x-request-id",
    "x-correlation-id",
    "content-type",
    "content-length",
    "server",
];

/// Pipeline stage that extracts specific headers into response metadata.
pub struct HeaderExtractionStage {
    config: HeaderExtractionConfig,
}

impl HeaderExtractionStage {
    pub fn new() -> Self {
        Self {
            config: HeaderExtractionConfig::default(),
        }
    }

    pub fn with_config(config: HeaderExtractionConfig) -> Self {
        Self { config }
    }

    pub fn with_headers(headers: Vec<&str>) -> Self {
        Self {
            config: HeaderExtractionConfig {
                headers: headers.into_iter().map(String::from).collect(),
                auto_extract: true,
            },
        }
    }
}

impl Default for HeaderExtractionStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for HeaderExtractionStage {
    fn name(&self) -> &str {
        "header_extraction"
    }

    fn priority(&self) -> i32 {
        25
    }

    async fn process(
        &self,
        mut response: MtwResponse,
        _context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let mut extracted = serde_json::Map::new();

        // Auto-extract common headers
        if self.config.auto_extract {
            for &header_name in AUTO_HEADERS {
                if let Some(value) = response.headers.get(header_name) {
                    extracted.insert(
                        header_name.to_string(),
                        serde_json::Value::String(value.clone()),
                    );
                }
            }
        }

        // Extract configured headers
        for header_name in &self.config.headers {
            let lower = header_name.to_lowercase();
            if let Some(value) = response.headers.get(&lower) {
                extracted.insert(lower, serde_json::Value::String(value.clone()));
            }
        }

        if !extracted.is_empty() {
            response.metadata.insert(
                "extracted_headers".into(),
                serde_json::Value::Object(extracted),
            );
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

    #[tokio::test]
    async fn test_auto_extract_headers() {
        let stage = HeaderExtractionStage::new();
        let resp = MtwResponse::new(200)
            .with_header("x-request-id", "req-123")
            .with_header("content-type", "application/json")
            .with_header("x-custom", "ignored");
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let headers = resp.metadata.get("extracted_headers").unwrap();
            assert_eq!(headers["x-request-id"], "req-123");
            assert_eq!(headers["content-type"], "application/json");
            assert!(headers.get("x-custom").is_none());
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_custom_header_extraction() {
        let stage = HeaderExtractionStage::with_headers(vec!["x-custom-header"]);
        let resp = MtwResponse::new(200).with_header("x-custom-header", "custom-value");
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let headers = resp.metadata.get("extracted_headers").unwrap();
            assert_eq!(headers["x-custom-header"], "custom-value");
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_no_headers_no_metadata() {
        let stage = HeaderExtractionStage::with_config(HeaderExtractionConfig {
            headers: Vec::new(),
            auto_extract: false,
        });
        let resp = MtwResponse::new(200);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert!(resp.metadata.get("extracted_headers").is_none());
        } else {
            panic!("expected Continue");
        }
    }
}
