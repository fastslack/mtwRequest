use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommChannel {
    Email,
    WhatsApp,
    Mattermost,
    Slack,
    Discord,
    Sms,
}

impl fmt::Display for CommChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Email => write!(f, "email"),
            Self::WhatsApp => write!(f, "whatsapp"),
            Self::Mattermost => write!(f, "mattermost"),
            Self::Slack => write!(f, "slack"),
            Self::Discord => write!(f, "discord"),
            Self::Sms => write!(f, "sms"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommDirection { Inbound, Outbound }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommStatus { Draft, Ready, Sending, Sent, Failed, Archived }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountType { Personal, Work, Transactional, Marketing }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountProvider { Gmail, Resend, Smtp }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_serialization() {
        let s = CommStatus::Sent;
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"sent\"");
    }
}
