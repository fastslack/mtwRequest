use async_trait::async_trait;
use mtw_core::MtwError;
use mtw_protocol::MtwMessage;
use mtw_router::middleware::{MiddlewareAction, MiddlewareContext, MtwMiddleware};
use std::sync::Arc;

use crate::MtwAuth;

/// Authentication middleware that validates tokens on inbound messages.
///
/// Extracts the auth token from message metadata and validates it
/// before allowing the message through the pipeline.
pub struct AuthMiddleware {
    auth: Arc<dyn MtwAuth>,
    /// Metadata key to look for the auth token
    token_key: String,
    /// Message types that bypass authentication
    bypass_types: Vec<mtw_protocol::MsgType>,
}

impl AuthMiddleware {
    pub fn new(auth: Arc<dyn MtwAuth>) -> Self {
        Self {
            auth,
            token_key: "auth_token".to_string(),
            bypass_types: vec![
                mtw_protocol::MsgType::Connect,
                mtw_protocol::MsgType::Ping,
                mtw_protocol::MsgType::Pong,
            ],
        }
    }

    /// Set the metadata key used to extract the auth token
    pub fn with_token_key(mut self, key: impl Into<String>) -> Self {
        self.token_key = key.into();
        self
    }

    /// Add a message type that should bypass authentication
    pub fn with_bypass(mut self, msg_type: mtw_protocol::MsgType) -> Self {
        self.bypass_types.push(msg_type);
        self
    }
}

#[async_trait]
impl MtwMiddleware for AuthMiddleware {
    fn name(&self) -> &str {
        "auth"
    }

    fn priority(&self) -> i32 {
        // Auth should run very early in the middleware chain
        10
    }

    async fn on_inbound(
        &self,
        msg: MtwMessage,
        _ctx: &MiddlewareContext,
    ) -> Result<MiddlewareAction, MtwError> {
        // Bypass authentication for certain message types
        if self.bypass_types.contains(&msg.msg_type) {
            return Ok(MiddlewareAction::Continue(msg));
        }

        // Extract token from metadata
        let token = msg
            .metadata
            .get(&self.token_key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| MtwError::Auth("missing auth token in message metadata".into()))?;

        // Validate the token
        let claims = self.auth.validate(token).await?;

        // Attach validated claims to the message metadata
        let mut msg = msg;
        msg.metadata.insert(
            "auth_claims".to_string(),
            serde_json::to_value(&claims)
                .map_err(|e| MtwError::Internal(format!("failed to serialize claims: {}", e)))?,
        );
        msg.metadata.insert(
            "auth_user".to_string(),
            serde_json::Value::String(claims.sub),
        );

        Ok(MiddlewareAction::Continue(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwt::{JwtAuth, JwtConfig};
    use mtw_protocol::{MsgType, Payload};
    use std::collections::HashMap;

    fn make_jwt_auth() -> Arc<dyn MtwAuth> {
        Arc::new(JwtAuth::new(JwtConfig::new("test-secret")))
    }

    fn make_ctx() -> MiddlewareContext {
        MiddlewareContext {
            conn_id: "test-conn".to_string(),
            channel: None,
        }
    }

    #[tokio::test]
    async fn test_bypass_connect() {
        let mw = AuthMiddleware::new(make_jwt_auth());
        let msg = MtwMessage::new(MsgType::Connect, Payload::None);
        let ctx = make_ctx();

        let result = mw.on_inbound(msg, &ctx).await.unwrap();
        match result {
            MiddlewareAction::Continue(_) => {} // expected
            _ => panic!("expected Continue"),
        }
    }

    #[tokio::test]
    async fn test_bypass_ping() {
        let mw = AuthMiddleware::new(make_jwt_auth());
        let msg = MtwMessage::new(MsgType::Ping, Payload::None);
        let ctx = make_ctx();

        let result = mw.on_inbound(msg, &ctx).await.unwrap();
        match result {
            MiddlewareAction::Continue(_) => {}
            _ => panic!("expected Continue"),
        }
    }

    #[tokio::test]
    async fn test_missing_token() {
        let mw = AuthMiddleware::new(make_jwt_auth());
        let msg = MtwMessage::event("hello");
        let ctx = make_ctx();

        let result = mw.on_inbound(msg, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_token() {
        let mw = AuthMiddleware::new(make_jwt_auth());
        let msg = MtwMessage::event("hello").with_metadata(
            "auth_token",
            serde_json::Value::String("invalid-token".into()),
        );
        let ctx = make_ctx();

        let result = mw.on_inbound(msg, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_valid_token() {
        let jwt = JwtAuth::new(JwtConfig::new("test-secret"));
        let token = jwt
            .create_token("user-123", vec!["admin".to_string()], HashMap::new())
            .unwrap();

        let mw = AuthMiddleware::new(Arc::new(JwtAuth::new(JwtConfig::new("test-secret"))));
        let msg = MtwMessage::event("hello").with_metadata(
            "auth_token",
            serde_json::Value::String(token.token),
        );
        let ctx = make_ctx();

        let result = mw.on_inbound(msg, &ctx).await.unwrap();
        match result {
            MiddlewareAction::Continue(msg) => {
                assert!(msg.metadata.contains_key("auth_claims"));
                assert_eq!(
                    msg.metadata.get("auth_user"),
                    Some(&serde_json::Value::String("user-123".into()))
                );
            }
            _ => panic!("expected Continue with claims"),
        }
    }

    #[tokio::test]
    async fn test_custom_token_key() {
        let jwt = JwtAuth::new(JwtConfig::new("test-secret"));
        let token = jwt
            .create_token("user-1", vec![], HashMap::new())
            .unwrap();

        let mw = AuthMiddleware::new(Arc::new(JwtAuth::new(JwtConfig::new("test-secret"))))
            .with_token_key("x-api-token");

        let msg = MtwMessage::event("hello").with_metadata(
            "x-api-token",
            serde_json::Value::String(token.token),
        );
        let ctx = make_ctx();

        let result = mw.on_inbound(msg, &ctx).await.unwrap();
        match result {
            MiddlewareAction::Continue(_) => {}
            _ => panic!("expected Continue"),
        }
    }
}
