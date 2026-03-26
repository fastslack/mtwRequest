use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid message format: {0}")]
    InvalidFormat(String),

    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u8),

    #[error("payload too large: {size} bytes (max: {max})")]
    PayloadTooLarge { size: usize, max: usize },

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("invalid message type: {0}")]
    InvalidMsgType(String),
}
