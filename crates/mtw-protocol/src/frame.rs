use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::ProtocolError;
use crate::message::MtwMessage;

/// Current protocol version
pub const PROTOCOL_VERSION: u8 = 1;

/// Magic bytes to identify mtwRequest binary frames
const MAGIC: [u8; 3] = [b'M', b'T', b'W'];

/// Maximum frame size (10MB)
pub const MAX_FRAME_SIZE: usize = 10 * 1024 * 1024;

/// Frame type identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// JSON-encoded MtwMessage
    Json = 0x01,
    /// Raw binary data (for 3D, audio, etc.)
    Binary = 0x02,
    /// Ping frame
    Ping = 0x03,
    /// Pong frame
    Pong = 0x04,
}

impl TryFrom<u8> for FrameType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(FrameType::Json),
            0x02 => Ok(FrameType::Binary),
            0x03 => Ok(FrameType::Ping),
            0x04 => Ok(FrameType::Pong),
            _ => Err(ProtocolError::InvalidFormat(format!(
                "unknown frame type: 0x{:02x}",
                value
            ))),
        }
    }
}

/// Wire frame format:
///
/// ```text
/// ┌───────┬─────────┬──────────┬────────────┬─────────┐
/// │ MAGIC │ VERSION │ FRAME_TYPE│ PAYLOAD_LEN│ PAYLOAD │
/// │ 3B    │ 1B      │ 1B       │ 4B (BE)    │ N bytes │
/// └───────┴─────────┴──────────┴────────────┴─────────┘
/// ```
///
/// Total header: 9 bytes
pub struct Frame;

impl Frame {
    /// Encode an MtwMessage into a binary frame
    pub fn encode_message(msg: &MtwMessage) -> Result<Bytes, ProtocolError> {
        let json = serde_json::to_vec(msg)?;
        Ok(Self::encode_raw(FrameType::Json, &json)?)
    }

    /// Encode raw binary data into a frame
    pub fn encode_binary(data: &[u8]) -> Result<Bytes, ProtocolError> {
        Self::encode_raw(FrameType::Binary, data)
    }

    /// Encode a ping frame
    pub fn encode_ping() -> Bytes {
        Self::encode_raw(FrameType::Ping, &[]).unwrap()
    }

    /// Encode a pong frame
    pub fn encode_pong() -> Bytes {
        Self::encode_raw(FrameType::Pong, &[]).unwrap()
    }

    fn encode_raw(frame_type: FrameType, payload: &[u8]) -> Result<Bytes, ProtocolError> {
        if payload.len() > MAX_FRAME_SIZE {
            return Err(ProtocolError::PayloadTooLarge {
                size: payload.len(),
                max: MAX_FRAME_SIZE,
            });
        }

        let mut buf = BytesMut::with_capacity(9 + payload.len());
        buf.put_slice(&MAGIC);
        buf.put_u8(PROTOCOL_VERSION);
        buf.put_u8(frame_type as u8);
        buf.put_u32(payload.len() as u32);
        buf.put_slice(payload);

        Ok(buf.freeze())
    }

    /// Decode a frame from bytes. Returns (frame_type, payload).
    pub fn decode(mut data: Bytes) -> Result<(FrameType, Bytes), ProtocolError> {
        if data.len() < 9 {
            return Err(ProtocolError::InvalidFormat(
                "frame too short (need at least 9 bytes)".into(),
            ));
        }

        // Check magic
        if &data[..3] != &MAGIC {
            return Err(ProtocolError::InvalidFormat("invalid magic bytes".into()));
        }
        data.advance(3);

        // Check version
        let version = data.get_u8();
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(version));
        }

        // Frame type
        let frame_type = FrameType::try_from(data.get_u8())?;

        // Payload length
        let payload_len = data.get_u32() as usize;
        if payload_len > MAX_FRAME_SIZE {
            return Err(ProtocolError::PayloadTooLarge {
                size: payload_len,
                max: MAX_FRAME_SIZE,
            });
        }

        if data.remaining() < payload_len {
            return Err(ProtocolError::InvalidFormat(format!(
                "expected {} bytes of payload, got {}",
                payload_len,
                data.remaining()
            )));
        }

        let payload = data.slice(..payload_len);
        Ok((frame_type, payload))
    }

    /// Decode a frame and parse it as an MtwMessage
    pub fn decode_message(data: Bytes) -> Result<MtwMessage, ProtocolError> {
        let (frame_type, payload) = Self::decode(data)?;
        match frame_type {
            FrameType::Json => {
                let msg: MtwMessage = serde_json::from_slice(&payload)?;
                Ok(msg)
            }
            _ => Err(ProtocolError::InvalidFormat(format!(
                "expected JSON frame, got {:?}",
                frame_type
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{MsgType, Payload};

    #[test]
    fn test_encode_decode_message() {
        let msg = MtwMessage::new(MsgType::Event, Payload::Text("hello".into()));
        let encoded = Frame::encode_message(&msg).unwrap();
        let decoded = Frame::decode_message(encoded).unwrap();

        assert_eq!(decoded.id, msg.id);
        assert_eq!(decoded.msg_type, MsgType::Event);
        assert_eq!(decoded.payload.as_text(), Some("hello"));
    }

    #[test]
    fn test_encode_decode_binary() {
        let data = vec![0x00, 0x01, 0x02, 0xFF];
        let encoded = Frame::encode_binary(&data).unwrap();
        let (frame_type, payload) = Frame::decode(encoded).unwrap();

        assert_eq!(frame_type, FrameType::Binary);
        assert_eq!(&payload[..], &data[..]);
    }

    #[test]
    fn test_ping_pong() {
        let ping = Frame::encode_ping();
        let (ft, payload) = Frame::decode(ping).unwrap();
        assert_eq!(ft, FrameType::Ping);
        assert!(payload.is_empty());

        let pong = Frame::encode_pong();
        let (ft, _) = Frame::decode(pong).unwrap();
        assert_eq!(ft, FrameType::Pong);
    }

    #[test]
    fn test_payload_too_large() {
        let data = vec![0u8; MAX_FRAME_SIZE + 1];
        let result = Frame::encode_binary(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_magic() {
        let data = Bytes::from_static(&[0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00]);
        let result = Frame::decode(data);
        assert!(result.is_err());
    }
}
