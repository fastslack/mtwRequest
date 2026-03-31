use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignStatus { Draft, Sending, Sent, Paused, Cancelled }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipientStatus { Pending, Sent, Failed, Skipped }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailCampaign {
    pub id: String, pub name: String, pub template_id: Option<String>,
    pub account_id: Option<String>, pub status: CampaignStatus,
    pub subject_override: String, pub total_recipients: u32,
    pub sent_count: u32, pub failed_count: u32,
    pub scheduled_at: Option<String>, pub started_at: Option<String>,
    pub completed_at: Option<String>, pub created_at: String, pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignRecipient {
    pub id: String, pub campaign_id: String, pub contact_id: Option<String>,
    pub email: String, pub name: String, pub variables: HashMap<String, String>,
    pub status: RecipientStatus, pub comm_id: Option<String>,
    pub sent_at: Option<String>, pub error_message: String, pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignProgress {
    pub campaign_id: String, pub total: u32, pub sent: u32,
    pub failed: u32, pub pending: u32, pub percent: f32,
}

pub struct CampaignManager {
    campaigns: DashMap<String, EmailCampaign>,
    recipients: DashMap<String, Vec<CampaignRecipient>>,
}

impl CampaignManager {
    pub fn new() -> Self { Self { campaigns: DashMap::new(), recipients: DashMap::new() } }

    pub fn create(&self, name: impl Into<String>) -> EmailCampaign {
        let now = format!("{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
        let c = EmailCampaign {
            id: ulid::Ulid::new().to_string(), name: name.into(),
            template_id: None, account_id: None, status: CampaignStatus::Draft,
            subject_override: String::new(), total_recipients: 0,
            sent_count: 0, failed_count: 0, scheduled_at: None, started_at: None,
            completed_at: None, created_at: now.clone(), updated_at: now,
        };
        self.campaigns.insert(c.id.clone(), c.clone());
        self.recipients.insert(c.id.clone(), Vec::new());
        c
    }

    pub fn get(&self, id: &str) -> Option<EmailCampaign> { self.campaigns.get(id).map(|c| c.clone()) }
    pub fn list(&self) -> Vec<EmailCampaign> { self.campaigns.iter().map(|e| e.value().clone()).collect() }

    pub fn add_recipients(&self, campaign_id: &str, recs: Vec<CampaignRecipient>) -> Result<(), MtwError> {
        let mut entry = self.recipients.get_mut(campaign_id)
            .ok_or_else(|| MtwError::Internal(format!("campaign not found: {}", campaign_id)))?;
        let count = recs.len() as u32;
        entry.extend(recs);
        if let Some(mut c) = self.campaigns.get_mut(campaign_id) { c.total_recipients += count; }
        Ok(())
    }

    pub fn set_status(&self, id: &str, status: CampaignStatus) -> Result<(), MtwError> {
        let mut c = self.campaigns.get_mut(id)
            .ok_or_else(|| MtwError::Internal(format!("campaign not found: {}", id)))?;
        c.status = status;
        Ok(())
    }
}

impl Default for CampaignManager { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_create() {
        let m = CampaignManager::new();
        let c = m.create("Test");
        assert_eq!(c.status, CampaignStatus::Draft);
        assert_eq!(m.list().len(), 1);
    }
}
