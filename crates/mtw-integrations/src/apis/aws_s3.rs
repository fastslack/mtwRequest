//! AWS S3 integration.

use serde::{Deserialize, Serialize};

use super::{IntegrationInfo, IntegrationStatus};

pub const INFO: IntegrationInfo = IntegrationInfo {
    id: "aws_s3",
    name: "AWS S3",
    base_url: "https://s3.amazonaws.com",
    docs_url: "https://docs.aws.amazon.com/s3/",
    oauth2_supported: false,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwsS3Config {
    /// AWS access key ID.
    pub access_key_id: String,
    /// AWS secret access key.
    pub secret_access_key: String,
    /// AWS region (e.g. "us-east-1").
    #[serde(default = "default_region")]
    pub region: String,
    /// Default bucket name.
    pub bucket: Option<String>,
    /// Custom endpoint URL (for S3-compatible services like MinIO).
    pub endpoint_url: Option<String>,
    /// Path style access (required for some S3-compatible services).
    #[serde(default)]
    pub path_style: bool,
}

fn default_region() -> String {
    "us-east-1".to_string()
}

pub struct AwsS3Client {
    config: AwsS3Config,
    status: IntegrationStatus,
}

impl AwsS3Client {
    pub fn new(config: AwsS3Config) -> Self {
        Self {
            config,
            status: IntegrationStatus::Disconnected,
        }
    }

    pub fn config(&self) -> &AwsS3Config {
        &self.config
    }

    pub fn status(&self) -> &IntegrationStatus {
        &self.status
    }

    pub fn info() -> &'static IntegrationInfo {
        &INFO
    }

    pub async fn connect(&mut self) -> Result<(), String> {
        self.status = IntegrationStatus::Connected;
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.status = IntegrationStatus::Disconnected;
    }
}
