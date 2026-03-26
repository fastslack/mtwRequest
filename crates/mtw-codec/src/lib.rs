pub mod json;

use async_trait::async_trait;
use bytes::Bytes;
use mtw_core::MtwError;
use mtw_protocol::MtwMessage;

/// Codec trait — encode/decode MtwMessage to/from bytes
#[async_trait]
pub trait MtwCodec: Send + Sync {
    /// Codec name identifier
    fn name(&self) -> &str;

    /// Encode a message to bytes
    fn encode(&self, msg: &MtwMessage) -> Result<Bytes, MtwError>;

    /// Decode bytes into a message
    fn decode(&self, data: &[u8]) -> Result<MtwMessage, MtwError>;

    /// Content type (for HTTP transport)
    fn content_type(&self) -> &str;
}

/// Codec registry
pub struct CodecRegistry {
    codecs: std::collections::HashMap<String, Box<dyn MtwCodec>>,
    default: String,
}

impl CodecRegistry {
    pub fn new(default: &str) -> Self {
        let mut registry = Self {
            codecs: std::collections::HashMap::new(),
            default: default.to_string(),
        };
        // Register built-in JSON codec
        registry.register(Box::new(json::JsonCodec));
        registry
    }

    pub fn register(&mut self, codec: Box<dyn MtwCodec>) {
        self.codecs.insert(codec.name().to_string(), codec);
    }

    pub fn get(&self, name: &str) -> Option<&dyn MtwCodec> {
        self.codecs.get(name).map(|c| c.as_ref())
    }

    pub fn default_codec(&self) -> &dyn MtwCodec {
        self.codecs
            .get(&self.default)
            .map(|c| c.as_ref())
            .expect("default codec not found")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_registry() {
        let registry = CodecRegistry::new("json");
        assert!(registry.get("json").is_some());
        assert_eq!(registry.default_codec().name(), "json");
    }
}
