//! What each connection may do on the broker.
//!
//! The handshake (mtw-transport `auth`) admits the owner and, when the
//! kernel names one, a user. The owner keeps full access: every channel,
//! store queries, kernel tools, WhatsApp. A user is held to:
//!
//! - subscribing to [`CHANGES_CHANNEL`] (which modules changed, no data) and
//!   to their own channel, `user:<id>`;
//! - publishing RPC requests on [`RPC_CHANNEL`], which the broker stamps
//!   with who sent them so the kernel runs them with that user's
//!   permissions and office scope;
//! - connect, ping, disconnect.
//!
//! Everything else from a user (requests, publishing elsewhere, answering
//! on a channel) is refused. Data itself reaches users over the kernel's
//! HTTP API, where permissions and scopes are applied.
//!
//! Every RPC request on [`RPC_CHANNEL`], the owner's included, gets
//! `metadata.principal` and `metadata.reply_to` overwritten by the broker,
//! so a client can never claim to be someone else. A response that carries
//! `metadata.reply_to` goes to that connection alone instead of the whole
//! channel.

use std::sync::LazyLock;

use dashmap::DashMap;
use mtw_protocol::{ConnMetadata, MsgType, MtwMessage};
use mtw_transport::ws::ROLE_USER;

/// Channel the kernel serves RPC on.
pub const RPC_CHANNEL: &str = "rpc";
/// Channel the kernel announces changed modules on (no data).
pub const CHANGES_CHANNEL: &str = "changes";
/// Features this broker reports (`broker.info`).
pub const FEATURES: &[&str] = &["principals"];

/// Who is behind a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnRole {
    /// The owner (super admin), or any connection when auth is disabled.
    Owner,
    /// A user, by id.
    User(String),
}

impl ConnRole {
    /// The role the handshake gave a connection.
    pub fn from_meta(meta: &ConnMetadata) -> Self {
        match &meta.auth {
            Some(auth) if auth.roles.iter().any(|r| r == ROLE_USER) => match &auth.user_id {
                Some(id) => ConnRole::User(id.clone()),
                // A user without an id cannot be held to their own channel;
                // give them nothing rather than everything.
                None => ConnRole::User(String::new()),
            },
            _ => ConnRole::Owner,
        }
    }

    fn principal(&self) -> serde_json::Value {
        match self {
            ConnRole::Owner => serde_json::json!({ "kind": "super_admin" }),
            ConnRole::User(id) => serde_json::json!({ "kind": "user", "id": id }),
        }
    }
}

/// A user's own channel.
pub fn user_channel(id: &str) -> String {
    format!("user:{id}")
}

static ROLES: LazyLock<DashMap<String, ConnRole>> = LazyLock::new(DashMap::new);

/// Remember a new connection's role.
pub fn register(conn_id: &str, meta: &ConnMetadata) {
    ROLES.insert(conn_id.to_string(), ConnRole::from_meta(meta));
}

/// Forget a closed connection.
pub fn forget(conn_id: &str) {
    ROLES.remove(conn_id);
}

/// A connection's role. One the server never saw connect is treated as a
/// user with no id: it may do nothing a user could not.
pub fn role_of(conn_id: &str) -> ConnRole {
    ROLES
        .get(conn_id)
        .map(|r| r.value().clone())
        .unwrap_or(ConnRole::User(String::new()))
}

/// Why a message is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub &'static str);

fn user_may_subscribe(id: &str, channel: &str) -> bool {
    channel == CHANGES_CHANNEL || (!id.is_empty() && channel == user_channel(id))
}

/// Check a message against its sender's role, and stamp an RPC request
/// with who sent it. Returns the message to handle, or why it is refused.
pub fn admit(role: &ConnRole, conn_id: &str, msg: MtwMessage) -> Result<MtwMessage, Refusal> {
    if let ConnRole::User(id) = role {
        match msg.msg_type {
            MsgType::Connect | MsgType::Ping | MsgType::Pong | MsgType::Disconnect => {}
            MsgType::Subscribe | MsgType::Unsubscribe => {
                let channel = msg.channel.as_deref().unwrap_or("");
                if !user_may_subscribe(id, channel) {
                    return Err(Refusal(
                        "a user may only subscribe to 'changes' and their own channel",
                    ));
                }
            }
            MsgType::Publish => {
                if msg.channel.as_deref() != Some(RPC_CHANNEL) || id.is_empty() {
                    return Err(Refusal("a user may only publish RPC requests"));
                }
            }
            _ => return Err(Refusal("not allowed for a user")),
        }
    }
    Ok(stamp(role, conn_id, msg))
}

/// Overwrite who sent an RPC request; leave every other message alone.
fn stamp(role: &ConnRole, conn_id: &str, mut msg: MtwMessage) -> MtwMessage {
    if matches!(msg.msg_type, MsgType::Publish | MsgType::Request)
        && msg.channel.as_deref() == Some(RPC_CHANNEL)
    {
        msg.metadata.insert("principal".into(), role.principal());
        msg.metadata.insert(
            "reply_to".into(),
            serde_json::Value::String(conn_id.to_string()),
        );
    }
    msg
}

/// Where a response goes: the connection named in `reply_to`, when the owner
/// (the kernel) sent it. `None` keeps the old behaviour (the whole channel).
pub fn reply_target(role: &ConnRole, msg: &MtwMessage) -> Option<String> {
    if *role != ConnRole::Owner {
        return None;
    }
    msg.metadata
        .get("reply_to")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_protocol::{AuthInfo, Payload};

    fn meta(user: Option<&str>) -> ConnMetadata {
        ConnMetadata {
            conn_id: "c".into(),
            remote_addr: None,
            user_agent: None,
            auth: Some(AuthInfo {
                user_id: user.map(str::to_string),
                token: None,
                roles: vec![if user.is_some() {
                    "user"
                } else {
                    "super_admin"
                }
                .into()],
                claims: Default::default(),
            }),
            connected_at: 0,
        }
    }

    fn msg(t: MsgType, channel: Option<&str>) -> MtwMessage {
        let mut m = MtwMessage::new(t, Payload::None);
        m.channel = channel.map(str::to_string);
        m
    }

    #[test]
    fn roles_come_from_the_handshake() {
        assert_eq!(ConnRole::from_meta(&meta(None)), ConnRole::Owner);
        assert_eq!(
            ConnRole::from_meta(&meta(Some("u1"))),
            ConnRole::User("u1".into())
        );
        let mut no_auth = meta(None);
        no_auth.auth = None;
        assert_eq!(ConnRole::from_meta(&no_auth), ConnRole::Owner);
    }

    #[test]
    fn a_user_subscribes_only_to_changes_and_their_own_channel() {
        let ana = ConnRole::User("u1".into());
        assert!(admit(&ana, "c", msg(MsgType::Subscribe, Some("changes"))).is_ok());
        assert!(admit(&ana, "c", msg(MsgType::Subscribe, Some("user:u1"))).is_ok());
        for ch in [
            "user:u2",
            "rpc",
            "dashboard",
            "agents",
            "whatsapp:inbound",
            "notifications",
        ] {
            assert!(
                admit(&ana, "c", msg(MsgType::Subscribe, Some(ch))).is_err(),
                "{ch}"
            );
        }
    }

    #[test]
    fn a_user_cannot_query_call_tools_or_answer() {
        let ana = ConnRole::User("u1".into());
        assert!(admit(&ana, "c", msg(MsgType::Request, None)).is_err());
        assert!(admit(&ana, "c", msg(MsgType::Publish, Some("dashboard"))).is_err());
        assert!(admit(&ana, "c", msg(MsgType::Response, Some("rpc"))).is_err());
        assert!(admit(&ana, "c", msg(MsgType::Error, Some("rpc"))).is_err());
        assert!(admit(&ana, "c", msg(MsgType::Ping, None)).is_ok());
    }

    #[test]
    fn a_connection_the_server_never_saw_gets_nothing() {
        let unknown = role_of("never-registered");
        assert!(admit(&unknown, "c", msg(MsgType::Publish, Some("rpc"))).is_err());
        assert!(admit(&unknown, "c", msg(MsgType::Request, None)).is_err());
    }

    #[test]
    fn rpc_requests_carry_who_sent_them_whatever_the_client_claims() {
        let ana = ConnRole::User("u1".into());
        let mut forged = msg(MsgType::Publish, Some("rpc"));
        forged.metadata.insert(
            "principal".into(),
            serde_json::json!({"kind": "super_admin"}),
        );
        forged
            .metadata
            .insert("reply_to".into(), "someone-else".into());
        let stamped = admit(&ana, "conn-ana", forged).unwrap();
        assert_eq!(
            stamped.metadata["principal"],
            serde_json::json!({"kind": "user", "id": "u1"})
        );
        assert_eq!(stamped.metadata["reply_to"], "conn-ana");

        let owner = admit(
            &ConnRole::Owner,
            "conn-o",
            msg(MsgType::Publish, Some("rpc")),
        )
        .unwrap();
        assert_eq!(
            owner.metadata["principal"],
            serde_json::json!({"kind": "super_admin"})
        );
        // Other channels are left as they are.
        let other = admit(
            &ConnRole::Owner,
            "conn-o",
            msg(MsgType::Publish, Some("dashboard")),
        )
        .unwrap();
        assert!(!other.metadata.contains_key("principal"));
    }

    #[test]
    fn only_the_owner_can_aim_a_response_at_one_connection() {
        let mut reply = msg(
            MtwMessage::response("x", Payload::None).msg_type,
            Some("rpc"),
        );
        reply.metadata.insert("reply_to".into(), "conn-ana".into());
        assert_eq!(
            reply_target(&ConnRole::Owner, &reply).as_deref(),
            Some("conn-ana")
        );
        assert_eq!(reply_target(&ConnRole::User("u1".into()), &reply), None);
        let plain = msg(MsgType::Response, Some("rpc"));
        assert_eq!(reply_target(&ConnRole::Owner, &plain), None);
    }

    #[test]
    fn registry_tracks_connections() {
        register("c-reg", &meta(Some("u9")));
        assert_eq!(role_of("c-reg"), ConnRole::User("u9".into()));
        forget("c-reg");
        assert_eq!(role_of("c-reg"), ConnRole::User(String::new()));
    }
}
