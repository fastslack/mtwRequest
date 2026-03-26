use async_trait::async_trait;
use mtw_core::MtwError;
use std::collections::HashSet;

use crate::pipeline::{PipelineAction, PipelineContext, PipelineStage};
use crate::response::MtwResponse;

/// Log level for the logging stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Configuration for the logging stage.
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Default log level for successful responses.
    pub success_level: LogLevel,
    /// Log level for client error responses (4xx).
    pub client_error_level: LogLevel,
    /// Log level for server error responses (5xx).
    pub server_error_level: LogLevel,
    /// Whether to log response headers.
    pub log_headers: bool,
    /// Whether to log response body (truncated).
    pub log_body: bool,
    /// Maximum body length to log.
    pub max_body_log_length: usize,
    /// Header values to redact in logs.
    pub redact_headers: HashSet<String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        let mut redact = HashSet::new();
        redact.insert("authorization".to_string());
        redact.insert("cookie".to_string());
        redact.insert("set-cookie".to_string());
        redact.insert("x-api-key".to_string());

        Self {
            success_level: LogLevel::Debug,
            client_error_level: LogLevel::Warn,
            server_error_level: LogLevel::Error,
            log_headers: false,
            log_body: false,
            max_body_log_length: 1024,
            redact_headers: redact,
        }
    }
}

/// Pipeline stage that logs request/response details.
pub struct LoggingStage {
    config: LoggingConfig,
}

impl LoggingStage {
    pub fn new() -> Self {
        Self {
            config: LoggingConfig::default(),
        }
    }

    pub fn with_config(config: LoggingConfig) -> Self {
        Self { config }
    }

    fn log_level_for_status(&self, status: u16) -> &LogLevel {
        if (200..400).contains(&status) {
            &self.config.success_level
        } else if (400..500).contains(&status) {
            &self.config.client_error_level
        } else {
            &self.config.server_error_level
        }
    }

    fn redact_header_value(&self, key: &str) -> bool {
        self.config.redact_headers.contains(&key.to_lowercase())
    }
}

impl Default for LoggingStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineStage for LoggingStage {
    fn name(&self) -> &str {
        "logging"
    }

    fn priority(&self) -> i32 {
        90
    }

    async fn process(
        &self,
        response: MtwResponse,
        context: &mut PipelineContext,
    ) -> Result<PipelineAction, MtwError> {
        let level = self.log_level_for_status(response.status);
        let method = &context.request.method;
        let url = &context.request.url;
        let status = response.status;
        let duration_ms = response.timing.duration.as_millis();

        let base_msg = format!("{} {} -> {} ({}ms)", method, url, status, duration_ms);

        match level {
            LogLevel::Trace => tracing::trace!("{}", base_msg),
            LogLevel::Debug => tracing::debug!("{}", base_msg),
            LogLevel::Info => tracing::info!("{}", base_msg),
            LogLevel::Warn => tracing::warn!("{}", base_msg),
            LogLevel::Error => tracing::error!("{}", base_msg),
        }

        if self.config.log_headers {
            for (key, value) in &response.headers {
                let display_value = if self.redact_header_value(key) {
                    "[REDACTED]".to_string()
                } else {
                    value.clone()
                };
                tracing::debug!(header = %key, value = %display_value);
            }
        }

        if self.config.log_body {
            let body_str = match &response.body {
                crate::response::ResponseBody::Bytes(b) => {
                    let s = String::from_utf8_lossy(b);
                    if s.len() > self.config.max_body_log_length {
                        format!("{}...[truncated]", &s[..self.config.max_body_log_length])
                    } else {
                        s.to_string()
                    }
                }
                crate::response::ResponseBody::Json(v) => {
                    let s = v.to_string();
                    if s.len() > self.config.max_body_log_length {
                        format!("{}...[truncated]", &s[..self.config.max_body_log_length])
                    } else {
                        s
                    }
                }
                crate::response::ResponseBody::Empty => "[empty]".to_string(),
            };
            tracing::debug!(body = %body_str);
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
    async fn test_logging_passes_through() {
        let stage = LoggingStage::new();
        let resp = MtwResponse::new(200);
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(r) if r.status == 200));
    }

    #[tokio::test]
    async fn test_log_level_selection() {
        let stage = LoggingStage::new();
        assert_eq!(stage.log_level_for_status(200), &LogLevel::Debug);
        assert_eq!(stage.log_level_for_status(301), &LogLevel::Debug);
        assert_eq!(stage.log_level_for_status(404), &LogLevel::Warn);
        assert_eq!(stage.log_level_for_status(500), &LogLevel::Error);
    }

    #[tokio::test]
    async fn test_header_redaction() {
        let stage = LoggingStage::new();
        assert!(stage.redact_header_value("authorization"));
        assert!(stage.redact_header_value("Authorization"));
        assert!(stage.redact_header_value("cookie"));
        assert!(!stage.redact_header_value("content-type"));
    }

    #[tokio::test]
    async fn test_logging_with_headers_and_body() {
        let config = LoggingConfig {
            log_headers: true,
            log_body: true,
            ..Default::default()
        };
        let stage = LoggingStage::with_config(config);
        let resp = MtwResponse::new(200)
            .with_header("content-type", "application/json")
            .with_body(b"{\"hello\":\"world\"}".to_vec());
        let mut ctx = make_ctx();

        let result = stage.process(resp, &mut ctx).await.unwrap();
        assert!(matches!(result, PipelineAction::Continue(_)));
    }
}
