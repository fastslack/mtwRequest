use async_trait::async_trait;
use bytes::Bytes;
use mtw_core::MtwError;
use mtw_protocol::MtwMessage;

use crate::MtwCodec;

/// JSON codec — the default codec for mtwRequest
pub struct JsonCodec;

#[async_trait]
impl MtwCodec for JsonCodec {
    fn name(&self) -> &str {
        "json"
    }

    fn encode(&self, msg: &MtwMessage) -> Result<Bytes, MtwError> {
        let json = serde_json::to_vec(msg).map_err(|e| MtwError::Codec(e.to_string()))?;
        Ok(Bytes::from(json))
    }

    fn decode(&self, data: &[u8]) -> Result<MtwMessage, MtwError> {
        serde_json::from_slice(data).map_err(|e| MtwError::Codec(e.to_string()))
    }

    fn content_type(&self) -> &str {
        "application/json"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_protocol::{MsgType, Payload};

    #[test]
    fn test_json_encode_decode() {
        let codec = JsonCodec;
        let msg = MtwMessage::new(MsgType::Event, Payload::Text("hello".into()));

        let encoded = codec.encode(&msg).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded.id, msg.id);
        assert_eq!(decoded.msg_type, MsgType::Event);
        assert_eq!(decoded.payload.as_text(), Some("hello"));
    }

    #[test]
    fn test_json_codec_metadata() {
        let codec = JsonCodec;
        assert_eq!(codec.name(), "json");
        assert_eq!(codec.content_type(), "application/json");
    }

    #[test]
    fn test_json_decode_invalid() {
        let codec = JsonCodec;
        let result = codec.decode(b"not json");
        assert!(result.is_err());
    }
}
