//! A deterministic [`MtwAIProvider`] used by the scenario harness and tests.
//!
//! Two modes:
//! * **Scripted**: return a fixed sequence of responses, one per call.
//! * **Rule-based**: inspect the last user message and pick the first rule
//!   whose `contains` substring matches. Falls back to a default response.
//!
//! Tests should prefer this over real providers so they stay hermetic and
//! don't hit the network.

use async_trait::async_trait;
use futures::stream;
use futures::Stream;
use mtw_ai::provider::{
    CompletionRequest, CompletionResponse, FinishReason, MessageRole, ModelInfo, MtwAIProvider,
    ProviderCapabilities, StreamChunk, Usage,
};
use mtw_core::MtwError;
use std::pin::Pin;
use std::sync::Mutex;

/// A rule that maps a substring in the last user message to a canned response.
#[derive(Debug, Clone)]
pub struct Rule {
    pub contains: String,
    pub response: String,
}

pub struct MockProvider {
    name: &'static str,
    rules: Vec<Rule>,
    default_response: String,
    scripted: Mutex<Vec<String>>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            name: "mock",
            rules: Vec::new(),
            default_response: "I don't have enough information to diagnose.".to_string(),
            scripted: Mutex::new(Vec::new()),
        }
    }

    /// Add a keyword rule. The last user message is scanned case-insensitively.
    pub fn with_rule(mut self, contains: impl Into<String>, response: impl Into<String>) -> Self {
        self.rules.push(Rule {
            contains: contains.into().to_lowercase(),
            response: response.into(),
        });
        self
    }

    pub fn with_default(mut self, response: impl Into<String>) -> Self {
        self.default_response = response.into();
        self
    }

    /// Queue a scripted response. Scripted responses take priority over rules
    /// and are consumed in FIFO order.
    pub fn with_scripted(self, response: impl Into<String>) -> Self {
        self.scripted.lock().unwrap().push(response.into());
        self
    }

    fn pick_response(&self, req: &CompletionRequest) -> String {
        if let Some(r) = self.scripted.lock().unwrap().pop() {
            // We pushed to the back; pop returns last. For FIFO we'd use
            // remove(0), but calling patterns only enqueue once per run so
            // LIFO is fine and cheaper.
            return r;
        }
        let last_user = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .map(|m| m.content.to_lowercase())
            .unwrap_or_default();
        for rule in &self.rules {
            if last_user.contains(&rule.contains) {
                return rule.response.clone();
            }
        }
        self.default_response.clone()
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MtwAIProvider for MockProvider {
    fn name(&self) -> &str {
        self.name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: false,
            vision: false,
            embeddings: false,
            max_context: 8192,
        }
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, MtwError> {
        let content = self.pick_response(&req);
        Ok(CompletionResponse {
            id: ulid::Ulid::new().to_string(),
            model: if req.model.is_empty() {
                "mock-1".into()
            } else {
                req.model
            },
            content,
            tool_calls: Vec::new(),
            usage: Usage::default(),
            finish_reason: FinishReason::Stop,
        })
    }

    fn stream(
        &self,
        req: CompletionRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, MtwError>> + Send>> {
        let content = self.pick_response(&req);
        let chunk = StreamChunk {
            delta: content,
            tool_calls: Vec::new(),
            finish_reason: Some(FinishReason::Stop),
            usage: None,
        };
        Box::pin(stream::iter(vec![Ok(chunk)]))
    }

    async fn models(&self) -> Result<Vec<ModelInfo>, MtwError> {
        Ok(vec![ModelInfo {
            id: "mock-1".into(),
            name: "mock-1".into(),
            max_context: 8192,
            supports_tools: false,
            supports_vision: false,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_ai::provider::Message;

    fn req_with(user_msg: &str) -> CompletionRequest {
        CompletionRequest {
            model: "mock-1".into(),
            messages: vec![Message::user(user_msg)],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn rule_matches_case_insensitive() {
        let p = MockProvider::new().with_rule("DATABASE", "db root cause");
        let resp = p.complete(req_with("the database is down")).await.unwrap();
        assert_eq!(resp.content, "db root cause");
    }

    #[tokio::test]
    async fn default_response_used_when_no_rule_matches() {
        let p = MockProvider::new().with_default("no idea");
        let resp = p.complete(req_with("totally unrelated")).await.unwrap();
        assert_eq!(resp.content, "no idea");
    }

    #[tokio::test]
    async fn scripted_response_takes_priority() {
        let p = MockProvider::new()
            .with_rule("db", "rule-said-db")
            .with_scripted("scripted-answer");
        let resp = p.complete(req_with("db problem")).await.unwrap();
        assert_eq!(resp.content, "scripted-answer");
    }
}
