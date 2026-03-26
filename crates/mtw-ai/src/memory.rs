use crate::provider::Message;
use serde::{Deserialize, Serialize};

/// Agent memory -- stores conversation history and context
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentMemory {
    /// Conversation history
    messages: Vec<Message>,
    /// Maximum messages to retain before summarization
    max_messages: usize,
}

impl AgentMemory {
    pub fn new(max_messages: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_messages,
        }
    }

    /// Add a message to the conversation history
    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Get the full conversation context
    pub fn get_context(&self) -> &[Message] {
        &self.messages
    }

    /// Get the last N messages
    pub fn get_recent(&self, n: usize) -> &[Message] {
        let start = self.messages.len().saturating_sub(n);
        &self.messages[start..]
    }

    /// Clear all conversation history
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Number of messages in memory
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Check if memory is empty
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Summarize the context when it exceeds max_messages.
    /// Keeps the system message (if any) and the most recent messages,
    /// replacing older messages with a summary message.
    pub fn summarize_context(&mut self) {
        if self.messages.len() <= self.max_messages {
            return;
        }

        // Keep system message if first message is system
        let has_system = self
            .messages
            .first()
            .map(|m| m.role == crate::provider::MessageRole::System)
            .unwrap_or(false);

        let keep_count = self.max_messages / 2;

        if has_system && self.messages.len() > keep_count + 1 {
            let system_msg = self.messages[0].clone();
            let recent_start = self.messages.len().saturating_sub(keep_count);
            let recent: Vec<Message> = self.messages[recent_start..].to_vec();

            let summary_text = format!(
                "[Summary: {} earlier messages have been condensed]",
                self.messages.len() - 1 - recent.len()
            );

            self.messages = Vec::with_capacity(2 + recent.len());
            self.messages.push(system_msg);
            self.messages.push(Message::system(summary_text));
            self.messages.extend(recent);
        } else {
            let recent_start = self.messages.len().saturating_sub(keep_count);
            let recent: Vec<Message> = self.messages[recent_start..].to_vec();

            let summary_text = format!(
                "[Summary: {} earlier messages have been condensed]",
                self.messages.len() - recent.len()
            );

            self.messages = Vec::with_capacity(1 + recent.len());
            self.messages.push(Message::system(summary_text));
            self.messages.extend(recent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MessageRole;

    #[test]
    fn test_memory_basic() {
        let mut mem = AgentMemory::new(100);
        assert!(mem.is_empty());

        mem.add_message(Message::user("Hello"));
        mem.add_message(Message::assistant("Hi there"));

        assert_eq!(mem.len(), 2);
        assert!(!mem.is_empty());
    }

    #[test]
    fn test_get_context() {
        let mut mem = AgentMemory::new(100);
        mem.add_message(Message::system("You are helpful"));
        mem.add_message(Message::user("Hello"));
        mem.add_message(Message::assistant("Hi"));

        let ctx = mem.get_context();
        assert_eq!(ctx.len(), 3);
        assert_eq!(ctx[0].role, MessageRole::System);
    }

    #[test]
    fn test_get_recent() {
        let mut mem = AgentMemory::new(100);
        for i in 0..10 {
            mem.add_message(Message::user(format!("msg {}", i)));
        }

        let recent = mem.get_recent(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].content, "msg 7");
        assert_eq!(recent[2].content, "msg 9");
    }

    #[test]
    fn test_clear() {
        let mut mem = AgentMemory::new(100);
        mem.add_message(Message::user("Hello"));
        mem.clear();
        assert!(mem.is_empty());
    }

    #[test]
    fn test_summarize_context() {
        let mut mem = AgentMemory::new(6);
        mem.add_message(Message::system("You are helpful"));
        for i in 0..10 {
            mem.add_message(Message::user(format!("message {}", i)));
        }

        assert_eq!(mem.len(), 11);
        mem.summarize_context();

        // After summarization: system + summary + last 3 messages
        assert!(mem.len() < 11);
        // First message should be the system message
        assert_eq!(mem.messages[0].role, MessageRole::System);
        assert_eq!(mem.messages[0].content, "You are helpful");
        // Second should be the summary
        assert!(mem.messages[1].content.contains("Summary"));
    }

    #[test]
    fn test_summarize_no_op_when_under_limit() {
        let mut mem = AgentMemory::new(100);
        mem.add_message(Message::user("Hello"));
        mem.add_message(Message::assistant("Hi"));

        mem.summarize_context();
        assert_eq!(mem.len(), 2);
    }

    #[test]
    fn test_summarize_without_system_message() {
        let mut mem = AgentMemory::new(4);
        for i in 0..8 {
            mem.add_message(Message::user(format!("msg {}", i)));
        }

        mem.summarize_context();
        // Should have summary + last 2 messages
        assert!(mem.len() < 8);
        assert_eq!(mem.messages[0].role, MessageRole::System);
        assert!(mem.messages[0].content.contains("Summary"));
    }
}
