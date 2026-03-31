use async_trait::async_trait;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Capabilities a notification provider supports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyCapabilities {
    pub supports_attachments: bool,
    pub supports_buttons: bool,
    pub supports_markdown: bool,
    pub supports_html: bool,
    pub max_message_length: Option<usize>,
}

impl Default for NotifyCapabilities {
    fn default() -> Self {
        Self {
            supports_attachments: false,
            supports_buttons: false,
            supports_markdown: true,
            supports_html: false,
            max_message_length: None,
        }
    }
}

/// Notification priority level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPriority {
    Low,
    Normal,
    High,
    Critical,
}

impl Default for NotificationPriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// Attachment type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentType {
    Image,
    Audio,
    Video,
    Document,
    Voice,
}

/// A notification attachment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationAttachment {
    pub attachment_type: AttachmentType,
    pub url: Option<String>,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub caption: Option<String>,
}

/// An inline button for interactive notifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationButton {
    pub text: String,
    pub callback_data: String,
}

/// A notification to be sent through one or more channels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub priority: NotificationPriority,
    #[serde(default)]
    pub silent: bool,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub attachments: Vec<NotificationAttachment>,
    #[serde(default)]
    pub buttons: Vec<Vec<NotificationButton>>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Notification {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            title: title.into(),
            body: None,
            channel: None,
            priority: NotificationPriority::Normal,
            silent: false,
            source: None,
            attachments: Vec::new(),
            buttons: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    pub fn with_priority(mut self, priority: NotificationPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_attachment(mut self, attachment: NotificationAttachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    pub fn with_button_row(mut self, buttons: Vec<NotificationButton>) -> Self {
        self.buttons.push(buttons);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

/// Status of a notification provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyStatus {
    pub provider_id: String,
    pub connected: bool,
    pub authenticated: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub info: HashMap<String, serde_json::Value>,
}

/// Notification provider trait
#[async_trait]
pub trait MtwNotifyProvider: Send + Sync {
    /// Unique provider identifier
    fn id(&self) -> &str;

    /// Human-readable provider name
    fn name(&self) -> &str;

    /// Provider capabilities
    fn capabilities(&self) -> NotifyCapabilities;

    /// Send a notification, returns message ID
    async fn send(&self, notification: &Notification) -> Result<String, MtwError>;

    /// Start the provider (connect, authenticate)
    async fn start(&self) -> Result<(), MtwError>;

    /// Stop the provider (disconnect, cleanup)
    async fn stop(&self) -> Result<(), MtwError>;

    /// Check if provider is ready to send
    fn is_ready(&self) -> bool;

    /// Get provider status
    fn status(&self) -> NotifyStatus;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_builder() {
        let n = Notification::new("Alert")
            .with_body("Something happened")
            .with_channel("telegram")
            .with_priority(NotificationPriority::High)
            .with_source("trading")
            .with_silent(true);

        assert_eq!(n.title, "Alert");
        assert_eq!(n.body, Some("Something happened".to_string()));
        assert_eq!(n.channel, Some("telegram".to_string()));
        assert_eq!(n.priority, NotificationPriority::High);
        assert!(n.silent);
        assert_eq!(n.source, Some("trading".to_string()));
    }

    #[test]
    fn test_notification_with_buttons() {
        let n = Notification::new("Task due")
            .with_button_row(vec![
                NotificationButton {
                    text: "Complete".into(),
                    callback_data: "complete:task:123".into(),
                },
                NotificationButton {
                    text: "Snooze".into(),
                    callback_data: "snooze:task:123".into(),
                },
            ]);

        assert_eq!(n.buttons.len(), 1);
        assert_eq!(n.buttons[0].len(), 2);
        assert_eq!(n.buttons[0][0].text, "Complete");
    }

    #[test]
    fn test_notification_serialization() {
        let n = Notification::new("Test").with_body("body");
        let json = serde_json::to_string(&n).unwrap();
        let deserialized: Notification = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.title, "Test");
        assert_eq!(deserialized.body, Some("body".to_string()));
    }

    #[test]
    fn test_capabilities_default() {
        let caps = NotifyCapabilities::default();
        assert!(!caps.supports_attachments);
        assert!(caps.supports_markdown);
        assert!(caps.max_message_length.is_none());
    }
}
