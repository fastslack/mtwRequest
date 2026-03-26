use futures::Stream;
use mtw_core::MtwError;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use crate::agent::{AgentChunk, AgentContext, AgentResponse, AgentTask, MtwAgent};

/// Strategy for routing tasks to agents
#[derive(Debug, Clone)]
pub enum RoutingStrategy {
    /// Route based on channel name matching agent accepts
    ChannelBased,
    /// Route through agents in sequence (pipeline)
    Pipeline(Vec<String>),
    /// Send to all agents and collect results
    FanOut,
    /// Round-robin across agents
    RoundRobin,
}

/// Multi-agent orchestrator -- routes tasks to the appropriate agent(s)
pub struct AgentOrchestrator {
    agents: HashMap<String, Arc<dyn MtwAgent>>,
    strategy: RoutingStrategy,
    round_robin_index: std::sync::atomic::AtomicUsize,
}

impl AgentOrchestrator {
    pub fn new(strategy: RoutingStrategy) -> Self {
        Self {
            agents: HashMap::new(),
            strategy,
            round_robin_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Register an agent with the orchestrator
    pub fn register_agent(&mut self, agent: Arc<dyn MtwAgent>) {
        let name = agent.description().name.clone();
        tracing::info!(agent = %name, "registered agent with orchestrator");
        self.agents.insert(name, agent);
    }

    /// Remove an agent by name
    pub fn remove_agent(&mut self, name: &str) -> Option<Arc<dyn MtwAgent>> {
        self.agents.remove(name)
    }

    /// Get an agent by name
    pub fn get_agent(&self, name: &str) -> Option<&Arc<dyn MtwAgent>> {
        self.agents.get(name)
    }

    /// List all registered agent names
    pub fn agent_names(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    /// Route a task to the appropriate agent and get a response
    pub async fn route(
        &self,
        task: AgentTask,
        ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        match &self.strategy {
            RoutingStrategy::ChannelBased => self.route_channel_based(task, ctx).await,
            RoutingStrategy::Pipeline(agents) => {
                self.route_pipeline(agents.clone(), task, ctx).await
            }
            RoutingStrategy::FanOut => self.route_fan_out(task, ctx).await,
            RoutingStrategy::RoundRobin => self.route_round_robin(task, ctx).await,
        }
    }

    /// Route a task with streaming response
    pub fn route_stream(
        &self,
        task: AgentTask,
        ctx: &AgentContext,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<AgentChunk, MtwError>> + Send>>, MtwError> {
        match &self.strategy {
            RoutingStrategy::ChannelBased => {
                let agent = self.find_agent_for_channel(task.channel.as_deref())?;
                Ok(agent.handle_stream(task, ctx))
            }
            RoutingStrategy::RoundRobin => {
                let agent = self.next_round_robin()?;
                Ok(agent.handle_stream(task, ctx))
            }
            RoutingStrategy::Pipeline(_) | RoutingStrategy::FanOut => {
                Err(MtwError::Agent(
                    "streaming not supported for pipeline/fan-out routing".into(),
                ))
            }
        }
    }

    /// Find an agent whose accepts list matches the task channel
    fn find_agent_for_channel(
        &self,
        channel: Option<&str>,
    ) -> Result<&Arc<dyn MtwAgent>, MtwError> {
        let channel = channel.unwrap_or("default");
        for agent in self.agents.values() {
            let desc = agent.description();
            for pattern in &desc.accepts {
                if channel_matches(pattern, channel) {
                    return Ok(agent);
                }
            }
        }
        Err(MtwError::Agent(format!(
            "no agent found for channel: {}",
            channel
        )))
    }

    async fn route_channel_based(
        &self,
        task: AgentTask,
        ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        let agent = self.find_agent_for_channel(task.channel.as_deref())?;
        agent.handle(task, ctx).await
    }

    async fn route_pipeline(
        &self,
        agent_names: Vec<String>,
        mut task: AgentTask,
        ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        let mut last_response = None;

        for name in &agent_names {
            let agent = self.agents.get(name).ok_or_else(|| {
                MtwError::Agent(format!("pipeline agent not found: {}", name))
            })?;

            let response = agent.handle(task.clone(), ctx).await?;

            // Feed the output of one agent as input to the next
            task = AgentTask::text(&task.from, &response.content);
            if let Some(ch) = task.channel.as_ref() {
                task.channel = Some(ch.clone());
            }
            last_response = Some(response);
        }

        last_response.ok_or_else(|| MtwError::Agent("empty pipeline".into()))
    }

    async fn route_fan_out(
        &self,
        task: AgentTask,
        ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        if self.agents.is_empty() {
            return Err(MtwError::Agent("no agents registered".into()));
        }

        let mut results = Vec::new();
        for agent in self.agents.values() {
            match agent.handle(task.clone(), ctx).await {
                Ok(response) => results.push(response),
                Err(e) => {
                    tracing::warn!(
                        agent = %agent.description().name,
                        error = %e,
                        "fan-out agent failed"
                    );
                }
            }
        }

        if results.is_empty() {
            return Err(MtwError::Agent("all fan-out agents failed".into()));
        }

        // Merge results: concatenate content from all agents
        let merged_content = results
            .iter()
            .map(|r| r.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        Ok(AgentResponse::text(merged_content))
    }

    fn next_round_robin(&self) -> Result<&Arc<dyn MtwAgent>, MtwError> {
        if self.agents.is_empty() {
            return Err(MtwError::Agent("no agents registered".into()));
        }

        let agents: Vec<&Arc<dyn MtwAgent>> = self.agents.values().collect();
        let idx = self
            .round_robin_index
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % agents.len();
        Ok(agents[idx])
    }

    async fn route_round_robin(
        &self,
        task: AgentTask,
        ctx: &AgentContext,
    ) -> Result<AgentResponse, MtwError> {
        let agent = self.next_round_robin()?;
        agent.handle(task, ctx).await
    }
}

/// Simple glob-style channel matching (supports trailing `*`)
fn channel_matches(pattern: &str, channel: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        channel.starts_with(prefix)
    } else {
        pattern == channel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentChunk, AgentDescription};
    use crate::provider::{ToolDef, ToolResult};
    use async_trait::async_trait;

    struct TestAgent {
        desc: AgentDescription,
        response: String,
    }

    impl TestAgent {
        fn new(name: &str, accepts: Vec<&str>, response: &str) -> Self {
            Self {
                desc: AgentDescription {
                    name: name.to_string(),
                    role: format!("Test agent: {}", name),
                    capabilities: vec![],
                    accepts: accepts.into_iter().map(String::from).collect(),
                    max_concurrent: None,
                },
                response: response.to_string(),
            }
        }
    }

    #[async_trait]
    impl MtwAgent for TestAgent {
        fn description(&self) -> &AgentDescription {
            &self.desc
        }

        async fn handle(
            &self,
            _task: AgentTask,
            _ctx: &AgentContext,
        ) -> Result<AgentResponse, MtwError> {
            Ok(AgentResponse::text(&self.response))
        }

        fn handle_stream(
            &self,
            _task: AgentTask,
            _ctx: &AgentContext,
        ) -> Pin<Box<dyn Stream<Item = Result<AgentChunk, MtwError>> + Send>> {
            let response = self.response.clone();
            Box::pin(futures::stream::once(async move {
                Ok(AgentChunk::text(response))
            }))
        }

        fn tools(&self) -> Vec<ToolDef> {
            vec![]
        }

        async fn on_tool_result(
            &self,
            _result: ToolResult,
            _ctx: &AgentContext,
        ) -> Result<AgentResponse, MtwError> {
            Ok(AgentResponse::text("tool result processed"))
        }
    }

    #[test]
    fn test_channel_matching() {
        assert!(channel_matches("chat.*", "chat.general"));
        assert!(channel_matches("chat.*", "chat.private"));
        assert!(!channel_matches("chat.*", "code-review"));
        assert!(channel_matches("*", "anything"));
        assert!(channel_matches("exact", "exact"));
        assert!(!channel_matches("exact", "other"));
    }

    #[tokio::test]
    async fn test_channel_based_routing() {
        let mut orch = AgentOrchestrator::new(RoutingStrategy::ChannelBased);
        orch.register_agent(Arc::new(TestAgent::new(
            "chat-agent",
            vec!["chat.*"],
            "chat response",
        )));
        orch.register_agent(Arc::new(TestAgent::new(
            "code-agent",
            vec!["code-review"],
            "code response",
        )));

        let ctx = AgentContext::new();

        let task = AgentTask::text("conn-1", "hello").with_channel("chat.general");
        let resp = orch.route(task, &ctx).await.unwrap();
        assert_eq!(resp.content, "chat response");

        let task = AgentTask::text("conn-1", "review").with_channel("code-review");
        let resp = orch.route(task, &ctx).await.unwrap();
        assert_eq!(resp.content, "code response");
    }

    #[tokio::test]
    async fn test_no_agent_found() {
        let orch = AgentOrchestrator::new(RoutingStrategy::ChannelBased);
        let ctx = AgentContext::new();
        let task = AgentTask::text("conn-1", "hello").with_channel("unknown");
        let result = orch.route(task, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pipeline_routing() {
        let mut orch = AgentOrchestrator::new(RoutingStrategy::Pipeline(vec![
            "first".to_string(),
            "second".to_string(),
        ]));
        orch.register_agent(Arc::new(TestAgent::new("first", vec![], "first output")));
        orch.register_agent(Arc::new(TestAgent::new("second", vec![], "second output")));

        let ctx = AgentContext::new();
        let task = AgentTask::text("conn-1", "start");
        let resp = orch.route(task, &ctx).await.unwrap();
        // Pipeline returns the last agent's output
        assert_eq!(resp.content, "second output");
    }

    #[tokio::test]
    async fn test_fan_out_routing() {
        let mut orch = AgentOrchestrator::new(RoutingStrategy::FanOut);
        orch.register_agent(Arc::new(TestAgent::new("a", vec![], "response A")));
        orch.register_agent(Arc::new(TestAgent::new("b", vec![], "response B")));

        let ctx = AgentContext::new();
        let task = AgentTask::text("conn-1", "query");
        let resp = orch.route(task, &ctx).await.unwrap();
        // Fan-out merges results
        assert!(resp.content.contains("response A") || resp.content.contains("response B"));
    }

    #[test]
    fn test_register_and_list() {
        let mut orch = AgentOrchestrator::new(RoutingStrategy::ChannelBased);
        orch.register_agent(Arc::new(TestAgent::new("agent-1", vec![], "resp")));
        orch.register_agent(Arc::new(TestAgent::new("agent-2", vec![], "resp")));

        let names = orch.agent_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"agent-1".to_string()));
        assert!(names.contains(&"agent-2".to_string()));
    }
}
