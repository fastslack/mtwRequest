//! Transport layer for the MCP server.
//!
//! Two implementations live here:
//!
//! * [`serve_stdio`] — line-delimited JSON-RPC over `stdin`/`stdout`. The
//!   default. Used by Claude Code, Cursor, and every CLI client.
//! * [`serve_http`] — streamable HTTP. POST a JSON-RPC request to `/mcp`,
//!   get the response back. SSE `GET /mcp` exposes a server→client event
//!   channel for future async notifications. This is the deployment story
//!   for hyperscalers (the "stateless transport" the protocol team is
//!   converging on; we ship the streamable-HTTP shape today and can flip
//!   the wire format later without touching `protocol.rs`).
//!
//! The `protocol::McpServer` itself is transport-agnostic — both functions
//! just feed bytes in and write bytes out.

use crate::protocol::{JsonRpcRequest, JsonRpcResponse, McpServer};
use std::io::{self, BufRead, Write};
use std::sync::Arc;

/// Line-delimited JSON-RPC over stdio. Blocks the current task until stdin
/// closes. Logs go to stderr (stdout is reserved for protocol output).
pub async fn serve_stdio(server: Arc<McpServer>) {
    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("invalid JSON-RPC: {}", e);
                continue;
            }
        };

        if let Some(resp) = server.handle_request(req).await {
            write_response(&stdout, &resp);
        }
    }
}

fn write_response(stdout: &io::Stdout, resp: &JsonRpcResponse) {
    let mut out = stdout.lock();
    let _ = serde_json::to_writer(&mut out, resp);
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

// ── HTTP transport ──────────────────────────────────────────────────────────

/// Spec-light streamable HTTP transport.
///
/// Endpoints:
/// * `POST /mcp` — body is one JSON-RPC request, response body is one
///   JSON-RPC response (or empty 204 for notifications).
/// * `GET /mcp` — SSE stream for server→client events (notifications and,
///   eventually, elicitation requests).
/// * `GET /healthz` — liveness probe.
///
/// We deliberately keep the framing tiny: no session cookies, no upgrade
/// dance. Each POST is independent — a `McpServer` instance per process,
/// state shared via the server's internal `RwLock<NegotiatedSession>`. For
/// truly stateless deployment (one container handles many concurrent
/// clients with different versions) we'd need to move negotiation into a
/// per-connection cookie; mark that as a future-work item.
#[cfg(feature = "http")]
pub async fn serve_http(server: Arc<McpServer>, addr: std::net::SocketAddr) -> io::Result<()> {
    use axum::{
        extract::State,
        http::StatusCode,
        response::{sse::Event, IntoResponse, Sse},
        routing::{get, post},
        Json, Router,
    };
    use futures::stream::{self, Stream};
    use std::convert::Infallible;
    use std::time::Duration;

    type SharedServer = Arc<McpServer>;

    async fn handle_post(
        State(srv): State<SharedServer>,
        Json(req): Json<JsonRpcRequest>,
    ) -> impl IntoResponse {
        match srv.handle_request(req).await {
            Some(resp) => (StatusCode::OK, Json(resp)).into_response(),
            None => StatusCode::NO_CONTENT.into_response(),
        }
    }

    async fn handle_sse(
        State(_srv): State<SharedServer>,
    ) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
        // Minimal heartbeat stream. Wired up to the real notification
        // bus when elicitation/notifications/* methods land.
        let stream = stream::unfold(0u64, |n| async move {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let evt = Event::default()
                .event("ping")
                .data(format!(r#"{{"seq":{}}}"#, n));
            Some((Ok::<_, Infallible>(evt), n + 1))
        });
        Sse::new(stream)
            .keep_alive(axum::response::sse::KeepAlive::new())
    }

    async fn handle_health() -> &'static str {
        "ok"
    }

    let app = Router::new()
        .route("/mcp", post(handle_post).get(handle_sse))
        .route("/healthz", get(handle_health))
        .with_state(server);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("mtw-mcp HTTP listening on {}", addr);
    axum::serve(listener, app)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok(())
}

#[cfg(not(feature = "http"))]
pub async fn serve_http(_server: Arc<McpServer>, _addr: std::net::SocketAddr) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "rebuild with --features http to enable HTTP transport",
    ))
}
