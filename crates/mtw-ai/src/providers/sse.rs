//! Shared SSE (Server-Sent Events) line parser for OpenAI-compatible streaming
//! APIs. Used by both [`super::openai`] and [`super::lmstudio`].

use futures::stream::StreamExt;
use futures::Stream;
use mtw_core::MtwError;
use std::pin::Pin;

use crate::provider::{FinishReason, StreamChunk, ToolCall, Usage};
use super::openai::{parse_tool_calls, OaiResponse};

/// Parse an OpenAI-compatible SSE byte stream into a stream of [`StreamChunk`].
///
/// `provider_label` is used only in error messages (e.g. "openai", "lmstudio").
pub(crate) fn parse_oai_sse_stream<E: std::fmt::Display + Send + 'static>(
    bytes_stream: impl Stream<Item = Result<::bytes::Bytes, E>> + Send + 'static,
    provider_label: &'static str,
    with_tool_calls: bool,
) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>> {
    Box::pin(async_stream::try_stream! {
        let mut stream = Box::pin(bytes_stream);
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| MtwError::Internal(format!("{} stream read: {}", provider_label, e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer = buffer[line_end + 1..].to_string();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    if data.trim() == "[DONE]" {
                        return;
                    }

                    match serde_json::from_str::<OaiResponse>(data) {
                        Ok(parsed) => {
                            if let Some(choices) = &parsed.choices {
                                if let Some(choice) = choices.first() {
                                    let delta_content = choice
                                        .delta
                                        .as_ref()
                                        .and_then(|d| d.content.clone())
                                        .unwrap_or_default();
                                    let tool_calls = if with_tool_calls {
                                        choice
                                            .delta
                                            .as_ref()
                                            .and_then(|d| d.tool_calls.as_ref())
                                            .map(|tc| parse_tool_calls(tc))
                                            .unwrap_or_default()
                                    } else {
                                        Vec::<ToolCall>::new()
                                    };
                                    let finish_reason = choice
                                        .finish_reason
                                        .as_deref()
                                        .map(FinishReason::from_openai);
                                    let usage = parsed.usage.as_ref().map(|u| Usage {
                                        prompt_tokens: u.prompt_tokens.unwrap_or(0),
                                        completion_tokens: u.completion_tokens.unwrap_or(0),
                                        total_tokens: u.total_tokens.unwrap_or(0),
                                    });

                                    yield StreamChunk {
                                        delta: delta_content,
                                        tool_calls,
                                        finish_reason,
                                        usage,
                                    };
                                }
                            }
                        }
                        Err(_) => { /* skip unparseable lines */ }
                    }
                }
            }
        }
    })
}
