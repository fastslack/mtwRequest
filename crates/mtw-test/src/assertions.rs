//! Test assertion helpers for mtwRequest messages.

use mtw_protocol::{MsgType, MtwMessage};

/// Assert that a message has the expected message type.
///
/// # Panics
/// Panics if the message type does not match.
#[macro_export]
macro_rules! assert_message_type {
    ($msg:expr, $expected:expr) => {
        assert_eq!(
            $msg.msg_type, $expected,
            "expected message type {:?}, got {:?}",
            $expected, $msg.msg_type
        );
    };
}

/// Assert that a message has a text payload matching the expected value.
///
/// # Panics
/// Panics if the payload is not text or does not match.
#[macro_export]
macro_rules! assert_payload_text {
    ($msg:expr, $expected:expr) => {
        match &$msg.payload {
            mtw_protocol::Payload::Text(text) => {
                assert_eq!(
                    text, $expected,
                    "expected payload text {:?}, got {:?}",
                    $expected, text
                );
            }
            other => {
                panic!(
                    "expected Text payload with {:?}, got {:?}",
                    $expected, other
                );
            }
        }
    };
}

/// Assert that a message is on the expected channel.
///
/// # Panics
/// Panics if the channel does not match.
#[macro_export]
macro_rules! assert_channel {
    ($msg:expr, $expected:expr) => {
        assert_eq!(
            $msg.channel.as_deref(),
            Some($expected),
            "expected channel {:?}, got {:?}",
            $expected,
            $msg.channel
        );
    };
}

/// Assert that a message has a metadata entry with the given key.
///
/// # Panics
/// Panics if the metadata key is not present.
#[macro_export]
macro_rules! assert_has_metadata {
    ($msg:expr, $key:expr) => {
        assert!(
            $msg.metadata.contains_key($key),
            "expected metadata key {:?}, but it was not found. Keys: {:?}",
            $key,
            $msg.metadata.keys().collect::<Vec<_>>()
        );
    };
}

#[cfg(test)]
mod tests {
    use mtw_protocol::{MsgType, MtwMessage, Payload};

    #[test]
    fn test_assert_message_type() {
        let msg = MtwMessage::event("hello");
        assert_message_type!(msg, MsgType::Event);
    }

    #[test]
    fn test_assert_payload_text() {
        let msg = MtwMessage::event("hello world");
        assert_payload_text!(msg, "hello world");
    }

    #[test]
    fn test_assert_channel() {
        let msg = MtwMessage::event("test").with_channel("chat.general");
        assert_channel!(msg, "chat.general");
    }

    #[test]
    fn test_assert_has_metadata() {
        let msg = MtwMessage::event("test")
            .with_metadata("key", serde_json::json!("value"));
        assert_has_metadata!(msg, "key");
    }

    #[test]
    #[should_panic(expected = "expected message type")]
    fn test_assert_message_type_fails() {
        let msg = MtwMessage::event("hello");
        assert_message_type!(msg, MsgType::Request);
    }

    #[test]
    #[should_panic(expected = "expected payload text")]
    fn test_assert_payload_text_fails() {
        let msg = MtwMessage::event("hello");
        assert_payload_text!(msg, "wrong");
    }

    #[test]
    #[should_panic(expected = "expected channel")]
    fn test_assert_channel_fails() {
        let msg = MtwMessage::event("test").with_channel("wrong");
        assert_channel!(msg, "right");
    }

    #[test]
    #[should_panic(expected = "expected metadata key")]
    fn test_assert_has_metadata_fails() {
        let msg = MtwMessage::event("test");
        assert_has_metadata!(msg, "missing_key");
    }
}
