//! Third-party API integration modules.
//!
//! Each sub-module provides a config struct and integration client stub
//! for a specific external service.

pub mod airtable;
pub mod aws_s3;
pub mod cloudflare;
pub mod discord;
pub mod docker_hub;
pub mod firebase;
pub mod gcs;
pub mod github;
pub mod gitlab;
pub mod jira;
pub mod linear;
pub mod notion;
pub mod paypal;
pub mod sendgrid;
pub mod slack;
pub mod stripe;
pub mod supabase;
pub mod telegram;
pub mod twilio;
pub mod vercel;

use serde::{Deserialize, Serialize};

/// Common status returned by integration health checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStatus {
    Connected,
    Disconnected,
    Error(String),
}

/// Metadata about an API integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationInfo {
    /// Machine-readable identifier (e.g. "github").
    pub id: &'static str,
    /// Human-readable display name.
    pub name: &'static str,
    /// Base URL of the API.
    pub base_url: &'static str,
    /// Documentation URL.
    pub docs_url: &'static str,
    /// Whether OAuth2 is supported.
    pub oauth2_supported: bool,
}
