use mtw_core::MtwError;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use crate::callback::CallbackRegistry;
use crate::command::{CommandRegistry, SlashCommand};
use crate::types::{MessageContext, OrchestratorResponse};

pub type DefaultHandlerFn = Arc<dyn Fn(String, MessageContext) -> Pin<Box<dyn Future<Output = Result<OrchestratorResponse, MtwError>> + Send>> + Send + Sync>;

pub struct MtwOrchestrator {
    pub commands: CommandRegistry,
    pub callbacks: CallbackRegistry,
    default_handler: Option<DefaultHandlerFn>,
}

impl MtwOrchestrator {
    pub fn new() -> Self {
        Self { commands: CommandRegistry::new(), callbacks: CallbackRegistry::new(), default_handler: None }
    }

    pub fn register_command(&mut self, cmd: SlashCommand) -> &mut Self { self.commands.register(cmd); self }
    pub fn set_default_handler(&mut self, handler: DefaultHandlerFn) -> &mut Self { self.default_handler = Some(handler); self }

    pub async fn handle_message(&self, text: &str, ctx: MessageContext) -> Result<OrchestratorResponse, MtwError> {
        if let Some((name, args)) = CommandRegistry::parse_command(text) {
            if let Some(cmd) = self.commands.get(&name) {
                return (cmd.handler)(args, ctx).await;
            }
            return Err(MtwError::Internal(format!("unknown command: /{}", name)));
        }
        if let Some(ref handler) = self.default_handler {
            return (handler)(text.to_string(), ctx).await;
        }
        Err(MtwError::Internal("no handler for non-command messages".into()))
    }

    pub async fn handle_callback(&self, data: &str, ctx: MessageContext) -> Result<OrchestratorResponse, MtwError> {
        self.callbacks.handle(data, ctx).await
    }

    pub fn help(&self) -> OrchestratorResponse {
        let mut lines = vec!["Available commands:".to_string()];
        for (name, desc) in self.commands.list() { lines.push(format!("/{} - {}", name, desc)); }
        OrchestratorResponse::new(lines.join("\n"))
    }
}

impl Default for MtwOrchestrator { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Platform;

    fn make_ctx() -> MessageContext {
        MessageContext { user_id: "u1".into(), chat_id: "c1".into(), platform: Platform::Api,
            username: None, display_name: None, is_group: false, group_name: None }
    }

    #[tokio::test]
    async fn test_command_routing() {
        let mut orch = MtwOrchestrator::new();
        orch.register_command(SlashCommand {
            name: "ping".into(), description: "Ping".into(), usage: None,
            handler: Arc::new(|_, _| Box::pin(async { Ok(OrchestratorResponse::new("pong")) })),
        });
        let r = orch.handle_message("/ping", make_ctx()).await.unwrap();
        assert_eq!(r.text, "pong");
    }

    #[tokio::test]
    async fn test_default_handler() {
        let mut orch = MtwOrchestrator::new();
        orch.set_default_handler(Arc::new(|text, _| Box::pin(async move { Ok(OrchestratorResponse::new(format!("echo: {}", text))) })));
        let r = orch.handle_message("hello", make_ctx()).await.unwrap();
        assert_eq!(r.text, "echo: hello");
    }

    #[tokio::test]
    async fn test_unknown_command() {
        let orch = MtwOrchestrator::new();
        assert!(orch.handle_message("/nonexistent", make_ctx()).await.is_err());
    }
}
