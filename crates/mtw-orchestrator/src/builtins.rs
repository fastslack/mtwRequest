use std::sync::Arc;
use crate::command::SlashCommand;
use crate::orchestrator::MtwOrchestrator;
use crate::types::OrchestratorResponse;

pub fn register_builtins(orch: &mut MtwOrchestrator) {
    orch.register_command(SlashCommand {
        name: "help".into(), description: "Show available commands".into(), usage: None,
        handler: Arc::new(|_, _| Box::pin(async { Ok(OrchestratorResponse::new("Use /help to see commands")) })),
    });

    orch.register_command(SlashCommand {
        name: "ping".into(), description: "Check if server is alive".into(), usage: None,
        handler: Arc::new(|_, _| Box::pin(async {
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            Ok(OrchestratorResponse::new(format!("pong ({})", ts)))
        })),
    });

    orch.register_command(SlashCommand {
        name: "status".into(), description: "Show system status".into(), usage: None,
        handler: Arc::new(|_, ctx| Box::pin(async move {
            Ok(OrchestratorResponse::new(format!("System online | Platform: {} | User: {}", ctx.platform, ctx.user_id)))
        })),
    });
}
