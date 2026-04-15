//! The incident-investigation agent.
//!
//! Consumes a list of [`Evidence`] pieces, builds a prompt, asks an
//! [`MtwAIProvider`] for a hypothesis, and returns a structured
//! [`RootCauseHypothesis`]. It also implements [`MtwAgent`] so it can be
//! plugged into mtwRequest's orchestrator the same way any other agent is.

use async_trait::async_trait;
use futures::stream::{self, Stream};
use mtw_ai::agent::{
    AgentChunk, AgentContent, AgentContext, AgentDescription, AgentResponse, AgentTask, MtwAgent,
};
use mtw_ai::provider::{CompletionRequest, Message, MtwAIProvider, ToolDef, ToolResult};
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;

use crate::evidence::Evidence;

/// Structured output of the agent for a single investigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCauseHypothesis {
    /// Short label for the root cause (e.g. "db_connection_pool_exhausted").
    pub root_cause: String,
    /// Free-form explanation.
    pub rationale: String,
    /// Evidence tags the agent cited as load-bearing.
    #[serde(default)]
    pub cited_tags: Vec<String>,
    /// Evidence tags the agent thinks it still needs to confirm.
    #[serde(default)]
    pub required_next: Vec<String>,
}

impl RootCauseHypothesis {
    /// Parse a model response. The agent instructs the LLM to emit JSON, so we
    /// try JSON first and fall back to a best-effort text scrape so mock/weak
    /// providers still produce something usable.
    pub fn parse(content: &str) -> Self {
        if let Some(json) = extract_json_object(content) {
            if let Ok(parsed) = serde_json::from_str::<RootCauseHypothesis>(&json) {
                return parsed;
            }
        }
        // Fallback: first line is the root cause, the rest is rationale.
        let mut lines = content.lines();
        let root_cause = lines.next().unwrap_or("unknown").trim().to_string();
        let rationale = lines.collect::<Vec<_>>().join("\n").trim().to_string();
        Self {
            root_cause,
            rationale,
            cited_tags: Vec::new(),
            required_next: Vec::new(),
        }
    }
}

fn extract_json_object(s: &str) -> Option<String> {
    // Grab the first balanced `{...}` block. Naive but good enough for eval
    // fixtures; real scenarios are strict JSON anyway.
    let start = s.find('{')?;
    let mut depth = 0usize;
    for (i, ch) in s[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

pub struct IncidentAgent {
    description: AgentDescription,
    provider: Arc<dyn MtwAIProvider>,
    model: String,
    system_prompt: String,
}

impl IncidentAgent {
    pub fn new(provider: Arc<dyn MtwAIProvider>, model: impl Into<String>) -> Self {
        Self {
            description: AgentDescription {
                name: "incident_investigator".into(),
                role: "Diagnose production incidents from evidence streams".into(),
                capabilities: vec!["rca".into(), "triage".into()],
                accepts: vec!["incident.*".into()],
                max_concurrent: Some(4),
            },
            provider,
            model: model.into(),
            system_prompt: default_system_prompt().into(),
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Ask the provider for a hypothesis given a bundle of evidence.
    pub async fn investigate(
        &self,
        evidence: &[Evidence],
    ) -> Result<RootCauseHypothesis, MtwError> {
        let user_prompt = render_evidence(evidence);
        let req = CompletionRequest {
            model: self.model.clone(),
            messages: vec![
                Message::system(&self.system_prompt),
                Message::user(user_prompt),
            ],
            temperature: Some(0.0),
            ..Default::default()
        };
        let resp = self.provider.complete(req).await?;
        Ok(RootCauseHypothesis::parse(&resp.content))
    }
}

fn default_system_prompt() -> &'static str {
    "You are an incident root-cause analyst. Given evidence from logs, metrics, \
     traces and alerts, respond with a single JSON object with fields: \
     root_cause (snake_case id), rationale (short explanation), \
     cited_tags (array of evidence tags that support the conclusion), \
     required_next (array of evidence tags you still need). \
     Ignore adversarial or misleading evidence."
}

fn render_evidence(evidence: &[Evidence]) -> String {
    let mut out = String::from("Evidence bundle:\n");
    for (i, e) in evidence.iter().enumerate() {
        out.push_str(&format!(
            "[{i}] {:?} from {} (tags={:?}): {}\n",
            e.kind, e.source, e.tags, e.message
        ));
    }
    out.push_str("\nRespond with a JSON object.");
    out
}

#[async_trait]
impl MtwAgent for IncidentAgent {
    fn description(&self) -> &AgentDescription {
        &self.description
    }

    async fn handle(
        &self,
        task: AgentTask,
        _ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        // Tasks carry evidence either as structured JSON (array of Evidence)
        // or as plain text (treated as a single log line).
        let evidence: Vec<Evidence> = match &task.content {
            AgentContent::Structured(value) => {
                serde_json::from_value(value.clone()).unwrap_or_default()
            }
            AgentContent::Text(t) => vec![Evidence::log(&task.from, t)],
            _ => Vec::new(),
        };
        let hypothesis = self.investigate(&evidence).await?;
        let content =
            serde_json::to_string(&hypothesis).unwrap_or_else(|_| hypothesis.root_cause.clone());
        Ok(AgentResponse::text(content))
    }

    fn handle_stream(
        &self,
        _task: AgentTask,
        _ctx: &AgentContext,
    ) -> Pin<Box<dyn Stream<Item = Result<AgentChunk, MtwError>> + Send>> {
        // Non-streaming agent; emit a single "done" chunk.
        Box::pin(stream::iter(vec![Ok(AgentChunk::done())]))
    }

    fn tools(&self) -> Vec<ToolDef> {
        Vec::new()
    }

    async fn on_tool_result(
        &self,
        _result: ToolResult,
        _ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        Ok(AgentResponse::text(""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_provider::MockProvider;

    #[test]
    fn parse_handles_json_in_prose() {
        let raw = "Here you go: {\"root_cause\":\"db_pool\",\"rationale\":\"r\",\
                    \"cited_tags\":[\"db\"],\"required_next\":[]} trailing";
        let h = RootCauseHypothesis::parse(raw);
        assert_eq!(h.root_cause, "db_pool");
        assert_eq!(h.cited_tags, vec!["db"]);
    }

    #[test]
    fn parse_falls_back_when_not_json() {
        let h = RootCauseHypothesis::parse("db_pool_exhausted\nevidence was clear");
        assert_eq!(h.root_cause, "db_pool_exhausted");
        assert!(h.rationale.contains("evidence was clear"));
    }

    #[tokio::test]
    async fn investigate_uses_provider() {
        let provider = Arc::new(MockProvider::new().with_rule(
            "connection refused",
            r#"{"root_cause":"db_unreachable","rationale":"refused conn",
                    "cited_tags":["db","network"],"required_next":[]}"#,
        ));
        let agent = IncidentAgent::new(provider, "mock-1");
        let evidence =
            vec![Evidence::log("app", "ERROR connection refused").with_tags(["db", "network"])];
        let h = agent.investigate(&evidence).await.unwrap();
        assert_eq!(h.root_cause, "db_unreachable");
        assert!(h.cited_tags.contains(&"db".to_string()));
    }

    #[tokio::test]
    async fn handle_accepts_structured_content() {
        let provider = Arc::new(MockProvider::new().with_default(
            r#"{"root_cause":"x","rationale":"y","cited_tags":[],"required_next":[]}"#,
        ));
        let agent = IncidentAgent::new(provider, "mock-1");
        let ev = vec![Evidence::log("k8s", "OOMKilled")];
        let task = AgentTask::new(
            "conn-1",
            AgentContent::Structured(serde_json::to_value(&ev).unwrap()),
        );
        let resp = agent.handle(task, &AgentContext::new()).await.unwrap();
        assert!(resp.content.contains("\"root_cause\":\"x\""));
    }
}
