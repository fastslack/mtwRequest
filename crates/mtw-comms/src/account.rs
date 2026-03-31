use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use crate::types::{AccountProvider, AccountType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAccount {
    pub id: String,
    pub label: String,
    pub email: String,
    pub account_type: AccountType,
    pub provider: AccountProvider,
    pub company: String,
    pub signature: String,
    pub provider_config: serde_json::Value,
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub struct AccountRegistry {
    accounts: DashMap<String, EmailAccount>,
    default_id: RwLock<Option<String>>,
}

impl AccountRegistry {
    pub fn new() -> Self {
        Self { accounts: DashMap::new(), default_id: RwLock::new(None) }
    }

    pub fn add(&self, account: EmailAccount) {
        if account.is_default {
            *self.default_id.write().unwrap() = Some(account.id.clone());
        }
        self.accounts.insert(account.id.clone(), account);
    }

    pub fn remove(&self, id: &str) -> Option<EmailAccount> {
        let removed = self.accounts.remove(id).map(|(_, a)| a);
        if let Some(ref default) = *self.default_id.read().unwrap() {
            if default == id { *self.default_id.write().unwrap() = None; }
        }
        removed
    }

    pub fn get(&self, id: &str) -> Option<EmailAccount> {
        self.accounts.get(id).map(|a| a.clone())
    }

    pub fn get_default(&self) -> Option<EmailAccount> {
        self.default_id.read().unwrap().as_ref()
            .and_then(|id| self.accounts.get(id).map(|a| a.clone()))
    }

    pub fn list(&self) -> Vec<EmailAccount> {
        self.accounts.iter().map(|e| e.value().clone()).collect()
    }

    pub fn set_default(&self, id: &str) -> Result<(), MtwError> {
        if !self.accounts.contains_key(id) {
            return Err(MtwError::Internal(format!("account not found: {}", id)));
        }
        if let Some(prev_id) = self.default_id.read().unwrap().as_ref() {
            if let Some(mut prev) = self.accounts.get_mut(prev_id) { prev.is_default = false; }
        }
        if let Some(mut account) = self.accounts.get_mut(id) { account.is_default = true; }
        *self.default_id.write().unwrap() = Some(id.to_string());
        Ok(())
    }
}

impl Default for AccountRegistry { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    fn make_account(id: &str, is_default: bool) -> EmailAccount {
        EmailAccount {
            id: id.into(), label: format!("Acc {}", id), email: format!("{}@ex.com", id),
            account_type: AccountType::Personal, provider: AccountProvider::Gmail,
            company: String::new(), signature: String::new(),
            provider_config: serde_json::json!({}), is_default,
            created_at: "0".into(), updated_at: "0".into(),
        }
    }

    #[test]
    fn test_add_and_default() {
        let reg = AccountRegistry::new();
        reg.add(make_account("a1", true));
        reg.add(make_account("a2", false));
        assert_eq!(reg.get_default().unwrap().id, "a1");
        reg.set_default("a2").unwrap();
        assert_eq!(reg.get_default().unwrap().id, "a2");
    }
}
