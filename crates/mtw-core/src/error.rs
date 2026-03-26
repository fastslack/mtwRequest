use thiserror::Error;

#[derive(Debug, Error)]
pub enum MtwError {
    #[error("module error: {module}: {message}")]
    Module { module: String, message: String },

    #[error("config error: {0}")]
    Config(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("codec error: {0}")]
    Codec(String),

    #[error("router error: {0}")]
    Router(String),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("agent error: {0}")]
    Agent(String),

    #[error("connection not found: {0}")]
    ConnectionNotFound(String),

    #[error("channel not found: {0}")]
    ChannelNotFound(String),

    #[error("protocol error: {0}")]
    Protocol(#[from] mtw_protocol::ProtocolError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl MtwError {
    pub fn module(module: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Module {
            module: module.into(),
            message: message.into(),
        }
    }
}
