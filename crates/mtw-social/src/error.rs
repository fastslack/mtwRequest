use thiserror::Error;

#[derive(Debug, Error)]
pub enum SocialError {
    #[error("identity error: {0}")]
    Identity(#[from] mtw_identity::IdentityError),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("message payload is not JSON")]
    InvalidPayload,

    #[error("message has no signature — refusing to ingest")]
    Unsigned,

    #[error("message signature is invalid")]
    BadSignature,

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("event has no sender pubkey")]
    NoSender,
}

pub type Result<T> = std::result::Result<T, SocialError>;
