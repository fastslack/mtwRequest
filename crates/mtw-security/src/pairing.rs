use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// A time-limited pairing code for device authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingCode {
    pub code: String,
    pub platform: String,
    pub user_id: String,
    pub chat_id: String,
    pub approved: bool,
    pub approved_at: Option<String>,
    #[serde(skip)]
    pub expires_at: Option<Instant>,
}

/// A confirmed paired device
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub platform: String,
    pub user_id: String,
    pub chat_id: String,
    pub paired_at: String,
}

/// Manages device pairing with time-limited codes
pub struct PairingManager {
    pending: DashMap<String, PairingCode>,
    paired: DashMap<String, PairedDevice>,
    code_expiration: Duration,
}

impl PairingManager {
    pub fn new(code_expiration: Duration) -> Self {
        Self {
            pending: DashMap::new(),
            paired: DashMap::new(),
            code_expiration,
        }
    }

    /// Generate a 6-digit pairing code
    pub fn generate_code(
        &self,
        platform: impl Into<String>,
        user_id: impl Into<String>,
        chat_id: impl Into<String>,
    ) -> PairingCode {
        let code_num = ulid::Ulid::new().0 % 1_000_000;
        let code = format!("{:06}", code_num);

        let pairing = PairingCode {
            code: code.clone(),
            platform: platform.into(),
            user_id: user_id.into(),
            chat_id: chat_id.into(),
            approved: false,
            approved_at: None,
            expires_at: Some(Instant::now() + self.code_expiration),
        };

        self.pending.insert(code, pairing.clone());
        pairing
    }

    /// Verify a code exists and is not expired
    pub fn verify_code(&self, code: &str) -> Result<PairingCode, MtwError> {
        let entry = self
            .pending
            .get(code)
            .ok_or_else(|| MtwError::Auth("invalid pairing code".into()))?;

        if let Some(expires) = entry.expires_at {
            if Instant::now() >= expires {
                drop(entry);
                self.pending.remove(code);
                return Err(MtwError::Auth("pairing code expired".into()));
            }
        }

        Ok(entry.clone())
    }

    /// Approve a pairing code, registering the device
    pub fn approve(&self, code: &str) -> Result<PairedDevice, MtwError> {
        let mut entry = self
            .pending
            .get_mut(code)
            .ok_or_else(|| MtwError::Auth("invalid pairing code".into()))?;

        entry.approved = true;
        entry.approved_at = Some(format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));

        let device = PairedDevice {
            platform: entry.platform.clone(),
            user_id: entry.user_id.clone(),
            chat_id: entry.chat_id.clone(),
            paired_at: entry.approved_at.clone().unwrap_or_default(),
        };

        let key = format!("{}:{}", device.platform, device.user_id);
        self.paired.insert(key, device.clone());

        drop(entry);
        self.pending.remove(code);

        Ok(device)
    }

    /// Deny a pairing code
    pub fn deny(&self, code: &str) -> bool {
        self.pending.remove(code).is_some()
    }

    /// Check if a user on a platform is paired
    pub fn is_paired(&self, platform: &str, user_id: &str) -> bool {
        let key = format!("{}:{}", platform, user_id);
        self.paired.contains_key(&key)
    }

    /// Unpair a device
    pub fn unpair(&self, platform: &str, user_id: &str) -> bool {
        let key = format!("{}:{}", platform, user_id);
        self.paired.remove(&key).is_some()
    }

    /// List all paired devices
    pub fn list_paired(&self) -> Vec<PairedDevice> {
        self.paired.iter().map(|e| e.value().clone()).collect()
    }

    /// Remove expired pending codes
    pub fn cleanup_expired(&self) -> usize {
        let now = Instant::now();
        let expired: Vec<String> = self
            .pending
            .iter()
            .filter(|e| e.expires_at.map(|exp| now >= exp).unwrap_or(false))
            .map(|e| e.key().clone())
            .collect();

        let count = expired.len();
        for code in expired {
            self.pending.remove(&code);
        }
        count
    }
}

impl Default for PairingManager {
    fn default() -> Self {
        Self::new(Duration::from_secs(300))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_verify() {
        let manager = PairingManager::default();
        let pairing = manager.generate_code("telegram", "user1", "chat1");

        assert_eq!(pairing.code.len(), 6);
        assert!(!pairing.approved);

        let verified = manager.verify_code(&pairing.code).unwrap();
        assert_eq!(verified.platform, "telegram");
        assert_eq!(verified.user_id, "user1");
    }

    #[test]
    fn test_approve() {
        let manager = PairingManager::default();
        let pairing = manager.generate_code("telegram", "user1", "chat1");

        assert!(!manager.is_paired("telegram", "user1"));

        let device = manager.approve(&pairing.code).unwrap();
        assert_eq!(device.platform, "telegram");
        assert!(manager.is_paired("telegram", "user1"));

        // Code should be consumed
        assert!(manager.verify_code(&pairing.code).is_err());
    }

    #[test]
    fn test_deny() {
        let manager = PairingManager::default();
        let pairing = manager.generate_code("telegram", "user1", "chat1");

        assert!(manager.deny(&pairing.code));
        assert!(!manager.is_paired("telegram", "user1"));
        assert!(manager.verify_code(&pairing.code).is_err());
    }

    #[test]
    fn test_unpair() {
        let manager = PairingManager::default();
        let pairing = manager.generate_code("slack", "user2", "chat2");
        manager.approve(&pairing.code).unwrap();

        assert!(manager.is_paired("slack", "user2"));
        assert!(manager.unpair("slack", "user2"));
        assert!(!manager.is_paired("slack", "user2"));
    }

    #[test]
    fn test_invalid_code() {
        let manager = PairingManager::default();
        assert!(manager.verify_code("000000").is_err());
    }

    #[test]
    fn test_expired_code() {
        let manager = PairingManager::new(Duration::from_millis(0));
        let pairing = manager.generate_code("telegram", "user1", "chat1");

        // Code should already be expired
        std::thread::sleep(Duration::from_millis(1));
        assert!(manager.verify_code(&pairing.code).is_err());
    }
}
