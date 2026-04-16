use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Telegram, Mattermost, WhatsApp, Slack, Discord, WebChat, Signal, Matrix, Api,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Telegram => write!(f, "telegram"), Self::Mattermost => write!(f, "mattermost"),
            Self::WhatsApp => write!(f, "whatsapp"), Self::Slack => write!(f, "slack"),
            Self::Discord => write!(f, "discord"), Self::WebChat => write!(f, "webchat"),
            Self::Signal => write!(f, "signal"), Self::Matrix => write!(f, "matrix"),
            Self::Api => write!(f, "api"),
        }
    }
}

impl FromStr for Platform {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "telegram" => Ok(Self::Telegram), "mattermost" => Ok(Self::Mattermost),
            "whatsapp" => Ok(Self::WhatsApp), "slack" => Ok(Self::Slack),
            "discord" => Ok(Self::Discord), "webchat" => Ok(Self::WebChat),
            "signal" => Ok(Self::Signal), "matrix" => Ok(Self::Matrix), "api" => Ok(Self::Api),
            _ => Err(format!("unknown platform: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageContext {
    pub user_id: String, pub chat_id: String, pub platform: Platform,
    pub username: Option<String>, pub display_name: Option<String>,
    pub is_group: bool, pub group_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseMode { Markdown, Html, Plain }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineButton { pub text: String, pub callback_data: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorResponse {
    pub text: String, pub parse_mode: Option<ParseMode>,
    pub buttons: Vec<Vec<InlineButton>>,
}

impl OrchestratorResponse {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), parse_mode: None, buttons: Vec::new() }
    }
    pub fn with_parse_mode(mut self, mode: ParseMode) -> Self { self.parse_mode = Some(mode); self }
    pub fn with_button_row(mut self, row: Vec<InlineButton>) -> Self { self.buttons.push(row); self }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_platform_roundtrip() {
        let p = Platform::Telegram;
        assert_eq!(p.to_string(), "telegram");
        assert_eq!(Platform::from_str("telegram").unwrap(), p);
    }
    #[test]
    fn test_response_builder() {
        let r = OrchestratorResponse::new("Hello")
            .with_parse_mode(ParseMode::Markdown)
            .with_button_row(vec![InlineButton { text: "OK".into(), callback_data: "ok".into() }]);
        assert_eq!(r.text, "Hello");
        assert_eq!(r.buttons.len(), 1);
    }
}
