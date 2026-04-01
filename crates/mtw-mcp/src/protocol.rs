//! Minimal MCP protocol implementation (JSON-RPC over stdio)
//!
//! Implements the Model Context Protocol specification for tool serving.
//! No external MCP crate dependencies — pure serde_json.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::io::{self, BufRead, Write};
use std::pin::Pin;
use std::sync::Arc;

/// MCP tool definition
#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// MCP tool result content
#[derive(Debug, Serialize)]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

/// Handler function type
pub type ToolHandler = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync,
>;

/// JSON-RPC request
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

/// JSON-RPC response
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }
    fn error(id: Value, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".into(), id,
            result: None,
            error: Some(serde_json::json!({"code": code, "message": message})),
        }
    }
}

/// MCP Server
pub struct McpServer {
    name: String,
    version: String,
    tools: Vec<McpTool>,
    handlers: HashMap<String, ToolHandler>,
}

impl McpServer {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            tools: Vec::new(),
            handlers: HashMap::new(),
        }
    }

    /// Register a tool with its handler
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
        });
        self.handlers.insert(name, handler);
    }

    /// Run the MCP server on stdio
    pub async fn run(&self) {
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

            let id = req.id.clone().unwrap_or(Value::Null);
            let response = self.handle_request(req).await;

            if let Some(resp) = response {
                let mut out = stdout.lock();
                let _ = serde_json::to_writer(&mut out, &resp);
                let _ = out.write_all(b"\n");
                let _ = out.flush();
            }
        }
    }

    async fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = req.id.clone().unwrap_or(Value::Null);

        match req.method.as_str() {
            "initialize" => {
                Some(JsonRpcResponse::success(id, serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": self.name,
                        "version": self.version
                    }
                })))
            }

            "notifications/initialized" => None, // no response for notifications

            "tools/list" => {
                let tools: Vec<Value> = self.tools.iter().map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                    })
                }).collect();

                Some(JsonRpcResponse::success(id, serde_json::json!({ "tools": tools })))
            }

            "tools/call" => {
                let tool_name = req.params.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let args = req.params.get("arguments")
                    .cloned()
                    .unwrap_or(Value::Object(serde_json::Map::new()));

                if let Some(handler) = self.handlers.get(tool_name) {
                    match (handler)(args).await {
                        Ok(text) => {
                            Some(JsonRpcResponse::success(id, serde_json::json!({
                                "content": [{"type": "text", "text": text}],
                                "isError": false
                            })))
                        }
                        Err(err) => {
                            Some(JsonRpcResponse::success(id, serde_json::json!({
                                "content": [{"type": "text", "text": err}],
                                "isError": true
                            })))
                        }
                    }
                } else {
                    Some(JsonRpcResponse::error(id, -32601, &format!("tool not found: {}", tool_name)))
                }
            }

            "ping" => {
                Some(JsonRpcResponse::success(id, serde_json::json!({})))
            }

            _ => {
                // Unknown method — ignore notifications, error on requests
                if req.id.is_some() {
                    Some(JsonRpcResponse::error(id, -32601, &format!("method not found: {}", req.method)))
                } else {
                    None
                }
            }
        }
    }
}
