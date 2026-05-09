//! HTTP client for mtwKernel's agent API.
//!
//! mtwKernel exposes its agent surface over an authenticated REST API
//! (`/api/agents`, `/api/agents/run`, `/api/agents/ranks`). The MCP server
//! proxies a curated subset of that API as the `mtw_kernel_agents_*` tools so
//! Claude Code can discover and dispatch the kernel's real agent fleet —
//! distinct from `mtw_agents_*`, which talk to mtw-request's own SQLite store.
//!
//! Configuration (env vars, all optional with sensible defaults):
//!   * `KERNEL_URL`         — base URL (default: `http://localhost:3087`)
//!   * `KERNEL_AUTH_TOKEN`  — Bearer token; if empty, requests go unauth and
//!                            the kernel returns 401 unless `KERNEL_ALLOW_UNAUTH=1`
//!                            is set on the kernel side.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Subset of the kernel's `Agent` row that we expose to Claude Code. We don't
/// pull every column — system_prompt is large and goal_template is rarely
/// useful when *picking* an agent.
#[derive(Debug, Clone, Deserialize)]
pub struct KernelAgent {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub executor_type: String,
    #[serde(default)]
    pub flow_id: String,
    #[serde(default)]
    pub rank_id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub show_on_dashboard: i64,
    #[serde(default)]
    pub active: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KernelRank {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub level: i64,
    #[serde(default)]
    pub insignia: String,
}

#[derive(Debug, Deserialize)]
struct AgentsListResponse {
    agents: Vec<KernelAgent>,
}

#[derive(Debug, Deserialize)]
struct RanksListResponse {
    ranks: Vec<KernelRank>,
}

#[derive(Debug, Serialize)]
struct RunRequest<'a> {
    agent_id: &'a str,
    goal: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
pub struct KernelRunResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

pub struct KernelClient {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl KernelClient {
    /// Build a client from env. Never fails — missing creds just produce 401s
    /// at call time, which we surface back to Claude as a clear error.
    pub fn from_env() -> Self {
        let base_url = std::env::var("KERNEL_URL")
            .unwrap_or_else(|_| "http://localhost:3087".to_string());
        let token = std::env::var("KERNEL_AUTH_TOKEN").ok().filter(|s| !s.is_empty());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client build");
        Self { base_url, token, http }
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => req.header("Authorization", format!("Bearer {}", t)),
            None => req,
        }
    }

    pub async fn list_agents(&self) -> Result<Vec<KernelAgent>, String> {
        let url = format!("{}/api/agents", self.base_url);
        let resp = self
            .auth(self.http.get(&url))
            .send()
            .await
            .map_err(|e| format!("kernel GET /api/agents failed: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!(
                "kernel GET /api/agents -> HTTP {}",
                resp.status().as_u16()
            ));
        }
        let body: AgentsListResponse = resp
            .json()
            .await
            .map_err(|e| format!("kernel /api/agents body parse: {}", e))?;
        Ok(body.agents)
    }

    pub async fn list_ranks(&self) -> Result<Vec<KernelRank>, String> {
        let url = format!("{}/api/agents/ranks", self.base_url);
        let resp = self
            .auth(self.http.get(&url))
            .send()
            .await
            .map_err(|e| format!("kernel GET /api/agents/ranks failed: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!(
                "kernel GET /api/agents/ranks -> HTTP {}",
                resp.status().as_u16()
            ));
        }
        let body: RanksListResponse = resp
            .json()
            .await
            .map_err(|e| format!("kernel /api/agents/ranks body parse: {}", e))?;
        Ok(body.ranks)
    }

    pub async fn run_agent(
        &self,
        agent_id: &str,
        goal: &str,
        workspace: Option<&str>,
    ) -> Result<KernelRunResponse, String> {
        let url = format!("{}/api/agents/run", self.base_url);
        let body = RunRequest { agent_id, goal, workspace };
        let resp = self
            .auth(self.http.post(&url).json(&body))
            .send()
            .await
            .map_err(|e| format!("kernel POST /api/agents/run failed: {}", e))?;
        let status = resp.status();
        let parsed: KernelRunResponse = resp
            .json()
            .await
            .map_err(|e| format!("kernel /api/agents/run body parse: {}", e))?;
        if !status.is_success() {
            return Err(parsed.error.unwrap_or_else(|| {
                format!("kernel /api/agents/run -> HTTP {}", status.as_u16())
            }));
        }
        Ok(parsed)
    }
}
