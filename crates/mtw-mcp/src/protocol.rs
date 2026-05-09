//! MCP protocol implementation (JSON-RPC 2.0).
//!
//! Pure serde_json — no external MCP SDK dependency.
//!
//! ## Wire-level guarantees
//!
//! * **Version negotiation.** `initialize` reads the client's
//!   `protocolVersion` and returns the highest version both sides understand
//!   (see [`SUPPORTED_VERSIONS`]). Clients pinned at `2024-11-05` keep seeing
//!   exactly the legacy surface; newer clients unlock structured output,
//!   tasks, resources, prompts, elicitation, and applications.
//! * **Capability negotiation.** The server only advertises a capability
//!   when there's an actual provider wired for it — so a stdio server with
//!   no resource provider correctly omits `resources` from the response.
//! * **Backwards-compat tool API.** [`McpServer::tool`] keeps its 4-arg
//!   signature (`name, description, input_schema, handler`). Tools that want
//!   to declare a `outputSchema` use [`McpServer::tool_full`].
//!
//! ## Transport-agnostic
//!
//! This module knows nothing about stdio vs. HTTP. The transport layer
//! (`crate::transport`) feeds a single line / request body in, and gets a
//! single response line / body out.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Version table ───────────────────────────────────────────────────────────

/// Protocol versions this server speaks, ordered most-recent first. The
/// `initialize` handshake walks this list and returns the highest entry the
/// client also understands. If we don't recognise the client's version at
/// all, we fall back to the legacy `2024-11-05` so old clients keep working.
pub const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Version we *prefer* when the client doesn't pin one. Always the head of
/// `SUPPORTED_VERSIONS`.
pub const PREFERRED_VERSION: &str = "2025-06-18";

/// Legacy floor — pre-negotiation behaviour. If a feature isn't legal here,
/// we hide it from clients negotiating this version.
pub const LEGACY_VERSION: &str = "2024-11-05";

// ── Tool model ──────────────────────────────────────────────────────────────

/// MCP tool definition. `output_schema` is optional and only emitted on
/// `tools/list` for clients on a protocol version that understands it
/// (≥ `2025-03-26`).
#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(rename = "outputSchema", skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Free-form tags, used by `mtw_tool_search` for progressive discovery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Handler function type. Returns either a plain string (back-compat) or a
/// rich [`ToolResult`] for structured/UI content. Use [`ToolResult::text`]
/// for the simple case and [`ToolResult::structured`] / [`ToolResult::ui`]
/// for the new content types.
pub type ToolHandler = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolResult, String>> + Send>>
        + Send
        + Sync,
>;

/// What a tool returns. Encoded into the MCP `tools/call` response shape
/// according to the negotiated protocol version.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// Plain text payload. Always emitted for back-compat.
    pub text: String,
    /// Optional structured payload. Only emitted on protocols ≥ `2025-03-26`,
    /// inside the `structuredContent` field of the response.
    pub structured: Option<Value>,
    /// Optional UI payload (HTML or component JSON). Only emitted when the
    /// client declared the `experimental.applications` capability.
    pub ui: Option<UiContent>,
    /// Optional side-effects manifest the attestation layer signs into the
    /// receipt. Empty / `None` for pure read-only tools. The format is the
    /// same as `Receipt::side_effects` — see `mtw-attest`.
    pub side_effects: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiContent {
    /// `text/html`, `application/vnd.mtw.app+json`, etc.
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub body: String,
}

impl ToolResult {
    pub fn text(t: impl Into<String>) -> Self {
        Self { text: t.into(), structured: None, ui: None, side_effects: None }
    }

    pub fn structured(t: impl Into<String>, value: Value) -> Self {
        Self { text: t.into(), structured: Some(value), ui: None, side_effects: None }
    }

    pub fn ui(t: impl Into<String>, mime_type: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            text: t.into(),
            structured: None,
            ui: Some(UiContent { mime_type: mime_type.into(), body: body.into() }),
            side_effects: None,
        }
    }

    /// Attach a side-effects manifest that ends up in the signed receipt.
    /// Convention: `"<resource>.<verb>:<count>"`, e.g.
    /// `"agent.run.scheduled:1"`, `"channel.broadcast:42"`.
    pub fn with_side_effects(mut self, effects: Vec<String>) -> Self {
        self.side_effects = Some(effects);
        self
    }
}

impl From<String> for ToolResult {
    fn from(s: String) -> Self { Self::text(s) }
}

impl From<&str> for ToolResult {
    fn from(s: &str) -> Self { Self::text(s.to_string()) }
}

// ── JSON-RPC frames ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }
    pub fn error(id: Value, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(json!({"code": code, "message": message})),
        }
    }
}

// ── Capabilities ────────────────────────────────────────────────────────────

/// What the client declared it can do. Filled in by `initialize`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClientCapabilities {
    #[serde(default)]
    pub roots: Option<Value>,
    #[serde(default)]
    pub sampling: Option<Value>,
    #[serde(default)]
    pub elicitation: Option<Value>,
    #[serde(default)]
    pub experimental: Option<Value>,
}

impl ClientCapabilities {
    pub fn supports_elicitation(&self) -> bool { self.elicitation.is_some() }

    pub fn supports_applications(&self) -> bool {
        self.experimental
            .as_ref()
            .and_then(|v| v.get("applications"))
            .is_some()
    }
}

/// Outcome of `initialize`: every other request can ask the server which
/// version it landed on and what the client said it could do.
#[derive(Debug, Clone)]
pub struct NegotiatedSession {
    pub protocol_version: String,
    pub client_caps: ClientCapabilities,
}

impl NegotiatedSession {
    pub fn legacy() -> Self {
        Self {
            protocol_version: LEGACY_VERSION.to_string(),
            client_caps: ClientCapabilities::default(),
        }
    }

    /// Does the negotiated version include structured output / output schemas?
    pub fn supports_structured_output(&self) -> bool {
        self.protocol_version.as_str() >= "2025-03-26"
    }
}

// ── Optional providers ──────────────────────────────────────────────────────
//
// Each provider is plug-in: register it via `with_*` and its capability gets
// declared in `initialize`. None of these block compilation — they're all
// trait objects so we don't drag in mtw-skills, mtw-ai, etc. here.

/// Resources provider — exposes `resources/list` and `resources/read`.
/// Used by the skills-over-MCP integration.
#[async_trait::async_trait]
pub trait ResourceProvider: Send + Sync {
    async fn list(&self, cursor: Option<&str>) -> Result<Value, String>;
    async fn read(&self, uri: &str) -> Result<Value, String>;
}

/// Prompts provider — exposes `prompts/list` and `prompts/get`.
#[async_trait::async_trait]
pub trait PromptProvider: Send + Sync {
    async fn list(&self, cursor: Option<&str>) -> Result<Value, String>;
    async fn get(&self, name: &str, arguments: &Value) -> Result<Value, String>;
}

/// Tasks provider — exposes `tasks/create`, `tasks/get`, `tasks/list`,
/// `tasks/cancel`. Implemented by the `tasks` module.
#[async_trait::async_trait]
pub trait TaskProvider: Send + Sync {
    async fn create(&self, params: Value) -> Result<Value, String>;
    async fn get(&self, task_id: &str) -> Result<Value, String>;
    async fn list(&self, filter: Value) -> Result<Value, String>;
    async fn cancel(&self, task_id: &str) -> Result<Value, String>;
}

// ── Server ──────────────────────────────────────────────────────────────────

pub struct McpServer {
    name: String,
    version: String,
    tools: Vec<McpTool>,
    handlers: HashMap<String, ToolHandler>,
    resources: Option<Arc<dyn ResourceProvider>>,
    prompts: Option<Arc<dyn PromptProvider>>,
    tasks: Option<Arc<dyn TaskProvider>>,
    /// Optional Ed25519 attestation identity. When present, every
    /// successful `tools/call` response carries a signed `_meta.receipt`
    /// — see `mtw-attest`. When absent the server behaves exactly as
    /// before (no receipts) so legacy deployments aren't forced to roll
    /// out a keypair before they're ready.
    identity: Option<Arc<mtw_attest::Identity>>,
    /// Set by `initialize`; read by every other handler that needs to know
    /// the negotiated version or client caps.
    session: Arc<RwLock<NegotiatedSession>>,
}

impl McpServer {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            tools: Vec::new(),
            handlers: HashMap::new(),
            resources: None,
            prompts: None,
            tasks: None,
            identity: None,
            session: Arc::new(RwLock::new(NegotiatedSession::legacy())),
        }
    }

    pub fn with_identity(mut self, identity: Arc<mtw_attest::Identity>) -> Self {
        self.identity = Some(identity);
        self
    }

    pub fn identity(&self) -> Option<&Arc<mtw_attest::Identity>> {
        self.identity.as_ref()
    }

    /// Register a tool. Back-compat 4-arg signature — handlers returning
    /// `Result<String, String>` are auto-wrapped into [`ToolResult`].
    pub fn tool(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        schema: Value,
        handler: ToolHandler,
    ) {
        let name = name.into();
        self.tools.push(McpTool {
            name: name.clone(),
            description: description.into(),
            input_schema: schema,
            output_schema: None,
            tags: Vec::new(),
        });
        self.handlers.insert(name, handler);
    }

    /// Register a tool with full options: optional output schema and tags
    /// for progressive discovery via `mtw_tool_search`.
    pub fn tool_full(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        output_schema: Option<Value>,
        tags: Vec<String>,
        handler: ToolHandler,
    ) {
        let name = name.into();
        self.tools.push(McpTool {
            name: name.clone(),
            description: description.into(),
            input_schema,
            output_schema,
            tags,
        });
        self.handlers.insert(name, handler);
    }

    pub fn with_resources(mut self, p: Arc<dyn ResourceProvider>) -> Self {
        self.resources = Some(p);
        self
    }

    pub fn with_prompts(mut self, p: Arc<dyn PromptProvider>) -> Self {
        self.prompts = Some(p);
        self
    }

    pub fn with_tasks(mut self, p: Arc<dyn TaskProvider>) -> Self {
        self.tasks = Some(p);
        self
    }

    pub fn tools(&self) -> &[McpTool] { &self.tools }

    pub fn session_handle(&self) -> Arc<RwLock<NegotiatedSession>> { self.session.clone() }

    /// Invoke a tool by name without going through the JSON-RPC frame.
    /// Used by [`crate::code_mode`] to let scripts call other tools.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<ToolResult, String> {
        let handler = self
            .handlers
            .get(name)
            .ok_or_else(|| format!("tool not found: {}", name))?
            .clone();
        (handler)(args).await
    }

    /// Process one JSON-RPC request and return the response (None for
    /// notifications, which the caller must not echo back).
    pub async fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = req.id.clone().unwrap_or(Value::Null);
        let is_request = req.id.is_some();

        match req.method.as_str() {
            "initialize" => Some(self.handle_initialize(id, req.params).await),
            "notifications/initialized" => None,
            "ping" => Some(JsonRpcResponse::success(id, json!({}))),
            "tools/list" => Some(self.handle_tools_list(id).await),
            "tools/call" => Some(self.handle_tools_call(id, req.params).await),
            "resources/list" => Some(self.handle_resources_list(id, req.params).await),
            "resources/read" => Some(self.handle_resources_read(id, req.params).await),
            "prompts/list" => Some(self.handle_prompts_list(id, req.params).await),
            "prompts/get" => Some(self.handle_prompts_get(id, req.params).await),
            "tasks/create" => Some(self.handle_tasks_create(id, req.params).await),
            "tasks/get" => Some(self.handle_tasks_get(id, req.params).await),
            "tasks/list" => Some(self.handle_tasks_list(id, req.params).await),
            "tasks/cancel" => Some(self.handle_tasks_cancel(id, req.params).await),
            _ => {
                if is_request {
                    Some(JsonRpcResponse::error(
                        id,
                        -32601,
                        &format!("method not found: {}", req.method),
                    ))
                } else {
                    None
                }
            }
        }
    }

    async fn handle_initialize(&self, id: Value, params: Value) -> JsonRpcResponse {
        let client_version = params
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .unwrap_or(LEGACY_VERSION);

        // Pick the highest version we both understand. If the client asks
        // for something we know, return that; else return our most recent
        // and let the client decide whether to proceed.
        let chosen = if SUPPORTED_VERSIONS.contains(&client_version) {
            client_version.to_string()
        } else {
            PREFERRED_VERSION.to_string()
        };

        let client_caps: ClientCapabilities = params
            .get("capabilities")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();

        // Persist the negotiated session.
        {
            let mut s = self.session.write().await;
            *s = NegotiatedSession {
                protocol_version: chosen.clone(),
                client_caps: client_caps.clone(),
            };
        }

        // Build server capabilities map. Only declare what we actually have
        // a provider for, so the client doesn't see lies.
        let mut caps = json!({
            "tools": { "listChanged": false }
        });
        if self.resources.is_some() {
            caps["resources"] = json!({ "listChanged": false, "subscribe": false });
        }
        if self.prompts.is_some() {
            caps["prompts"] = json!({ "listChanged": false });
        }
        if self.tasks.is_some() {
            caps["tasks"] = json!({});
        }
        if client_caps.supports_applications() {
            caps["experimental"] = json!({ "applications": {} });
        }

        JsonRpcResponse::success(
            id,
            json!({
                "protocolVersion": chosen,
                "capabilities": caps,
                "serverInfo": {
                    "name": self.name,
                    "version": self.version,
                },
            }),
        )
    }

    async fn handle_tools_list(&self, id: Value) -> JsonRpcResponse {
        let session = self.session.read().await.clone();
        let allow_output_schema = session.supports_structured_output();

        let tools: Vec<Value> = self
            .tools
            .iter()
            .map(|t| {
                let mut entry = json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                });
                if allow_output_schema {
                    if let Some(os) = &t.output_schema {
                        entry["outputSchema"] = os.clone();
                    }
                }
                entry
            })
            .collect();

        JsonRpcResponse::success(id, json!({ "tools": tools }))
    }

    async fn handle_tools_call(&self, id: Value, params: Value) -> JsonRpcResponse {
        let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new()));

        let Some(handler) = self.handlers.get(&tool_name) else {
            return JsonRpcResponse::error(id, -32601, &format!("tool not found: {}", tool_name));
        };

        // Snapshot args for the receipt before handing them to the
        // handler — the handler is allowed to mutate its own copy.
        let args_for_receipt = args.clone();

        match (handler)(args).await {
            Ok(result) => {
                let session = self.session.read().await.clone();
                let receipt = self.maybe_sign_receipt(&tool_name, &args_for_receipt, &result, false);
                JsonRpcResponse::success(id, encode_tool_result(&result, &session, false, receipt))
            }
            Err(err) => {
                let session = self.session.read().await.clone();
                let result = ToolResult::text(err);
                let receipt = self.maybe_sign_receipt(&tool_name, &args_for_receipt, &result, true);
                JsonRpcResponse::success(id, encode_tool_result(&result, &session, true, receipt))
            }
        }
    }

    /// Build a signed [`mtw_attest::Receipt`] for one tool invocation when
    /// the server has an identity configured, otherwise return `None`.
    /// The receipt hashes:
    ///   * input — the raw `arguments` object the client sent
    ///   * output — a stable projection of the tool result (text +
    ///     structured payload + ui mimetype if present + isError)
    /// so downstream verifiers can replay the call with the same
    /// arguments and confirm the produced output matches.
    fn maybe_sign_receipt(
        &self,
        tool: &str,
        args: &Value,
        result: &ToolResult,
        is_error: bool,
    ) -> Option<mtw_attest::Receipt> {
        let identity = self.identity.as_ref()?;
        let input_hash = mtw_attest::hash_json(args);
        let output_value = output_canonical_value(result, is_error);
        let output_hash = mtw_attest::hash_json(&output_value);
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let side_effects = result.side_effects.clone().unwrap_or_default();
        Some(mtw_attest::Receipt::sign(
            identity,
            tool,
            input_hash,
            output_hash,
            side_effects,
            ts_ms,
        ))
    }

    // ── Resources ───────────────────────────────────────────────────────────

    async fn handle_resources_list(&self, id: Value, params: Value) -> JsonRpcResponse {
        match &self.resources {
            None => JsonRpcResponse::error(id, -32601, "resources not supported"),
            Some(p) => {
                let cursor = params.get("cursor").and_then(|v| v.as_str());
                match p.list(cursor).await {
                    Ok(v) => JsonRpcResponse::success(id, v),
                    Err(e) => JsonRpcResponse::error(id, -32603, &e),
                }
            }
        }
    }

    async fn handle_resources_read(&self, id: Value, params: Value) -> JsonRpcResponse {
        match &self.resources {
            None => JsonRpcResponse::error(id, -32601, "resources not supported"),
            Some(p) => {
                let Some(uri) = params.get("uri").and_then(|v| v.as_str()) else {
                    return JsonRpcResponse::error(id, -32602, "missing 'uri'");
                };
                match p.read(uri).await {
                    Ok(v) => JsonRpcResponse::success(id, v),
                    Err(e) => JsonRpcResponse::error(id, -32603, &e),
                }
            }
        }
    }

    // ── Prompts ─────────────────────────────────────────────────────────────

    async fn handle_prompts_list(&self, id: Value, params: Value) -> JsonRpcResponse {
        match &self.prompts {
            None => JsonRpcResponse::error(id, -32601, "prompts not supported"),
            Some(p) => {
                let cursor = params.get("cursor").and_then(|v| v.as_str());
                match p.list(cursor).await {
                    Ok(v) => JsonRpcResponse::success(id, v),
                    Err(e) => JsonRpcResponse::error(id, -32603, &e),
                }
            }
        }
    }

    async fn handle_prompts_get(&self, id: Value, params: Value) -> JsonRpcResponse {
        match &self.prompts {
            None => JsonRpcResponse::error(id, -32601, "prompts not supported"),
            Some(p) => {
                let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
                    return JsonRpcResponse::error(id, -32602, "missing 'name'");
                };
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match p.get(name, &args).await {
                    Ok(v) => JsonRpcResponse::success(id, v),
                    Err(e) => JsonRpcResponse::error(id, -32603, &e),
                }
            }
        }
    }

    // ── Tasks ───────────────────────────────────────────────────────────────

    async fn handle_tasks_create(&self, id: Value, params: Value) -> JsonRpcResponse {
        match &self.tasks {
            None => JsonRpcResponse::error(id, -32601, "tasks not supported"),
            Some(p) => match p.create(params).await {
                Ok(v) => JsonRpcResponse::success(id, v),
                Err(e) => JsonRpcResponse::error(id, -32603, &e),
            },
        }
    }

    async fn handle_tasks_get(&self, id: Value, params: Value) -> JsonRpcResponse {
        match &self.tasks {
            None => JsonRpcResponse::error(id, -32601, "tasks not supported"),
            Some(p) => {
                let Some(task_id) = params.get("task_id").and_then(|v| v.as_str()) else {
                    return JsonRpcResponse::error(id, -32602, "missing 'task_id'");
                };
                match p.get(task_id).await {
                    Ok(v) => JsonRpcResponse::success(id, v),
                    Err(e) => JsonRpcResponse::error(id, -32603, &e),
                }
            }
        }
    }

    async fn handle_tasks_list(&self, id: Value, params: Value) -> JsonRpcResponse {
        match &self.tasks {
            None => JsonRpcResponse::error(id, -32601, "tasks not supported"),
            Some(p) => match p.list(params).await {
                Ok(v) => JsonRpcResponse::success(id, v),
                Err(e) => JsonRpcResponse::error(id, -32603, &e),
            },
        }
    }

    async fn handle_tasks_cancel(&self, id: Value, params: Value) -> JsonRpcResponse {
        match &self.tasks {
            None => JsonRpcResponse::error(id, -32601, "tasks not supported"),
            Some(p) => {
                let Some(task_id) = params.get("task_id").and_then(|v| v.as_str()) else {
                    return JsonRpcResponse::error(id, -32602, "missing 'task_id'");
                };
                match p.cancel(task_id).await {
                    Ok(v) => JsonRpcResponse::success(id, v),
                    Err(e) => JsonRpcResponse::error(id, -32603, &e),
                }
            }
        }
    }

    /// Run the server on stdio. Kept for back-compat with `main.rs`; new code
    /// should use `crate::transport::serve_stdio` / `serve_http`.
    pub async fn run(self) {
        crate::transport::serve_stdio(Arc::new(self)).await;
    }
}

/// Build the `tools/call` response payload, version-aware. When a
/// `receipt` is provided (server has an attestation identity), it gets
/// attached under `_meta.receipt` — the spec-blessed extension slot for
/// transport-level metadata, so legacy clients ignore it cleanly.
fn encode_tool_result(
    result: &ToolResult,
    session: &NegotiatedSession,
    is_error: bool,
    receipt: Option<mtw_attest::Receipt>,
) -> Value {
    // Always emit the canonical text content for back-compat.
    let mut content = vec![json!({ "type": "text", "text": result.text })];

    // UI block — only if client opted in.
    if let Some(ui) = &result.ui {
        if session.client_caps.supports_applications() {
            content.push(json!({
                "type": "ui",
                "mimeType": ui.mime_type,
                "body": ui.body,
            }));
        }
    }

    let mut out = json!({
        "content": content,
        "isError": is_error,
    });

    // Structured content — only on protocols that defined it.
    if let Some(s) = &result.structured {
        if session.supports_structured_output() {
            out["structuredContent"] = s.clone();
        }
    }

    if let Some(r) = receipt {
        // `_meta` is the standard MCP slot for transport metadata. We
        // namespace under `mtw.attestation` so a future spec-level
        // `_meta.receipt` field can land without colliding.
        out["_meta"] = json!({
            "mtw.attestation": serde_json::to_value(&r).unwrap_or(Value::Null)
        });
    }

    out
}

/// Canonical projection of a [`ToolResult`] used for receipt hashing.
///
/// Verifiers reconstruct this from the same `tools/call` response shape
/// the server emitted, so the fields here MUST be a function of *what
/// goes on the wire*, not of internal state. Specifically:
///   * text content array (in order)
///   * structuredContent (if present)
///   * ui mimeType (the body itself is independent client-rendered
///     surface and not always shipped — we hash mimeType so swapping
///     content type is detectable, but not the body itself)
///   * isError flag
/// Side-effects live in the receipt body, not here.
fn output_canonical_value(result: &ToolResult, is_error: bool) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("text".into(), Value::String(result.text.clone()));
    obj.insert("isError".into(), Value::Bool(is_error));
    if let Some(s) = &result.structured {
        obj.insert("structuredContent".into(), s.clone());
    }
    if let Some(ui) = &result.ui {
        obj.insert("uiMimeType".into(), Value::String(ui.mime_type.clone()));
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(method: &str, params: Value, id: i64) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: method.into(),
            params,
        }
    }

    #[tokio::test]
    async fn legacy_client_pinned_to_2024_11_05() {
        let server = McpServer::new("t", "0.0.0");
        let resp = server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "2024-11-05", "capabilities": {}}),
                1,
            ))
            .await
            .unwrap();
        let v = resp.result.unwrap();
        assert_eq!(v["protocolVersion"], "2024-11-05");
        // Output schema must NOT be advertised on legacy.
        assert!(v["capabilities"].get("resources").is_none());
    }

    #[tokio::test]
    async fn new_client_gets_preferred_version() {
        let server = McpServer::new("t", "0.0.0");
        let resp = server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "2025-06-18", "capabilities": {}}),
                1,
            ))
            .await
            .unwrap();
        let v = resp.result.unwrap();
        assert_eq!(v["protocolVersion"], "2025-06-18");
    }

    #[tokio::test]
    async fn unknown_version_falls_back_to_preferred() {
        let server = McpServer::new("t", "0.0.0");
        let resp = server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "9999-12-31", "capabilities": {}}),
                1,
            ))
            .await
            .unwrap();
        let v = resp.result.unwrap();
        assert_eq!(v["protocolVersion"], PREFERRED_VERSION);
    }

    #[tokio::test]
    async fn output_schema_hidden_on_legacy_listed_on_new() {
        let mut server = McpServer::new("t", "0.0.0");
        server.tool_full(
            "echo",
            "echo",
            json!({"type": "object"}),
            Some(json!({"type": "object", "properties": {"x": {"type": "string"}}})),
            vec!["test".into()],
            Arc::new(|_| Box::pin(async { Ok(ToolResult::text("ok")) })),
        );

        // Legacy
        server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "2024-11-05"}),
                1,
            ))
            .await;
        let resp = server.handle_request(req("tools/list", json!({}), 2)).await.unwrap();
        let tools = resp.result.unwrap()["tools"].clone();
        assert!(tools[0].get("outputSchema").is_none(), "legacy must not see outputSchema");

        // New
        server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "2025-06-18"}),
                3,
            ))
            .await;
        let resp = server.handle_request(req("tools/list", json!({}), 4)).await.unwrap();
        let tools = resp.result.unwrap()["tools"].clone();
        assert!(tools[0].get("outputSchema").is_some(), "new client must see outputSchema");
    }

    #[tokio::test]
    async fn ui_content_only_when_client_opts_in() {
        let mut server = McpServer::new("t", "0.0.0");
        server.tool(
            "render",
            "render",
            json!({}),
            Arc::new(|_| {
                Box::pin(async {
                    Ok(ToolResult::ui("hello", "text/html", "<b>hi</b>"))
                })
            }),
        );

        // Without applications cap
        server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "2025-06-18"}),
                1,
            ))
            .await;
        let resp = server
            .handle_request(req("tools/call", json!({"name": "render"}), 2))
            .await
            .unwrap();
        let content = resp.result.unwrap()["content"].clone();
        assert_eq!(content.as_array().unwrap().len(), 1);

        // With applications cap
        server
            .handle_request(req(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"experimental": {"applications": {}}}
                }),
                3,
            ))
            .await;
        let resp = server
            .handle_request(req("tools/call", json!({"name": "render"}), 4))
            .await
            .unwrap();
        let content = resp.result.unwrap()["content"].clone();
        assert_eq!(content.as_array().unwrap().len(), 2);
        assert_eq!(content[1]["type"], "ui");
    }

    #[tokio::test]
    async fn tools_list_legacy_shape_matches_old_protocol() {
        // Ensure existing clients see exactly the same tool entries.
        let mut server = McpServer::new("t", "0.0.0");
        server.tool(
            "old_tool",
            "old description",
            json!({"type": "object", "properties": {}, "required": []}),
            Arc::new(|_| Box::pin(async { Ok(ToolResult::text("x")) })),
        );

        server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "2024-11-05"}),
                1,
            ))
            .await;
        let resp = server.handle_request(req("tools/list", json!({}), 2)).await.unwrap();
        let v = resp.result.unwrap();
        assert_eq!(v["tools"][0]["name"], "old_tool");
        assert_eq!(v["tools"][0]["description"], "old description");
        assert!(v["tools"][0].get("inputSchema").is_some());
    }

    #[tokio::test]
    async fn tools_call_back_compat_ok_path() {
        let mut server = McpServer::new("t", "0.0.0");
        server.tool(
            "echo",
            "echo",
            json!({}),
            Arc::new(|_| Box::pin(async { Ok(ToolResult::text("hello")) })),
        );
        server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "2024-11-05"}),
                1,
            ))
            .await;
        let resp = server
            .handle_request(req("tools/call", json!({"name": "echo"}), 2))
            .await
            .unwrap();
        let v = resp.result.unwrap();
        assert_eq!(v["isError"], false);
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][0]["text"], "hello");
        assert!(v.get("structuredContent").is_none());
    }

    #[tokio::test]
    async fn no_identity_means_no_receipt_in_meta() {
        let mut server = McpServer::new("t", "0.0.0");
        server.tool(
            "echo",
            "echo",
            json!({}),
            Arc::new(|_| Box::pin(async { Ok(ToolResult::text("hello")) })),
        );
        server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "2025-06-18"}),
                1,
            ))
            .await;
        let resp = server
            .handle_request(req("tools/call", json!({"name": "echo"}), 2))
            .await
            .unwrap();
        let v = resp.result.unwrap();
        assert!(v.get("_meta").is_none(), "no identity → no receipt");
    }

    #[tokio::test]
    async fn identity_present_emits_signed_receipt() {
        let identity = Arc::new(mtw_attest::Identity::generate());
        let server_id = identity.server_id();
        let mut server = McpServer::new("t", "0.0.0").with_identity(identity);
        server.tool(
            "demo",
            "demo",
            json!({}),
            Arc::new(|_| {
                Box::pin(async {
                    Ok(ToolResult::structured(
                        "ok".to_string(),
                        json!({"answer": 42}),
                    )
                    .with_side_effects(vec!["log.appended:1".into()]))
                })
            }),
        );
        server
            .handle_request(req(
                "initialize",
                json!({"protocolVersion": "2025-06-18"}),
                1,
            ))
            .await;
        let resp = server
            .handle_request(req(
                "tools/call",
                json!({"name": "demo", "arguments": {"q": "hi"}}),
                2,
            ))
            .await
            .unwrap();
        let v = resp.result.unwrap();

        // Receipt is parked under _meta.mtw.attestation.
        let receipt_v = v["_meta"]["mtw.attestation"].clone();
        assert!(!receipt_v.is_null(), "expected attestation receipt");

        let receipt: mtw_attest::Receipt = serde_json::from_value(receipt_v).unwrap();
        assert_eq!(receipt.tool, "demo");
        assert_eq!(receipt.server_id, server_id);
        assert_eq!(receipt.side_effects, vec!["log.appended:1".to_string()]);
        receipt.verify().expect("signature must round-trip");
    }
}
