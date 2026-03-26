use async_trait::async_trait;
use mtw_core::MtwError;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::{MtwResponse, ResponseBody};

/// Stream format types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamFormat {
    /// Server-Sent Events (text/event-stream).
    SSE,
    /// Newline-delimited JSON.
    NDJSON,
    /// Generic chunked response.
    Chunked,
}

/// Configuration for stream processing.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// Which stream format to expect.
    pub format: StreamFormat,
    /// Auto-detect the format from Content-Type header.
    pub auto_detect: bool,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            format: StreamFormat::SSE,
            auto_detect: true,
        }
    }
}

/// Pipeline stage that handles streaming response formats (SSE, NDJSON).
/// It parses the response body into structured events stored in metadata.
pub struct StreamProcessingStage {
    config: StreamConfig,
}

impl StreamProcessingStage {
    pub fn new() -> Self {
        Self {
            config: StreamConfig::default(),
        }
    }

    pub fn with_config(config: StreamConfig) -> Self {
        Self { config }
    }

    fn detect_format(
        headers: &std::collections::HashMap<String, String>,
    ) -> Option<StreamFormat> {
        let ct = headers.get("content-type")?;
        if ct.contains("text/event-stream") {
            Some(StreamFormat::SSE)
        } else if ct.contains("application/x-ndjson") || ct.contains("application/jsonl") {
            Some(StreamFormat::NDJSON)
        } else {
            None
        }
    }

    /// Parse SSE events from text.
    fn parse_sse(text: &str) -> Vec<serde_json::Value> {
        let mut events = Vec::new();
        let mut current_event = String::new();
        let mut current_type = String::from("message");

        for line in text.lines() {
            if line.is_empty() {
                if !current_event.is_empty() {
                    events.push(serde_json::json!({
                        "type": current_type,
                        "data": current_event.trim_end(),
                    }));
                    current_event.clear();
                    current_type = "message".to_string();
                }
            } else if let Some(data) = line.strip_prefix("data: ") {
                if !current_event.is_empty() {
                    current_event.push('\n');
                }
                current_event.push_str(data);
            } else if let Some(event_type) = line.strip_prefix("event: ") {
                current_type = event_type.to_string();
            }
        }

        // Handle final event without trailing newline
        if !current_event.is_empty() {
            events.push(serde_json::json!({
                "type": current_type,
                "data": current_event.trim_end(),
            }));
        }

        events
    }

    /// Parse NDJSON from text.
    fn parse_ndjson(text: &str) -> Vec<serde_json::Value> {
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

impl Default for StreamProcessingStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for StreamProcessingStage {
    fn name(&self) -> &str {
        "stream_processing"
    }

    fn priority(&self) -> i32 {
        22
    }

    async fn process(
        &self,
        mut response: MtwResponse,
        _context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let format = if self.config.auto_detect {
            Self::detect_format(&response.headers).unwrap_or(self.config.format.clone())
        } else {
            self.config.format.clone()
        };

        // Only process text/bytes bodies
        let text = match &response.body {
            ResponseBody::Bytes(b) => {
                match std::str::from_utf8(b) {
                    Ok(s) => s.to_string(),
                    Err(_) => return Ok(PipelineAction::Continue(response)),
                }
            }
            _ => return Ok(PipelineAction::Continue(response)),
        };

        // Check content-type to see if this is actually a stream format
        let is_stream = response
            .headers
            .get("content-type")
            .map(|ct| {
                ct.contains("text/event-stream")
                    || ct.contains("application/x-ndjson")
                    || ct.contains("application/jsonl")
            })
            .unwrap_or(false);

        if !is_stream {
            return Ok(PipelineAction::Continue(response));
        }

        let events = match format {
            StreamFormat::SSE => Self::parse_sse(&text),
            StreamFormat::NDJSON => Self::parse_ndjson(&text),
            StreamFormat::Chunked => {
                // For chunked, just store the raw text
                vec![serde_json::json!({"data": text})]
            }
        };

        response.metadata.insert(
            "stream_events".into(),
            serde_json::Value::Array(events),
        );
        response.metadata.insert(
            "stream_format".into(),
            serde_json::json!(format!("{:?}", format)),
        );

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
    async fn test_parse_sse_events() {
        let stage = StreamProcessingStage::new();
        let sse_body = "data: hello\n\ndata: world\n\n";
        let resp = MtwResponse::new(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_body.as_bytes().to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let events = resp.metadata.get("stream_events").unwrap().as_array().unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0]["data"], "hello");
            assert_eq!(events[1]["data"], "world");
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_parse_sse_with_event_types() {
        let sse_body = "event: update\ndata: {\"id\": 1}\n\nevent: delete\ndata: {\"id\": 2}\n\n";
        let events = StreamProcessingStage::parse_sse(sse_body);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "update");
        assert_eq!(events[1]["type"], "delete");
    }

    #[tokio::test]
    async fn test_parse_ndjson() {
        let stage = StreamProcessingStage::with_config(StreamConfig {
            format: StreamFormat::NDJSON,
            auto_detect: true,
        });
        let ndjson_body = "{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n";
        let resp = MtwResponse::new(200)
            .with_header("content-type", "application/x-ndjson")
            .with_body(ndjson_body.as_bytes().to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            let events = resp.metadata.get("stream_events").unwrap().as_array().unwrap();
            assert_eq!(events.len(), 3);
            assert_eq!(events[0]["a"], 1);
        } else {
            panic!("expected Continue");
        }
    }

    #[tokio::test]
    async fn test_non_stream_passes_through() {
        let stage = StreamProcessingStage::new();
        let resp = MtwResponse::new(200)
            .with_header("content-type", "application/json")
            .with_body(b"{\"key\":\"value\"}".to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert!(resp.metadata.get("stream_events").is_none());
        } else {
            panic!("expected Continue");
        }
    }
}
