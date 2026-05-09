//! Elicitation: server-initiated input requests.
//!
//! When a tool needs more information mid-execution, instead of failing
//! with "missing field X" it can ask the *client* (via the user) for that
//! value. The flow:
//!
//! 1. Tool handler calls [`ElicitationBus::request`] with a JSON Schema
//!    describing what it wants.
//! 2. The bus pushes an `elicitation/create` request onto the server →
//!    client SSE stream and parks the future on a oneshot channel.
//! 3. The client renders a UI, gathers the user's response, and posts it
//!    back as a JSON-RPC response. The transport hands it to the bus,
//!    which fulfils the oneshot.
//! 4. The tool handler resumes with the user-provided value.
//!
//! ## Capability gate
//! Only used when the negotiated session declared `elicitation` as a
//! client capability — see [`ClientCapabilities::supports_elicitation`].
//! Without it, `request` returns immediately with an error so handlers
//! fall back to their normal "missing field" path.
//!
//! ## Today
//! The bus is wired into the McpServer state. The HTTP SSE channel ships
//! the `elicitation/create` notifications; reverse responses are accepted
//! over the standard POST endpoint, indexed by `request_id`. Stdio
//! transport doesn't support full duplex, so elicitation calls return an
//! error there — the client should fall back to interactive prompting in
//! that mode.

use crate::protocol::NegotiatedSession;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};

/// One pending elicitation. Resolved when the client posts the user's
/// response keyed by `request_id`.
type Pending = oneshot::Sender<Result<Value, String>>;

#[derive(Clone)]
pub struct ElicitationBus {
    pending: Arc<DashMap<String, Pending>>,
    /// Outbound queue. The HTTP transport drains this onto its SSE stream.
    /// In stdio mode it stays empty — and `request` short-circuits to an
    /// error before ever queueing.
    pub outbound: Arc<RwLock<Vec<ElicitationRequest>>>,
    session: Arc<RwLock<NegotiatedSession>>,
    transport_supports_full_duplex: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationRequest {
    pub request_id: String,
    pub message: String,
    pub schema: Value,
}

impl ElicitationBus {
    pub fn new(
        session: Arc<RwLock<NegotiatedSession>>,
        transport_supports_full_duplex: bool,
    ) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            outbound: Arc::new(RwLock::new(Vec::new())),
            session,
            transport_supports_full_duplex,
        }
    }

    /// Ask the user for input. Blocks the calling tool until the client
    /// answers (or returns immediately with `Err` when elicitation isn't
    /// possible: no client capability, or stdio transport).
    pub async fn request(&self, message: &str, schema: Value) -> Result<Value, String> {
        if !self.transport_supports_full_duplex {
            return Err(
                "elicitation requires a full-duplex transport (HTTP); \
                 stdio cannot deliver server→client requests"
                    .to_string(),
            );
        }
        if !self.session.read().await.client_caps.supports_elicitation() {
            return Err("client did not declare elicitation capability".to_string());
        }

        let request_id = ulid::Ulid::new().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.insert(request_id.clone(), tx);

        self.outbound.write().await.push(ElicitationRequest {
            request_id: request_id.clone(),
            message: message.to_string(),
            schema,
        });

        rx.await.unwrap_or_else(|_| Err("elicitation cancelled".into()))
    }

    /// Called by the HTTP transport when it receives a client response
    /// to an elicitation/create request.
    pub fn fulfill(&self, request_id: &str, value: Result<Value, String>) -> bool {
        if let Some((_, tx)) = self.pending.remove(request_id) {
            let _ = tx.send(value);
            true
        } else {
            false
        }
    }
}

/// Helper: build an `elicitation/create` JSON-RPC request frame to send
/// down the SSE channel. The HTTP transport calls this for every entry it
/// drains from `ElicitationBus::outbound`.
pub fn build_outbound_frame(req: &ElicitationRequest) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": req.request_id,
        "method": "elicitation/create",
        "params": {
            "message": req.message,
            "requestedSchema": req.schema,
        }
    })
}
