use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::types::{CommChannel, CommDirection, CommStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Communication {
    pub id: String,
    pub channel: CommChannel,
    pub direction: CommDirection,
    pub status: CommStatus,
    pub subject: String,
    pub body: String,
    pub body_html: String,
    pub contact_id: Option<String>,
    pub task_id: Option<String>,
    pub account_id: Option<String>,
    pub thread_id: String,
    pub in_reply_to: String,
    pub recipients_to: Vec<String>,
    pub recipients_cc: Vec<String>,
    pub recipients_bcc: Vec<String>,
    pub external_message_id: String,
    pub external_thread_id: String,
    pub scheduled_at: Option<String>,
    pub sent_at: Option<String>,
    pub error_message: String,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}

impl Communication {
    pub fn new(channel: CommChannel, direction: CommDirection) -> Self {
        let now = format!("{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
        Self {
            id: ulid::Ulid::new().to_string(), channel, direction,
            status: CommStatus::Draft, subject: String::new(), body: String::new(),
            body_html: String::new(), contact_id: None, task_id: None, account_id: None,
            thread_id: String::new(), in_reply_to: String::new(),
            recipients_to: Vec::new(), recipients_cc: Vec::new(), recipients_bcc: Vec::new(),
            external_message_id: String::new(), external_thread_id: String::new(),
            scheduled_at: None, sent_at: None, error_message: String::new(),
            metadata: HashMap::new(), created_at: now.clone(), updated_at: now,
        }
    }

    pub fn with_subject(mut self, s: impl Into<String>) -> Self { self.subject = s.into(); self }
    pub fn with_body(mut self, b: impl Into<String>) -> Self { self.body = b.into(); self }
    pub fn with_to(mut self, to: Vec<String>) -> Self { self.recipients_to = to; self }
    pub fn with_account(mut self, id: impl Into<String>) -> Self { self.account_id = Some(id.into()); self }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommAttachment {
    pub id: String, pub comm_id: String, pub filename: String,
    pub original_path: String, pub stored_path: String,
    pub mime_type: String, pub size_bytes: u64, pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_builder() {
        let c = Communication::new(CommChannel::Email, CommDirection::Outbound)
            .with_subject("Hi").with_body("World").with_to(vec!["a@b.com".into()]);
        assert_eq!(c.subject, "Hi");
        assert_eq!(c.recipients_to.len(), 1);
    }
}
