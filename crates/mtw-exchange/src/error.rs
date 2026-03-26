use thiserror::Error;

/// Exchange-specific errors
#[derive(Debug, Error)]
pub enum ExchangeError {
    #[error("exchange {exchange}: HTTP error: {message}")]
    Http { exchange: String, message: String },

    #[error("exchange {exchange}: WebSocket error: {message}")]
    WebSocket { exchange: String, message: String },

    #[error("exchange {exchange}: authentication failed: {message}")]
    Auth { exchange: String, message: String },

    #[error("exchange {exchange}: rate limited, retry after {retry_after_ms}ms")]
    RateLimited {
        exchange: String,
        retry_after_ms: u64,
    },

    #[error("exchange {exchange}: invalid symbol: {symbol}")]
    InvalidSymbol { exchange: String, symbol: String },

    #[error("exchange {exchange}: {message}")]
    Api {
        exchange: String,
        code: Option<i32>,
        message: String,
    },

    #[error("exchange not found: {0}")]
    NotFound(String),

    #[error("exchange {exchange}: deserialization error: {message}")]
    Deserialization { exchange: String, message: String },

    #[error("provider not configured: {0}")]
    NotConfigured(String),

    #[error("{0}")]
    Internal(String),
}

impl From<ExchangeError> for mtw_core::MtwError {
    fn from(e: ExchangeError) -> Self {
        mtw_core::MtwError::Internal(e.to_string())
    }
}
