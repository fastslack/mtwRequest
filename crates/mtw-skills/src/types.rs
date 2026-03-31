use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillPermission {
    ReadContacts, WriteContacts, ReadTasks, WriteTasks,
    ReadReminders, WriteReminders, ReadEvents, WriteEvents,
    ReadFinance, WriteFinance, ReadHealth, WriteHealth,
    Network, FileSystem, Notifications, Voice,
    #[serde(untagged)]
    Custom(String),
}

impl fmt::Display for SkillPermission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadContacts => write!(f, "read:contacts"),
            Self::WriteContacts => write!(f, "write:contacts"),
            Self::ReadTasks => write!(f, "read:tasks"),
            Self::WriteTasks => write!(f, "write:tasks"),
            Self::ReadReminders => write!(f, "read:reminders"),
            Self::WriteReminders => write!(f, "write:reminders"),
            Self::ReadEvents => write!(f, "read:events"),
            Self::WriteEvents => write!(f, "write:events"),
            Self::ReadFinance => write!(f, "read:finance"),
            Self::WriteFinance => write!(f, "write:finance"),
            Self::ReadHealth => write!(f, "read:health"),
            Self::WriteHealth => write!(f, "write:health"),
            Self::Network => write!(f, "network"),
            Self::FileSystem => write!(f, "filesystem"),
            Self::Notifications => write!(f, "notifications"),
            Self::Voice => write!(f, "voice"),
            Self::Custom(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for SkillPermission {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read:contacts" => Ok(Self::ReadContacts), "write:contacts" => Ok(Self::WriteContacts),
            "read:tasks" => Ok(Self::ReadTasks), "write:tasks" => Ok(Self::WriteTasks),
            "read:reminders" => Ok(Self::ReadReminders), "write:reminders" => Ok(Self::WriteReminders),
            "read:events" => Ok(Self::ReadEvents), "write:events" => Ok(Self::WriteEvents),
            "read:finance" => Ok(Self::ReadFinance), "write:finance" => Ok(Self::WriteFinance),
            "read:health" => Ok(Self::ReadHealth), "write:health" => Ok(Self::WriteHealth),
            "network" => Ok(Self::Network), "filesystem" => Ok(Self::FileSystem),
            "notifications" => Ok(Self::Notifications), "voice" => Ok(Self::Voice),
            other => Ok(Self::Custom(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillCategory { Productivity, Communication, Finance, Health, Home, Utility, Custom }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub id: String, pub name: String, pub description: String, pub version: String,
    pub author: String, pub homepage: Option<String>, pub icon: Option<String>,
    pub category: Option<SkillCategory>, pub permissions: Vec<SkillPermission>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    #[serde(flatten)]
    pub metadata: SkillMetadata,
    pub main: String,
    pub min_kernel_version: Option<String>,
    pub dependencies: Vec<String>,
    pub config_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConfig {
    pub enabled: bool,
    pub granted_permissions: Vec<SkillPermission>,
    pub settings: HashMap<String, serde_json::Value>,
}

impl Default for SkillConfig {
    fn default() -> Self { Self { enabled: true, granted_permissions: Vec::new(), settings: HashMap::new() } }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Local { path: String }, Npm { package: String, version: Option<String> },
    Git { url: String, ref_name: Option<String> }, Bundled { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus { Available, Installed, Active, Disabled, Error }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_permission_roundtrip() {
        let p = SkillPermission::ReadContacts;
        assert_eq!(p.to_string(), "read:contacts");
        assert_eq!(SkillPermission::from_str("read:contacts").unwrap(), p);
    }
    #[test]
    fn test_custom_permission() {
        let p: SkillPermission = "custom:foo".parse().unwrap();
        assert_eq!(p, SkillPermission::Custom("custom:foo".into()));
    }
}
