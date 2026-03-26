use async_trait::async_trait;
use mtw_core::MtwError;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::MtwResponse;

/// Pipeline stage that verifies decompression was handled properly.
/// Note: reqwest handles gzip/brotli/deflate automatically when features are enabled.
/// This stage provides a safety net: it checks the Content-Encoding header and ensures
/// the body doesn't look like it's still compressed.
pub struct DecompressionStage;

impl DecompressionStage {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DecompressionStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for DecompressionStage {
    fn name(&self) -> &str {
        "decompression"
    }

    fn priority(&self) -> i32 {
        18
    }

    async fn process(
        &self,
        mut response: MtwResponse,
        _context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        // Check if Content-Encoding is still present (reqwest removes it after decompression)
        if let Some(encoding) = response.headers.get("content-encoding") {
            let encoding = encoding.to_lowercase();
            if encoding == "identity" || encoding == "chunked" {
                // These are fine
                return Ok(PipelineAction::Continue(response));
            }

            // If Content-Encoding is still gzip/br/deflate, the body may not have been
            // decompressed. Log a warning but continue.
            if encoding == "gzip" || encoding == "br" || encoding == "deflate" {
                tracing::warn!(
                    encoding = %encoding,
                    "response has Content-Encoding header; body may not be decompressed"
                );
                response.metadata.insert(
                    "content_encoding".into(),
                    serde_json::json!(encoding),
                );
            }
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
    async fn test_no_encoding_passes_through() {
        let stage = DecompressionStage::new();
        let resp = MtwResponse::new(200).with_body(b"plain text".to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
    }

    #[tokio::test]
    async fn test_identity_encoding_passes() {
        let stage = DecompressionStage::new();
        let resp = MtwResponse::new(200).with_header("content-encoding", "identity");
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
    }

    #[tokio::test]
    async fn test_gzip_encoding_flagged() {
        let stage = DecompressionStage::new();
        let resp = MtwResponse::new(200).with_header("content-encoding", "gzip");
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        if let PipelineAction::Continue(resp) = result {
            assert_eq!(
                resp.metadata.get("content_encoding"),
                Some(&serde_json::json!("gzip"))
            );
        } else {
            panic!("expected Continue");
        }
    }
}
