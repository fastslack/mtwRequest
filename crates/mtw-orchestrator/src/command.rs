use mtw_core::MtwError;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use crate::types::{MessageContext, OrchestratorResponse};

pub type CommandHandlerFn = Arc<dyn Fn(Vec<String>, MessageContext) -> Pin<Box<dyn Future<Output = Result<OrchestratorResponse, MtwError>> + Send>> + Send + Sync>;

pub struct SlashCommand {
    pub name: String, pub description: String, pub usage: Option<String>,
    pub handler: CommandHandlerFn,
}

pub struct CommandRegistry { commands: HashMap<String, SlashCommand> }

impl CommandRegistry {
    pub fn new() -> Self { Self { commands: HashMap::new() } }

    pub fn register(&mut self, cmd: SlashCommand) { self.commands.insert(cmd.name.clone(), cmd); }

    pub fn get(&self, name: &str) -> Option<&SlashCommand> { self.commands.get(name) }

    pub fn list(&self) -> Vec<(&str, &str)> {
        self.commands.iter().map(|(k, v)| (k.as_str(), v.description.as_str())).collect()
    }

    pub fn parse_command(text: &str) -> Option<(String, Vec<String>)> {
        let text = text.trim();
        if !text.starts_with('/') { return None; }
        let parts: Vec<&str> = text[1..].splitn(2, char::is_whitespace).collect();
        let name = parts[0].to_string();
        let args = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else { Vec::new() };
        Some((name, args))
    }
}

impl Default for CommandRegistry { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_command() {
        let (name, args) = CommandRegistry::parse_command("/task buy milk").unwrap();
        assert_eq!(name, "task");
        assert_eq!(args, vec!["buy", "milk"]);
    }
    #[test]
    fn test_parse_no_args() {
        let (name, args) = CommandRegistry::parse_command("/help").unwrap();
        assert_eq!(name, "help");
        assert!(args.is_empty());
    }
    #[test]
    fn test_not_a_command() {
        assert!(CommandRegistry::parse_command("hello world").is_none());
    }
}
