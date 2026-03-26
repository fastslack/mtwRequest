use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unique connection identifier
pub type ConnId = String;

/// Every message on the wire follows this format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtwMessage {
    /// Unique message ID (ULID)
    pub id: String,
    /// Message type
    #[serde(rename = "type")]
    pub msg_type: MsgType,
    /// Target channel/room (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Message payload
    pub payload: Payload,
    /// Arbitrary metadata
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Unix timestamp in milliseconds
    pub timestamp: u64,
    /// Reference to another message ID (for request/response correlation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
}

impl MtwMessage {
    pub fn new(msg_type: MsgType, payload: Payload) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            msg_type,
            channel: None,
            payload,
            metadata: HashMap::new(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            ref_id: None,
        }
    }

    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    pub fn with_ref(mut self, ref_id: impl Into<String>) -> Self {
        self.ref_id = Some(ref_id.into());
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Create an event message with text payload
    pub fn event(text: impl Into<String>) -> Self {
        Self::new(MsgType::Event, Payload::Text(text.into()))
    }

    /// Create a request message
    pub fn request(payload: Payload) -> Self {
        Self::new(MsgType::Request, payload)
    }

    /// Create a response to another message
    pub fn response(ref_id: impl Into<String>, payload: Payload) -> Self {
        Self::new(MsgType::Response, payload).with_ref(ref_id)
    }

    /// Create an error message
    pub fn error(code: u32, message: impl Into<String>) -> Self {
        Self::new(
            MsgType::Error,
            Payload::Json(serde_json::json!({
                "code": code,
                "message": message.into(),
            })),
        )
    }

    /// Create an agent task message
    pub fn agent_task(agent: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(MsgType::AgentTask, Payload::Text(content.into()))
            .with_metadata("agent", serde_json::Value::String(agent.into()))
    }

    /// Create a streaming chunk
    pub fn stream_chunk(ref_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(MsgType::Stream, Payload::Text(text.into())).with_ref(ref_id)
    }

    /// Create a stream end marker
    pub fn stream_end(ref_id: impl Into<String>) -> Self {
        Self::new(MsgType::StreamEnd, Payload::None).with_ref(ref_id)
    }
}

/// Message type enum
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgType {
    // Transport lifecycle
    Connect,
    Disconnect,
    Ping,
    Pong,

    // Data exchange
    Request,
    Response,
    Event,
    Stream,
    StreamEnd,

    // Channel operations
    Subscribe,
    Unsubscribe,
    Publish,

    // AI Agent
    AgentTask,
    AgentChunk,
    AgentToolCall,
    AgentToolResult,
    AgentComplete,

    // System
    Error,
    Ack,
}

/// Message payload variants
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum Payload {
    None,
    Text(String),
    Json(serde_json::Value),
    #[serde(with = "base64_bytes")]
    Binary(Vec<u8>),
}

impl Payload {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Payload::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_json(&self) -> Option<&serde_json::Value> {
        match self {
            Payload::Json(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_binary(&self) -> Option<&[u8]> {
        match self {
            Payload::Binary(b) => Some(b),
            _ => None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Payload::None)
    }
}

/// Serde helper for base64 encoding binary data in JSON
mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        use base64::Engine;
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

/// Connection metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnMetadata {
    pub conn_id: ConnId,
    pub remote_addr: Option<String>,
    pub user_agent: Option<String>,
    pub auth: Option<AuthInfo>,
    pub connected_at: u64,
}

/// Authentication info attached to a connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInfo {
    pub user_id: Option<String>,
    pub token: Option<String>,
    pub roles: Vec<String>,
    pub claims: HashMap<String, serde_json::Value>,
}

/// Disconnect reason
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DisconnectReason {
    Normal,
    Timeout,
    Error(String),
    Kicked(String),
    ServerShutdown,
}

/// Transport-level events
#[derive(Debug, Clone)]
pub enum TransportEvent {
    Connected(ConnId, ConnMetadata),
    Disconnected(ConnId, DisconnectReason),
    Message(ConnId, MtwMessage),
    Binary(ConnId, Vec<u8>),
    Error(ConnId, String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let msg = MtwMessage::event("hello world");
        assert_eq!(msg.msg_type, MsgType::Event);
        assert_eq!(msg.payload.as_text(), Some("hello world"));
        assert!(msg.channel.is_none());
        assert!(!msg.id.is_empty());
    }

    #[test]
    fn test_message_with_channel() {
        let msg = MtwMessage::event("test").with_channel("chat.general");
        assert_eq!(msg.channel, Some("chat.general".to_string()));
    }

    #[test]
    fn test_request_response() {
        let req = MtwMessage::request(Payload::Json(serde_json::json!({"action": "get_users"})));
        let res = MtwMessage::response(&req.id, Payload::Json(serde_json::json!({"users": []})));
        assert_eq!(res.ref_id, Some(req.id));
    }

    #[test]
    fn test_agent_task() {
        let msg = MtwMessage::agent_task("assistant", "explain this code");
        assert_eq!(msg.msg_type, MsgType::AgentTask);
        assert_eq!(msg.payload.as_text(), Some("explain this code"));
        assert_eq!(
            msg.metadata.get("agent"),
            Some(&serde_json::Value::String("assistant".to_string()))
        );
    }

    #[test]
    fn test_serialization_roundtrip() {
        let msg = MtwMessage::event("hello")
            .with_channel("chat.general")
            .with_metadata("key", serde_json::json!("value"));

        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: MtwMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, msg.id);
        assert_eq!(deserialized.msg_type, msg.msg_type);
        assert_eq!(deserialized.channel, msg.channel);
    }

    #[test]
    fn test_stream_messages() {
        let task = MtwMessage::agent_task("assistant", "hello");
        let chunk = MtwMessage::stream_chunk(&task.id, "partial ");
        let end = MtwMessage::stream_end(&task.id);

        assert_eq!(chunk.ref_id, Some(task.id.clone()));
        assert_eq!(end.ref_id, Some(task.id));
        assert_eq!(end.msg_type, MsgType::StreamEnd);
    }
}
