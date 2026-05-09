//! MCP tools for the attestation primitive.
//!
//! Two tools live here:
//!
//!   * `mtw_attest_identity` — return the running server's `server_id`
//!     (Ed25519 fingerprint). Clients use this to learn whose receipts
//!     they should trust before they start verifying.
//!   * `mtw_attest_verify`   — verify any receipt the client hands back.
//!     Accepts the JSON shape that `_meta.mtw.attestation` ships in
//!     every `tools/call` response.
//!
//! These are deliberately small. The whole point of attestation is that
//! the client has the public key, downloads the receipts, and verifies
//! offline — but having a "verify this for me" tool is invaluable for
//! debugging, end-to-end demos, and clients that don't want to pull in
//! the crypto themselves.

use crate::protocol::{McpServer, ToolHandler, ToolResult};
use serde_json::{json, Value};
use std::sync::Arc;

pub fn register(server: &mut McpServer) {
    register_identity(server);
    register_verify(server);
}

fn register_identity(server: &mut McpServer) {
    let identity = server.identity().cloned();
    server.tool_full(
        "mtw_attest_identity",
        "Return this server's Ed25519 attestation identity (server_id) and \
         whether attestation is enabled. Clients pin this fingerprint and \
         then verify every receipt against it.",
        json!({ "type": "object", "properties": {}, "required": [] }),
        Some(json!({
            "type": "object",
            "properties": {
                "enabled":   { "type": "boolean" },
                "server_id": { "type": "string", "description": "ed25519:<hex32>, present when enabled." }
            },
            "required": ["enabled"]
        })),
        vec!["meta".into(), "attestation".into(), "identity".into()],
        handler(move |_args| {
            let identity = identity.clone();
            async move {
                let out = match identity {
                    Some(id) => json!({ "enabled": true, "server_id": id.server_id() }),
                    None => json!({ "enabled": false }),
                };
                Ok(ToolResult::structured(out.to_string(), out))
            }
        }),
    );
}

fn register_verify(server: &mut McpServer) {
    server.tool_full(
        "mtw_attest_verify",
        "Verify a previously-issued attestation receipt. Returns whether \
         the signature is valid against its embedded `server_id` plus the \
         receipt's payload metadata. Pure verification — does not check \
         whether you trust the signing identity.",
        json!({
            "type": "object",
            "properties": {
                "receipt": {
                    "type": "object",
                    "description": "The full receipt object as it appeared in `_meta.mtw.attestation`."
                }
            },
            "required": ["receipt"]
        }),
        Some(json!({
            "type": "object",
            "properties": {
                "valid":      { "type": "boolean" },
                "tool":       { "type": "string" },
                "server_id":  { "type": "string" },
                "ts_ms":      { "type": "integer" },
                "side_effects": { "type": "array", "items": { "type": "string" } },
                "reason":     { "type": "string", "description": "Set when valid=false." }
            },
            "required": ["valid"]
        })),
        vec!["meta".into(), "attestation".into(), "verify".into()],
        handler(|args| async move {
            let receipt_value = args
                .get("receipt")
                .cloned()
                .ok_or("missing 'receipt'")?;
            let receipt: mtw_attest::Receipt = serde_json::from_value(receipt_value)
                .map_err(|e| format!("malformed receipt: {}", e))?;

            match receipt.verify() {
                Ok(()) => {
                    let out = json!({
                        "valid": true,
                        "tool": receipt.tool,
                        "server_id": receipt.server_id,
                        "ts_ms": receipt.ts_ms,
                        "side_effects": receipt.side_effects,
                    });
                    Ok(ToolResult::structured(out.to_string(), out))
                }
                Err(e) => {
                    let out = json!({
                        "valid": false,
                        "tool": receipt.tool,
                        "server_id": receipt.server_id,
                        "ts_ms": receipt.ts_ms,
                        "side_effects": receipt.side_effects,
                        "reason": e.to_string(),
                    });
                    Ok(ToolResult::structured(out.to_string(), out))
                }
            }
        }),
    );
}

fn handler<F, Fut>(f: F) -> ToolHandler
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<ToolResult, String>> + Send + 'static,
{
    Arc::new(move |args| {
        let fut = f(args);
        Box::pin(fut)
    })
}
