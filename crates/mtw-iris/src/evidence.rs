//! Evidence: the signal an incident agent consumes (logs, metrics, traces,
//! alerts, runbook entries). Evidence is tagged so scenarios can assert which
//! tags were actually required to reach the correct root cause.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Log,
    Metric,
    Trace,
    Alert,
    Runbook,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub source: String,
    pub message: String,
    /// Tags used by the harness to check "required evidence" coverage.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Scripted delivery delay in ms (used by scenarios; ignored in production).
    #[serde(default)]
    pub delay_ms: u64,
    /// If true, the scenario considers this piece adversarial (red herring).
    #[serde(default)]
    pub red_herring: bool,
}

impl Evidence {
    pub fn log(source: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: EvidenceKind::Log,
            source: source.into(),
            message: message.into(),
            tags: Vec::new(),
            delay_ms: 0,
            red_herring: false,
        }
    }

    pub fn metric(source: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: EvidenceKind::Metric,
            source: source.into(),
            message: message.into(),
            tags: Vec::new(),
            delay_ms: 0,
            red_herring: false,
        }
    }

    pub fn alert(source: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: EvidenceKind::Alert,
            source: source.into(),
            message: message.into(),
            tags: Vec::new(),
            delay_ms: 0,
            red_herring: false,
        }
    }

    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    pub fn red_herring(mut self) -> Self {
        self.red_herring = true;
        self
    }
}

/// A simple alias for a scripted stream of evidence pieces.
pub type EvidenceStream = Vec<Evidence>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_set_fields() {
        let e = Evidence::log("app", "ERROR connection refused").with_tags(["db", "network"]);
        assert_eq!(e.kind, EvidenceKind::Log);
        assert_eq!(e.tags, vec!["db", "network"]);
        assert!(!e.red_herring);

        let r = Evidence::metric("cw", "CPU 99%").red_herring();
        assert!(r.red_herring);
    }

    #[test]
    fn evidence_roundtrips_json() {
        let e = Evidence::alert("pagerduty", "p1 incident").with_tags(["paging"]);
        let s = serde_json::to_string(&e).unwrap();
        let back: Evidence = serde_json::from_str(&s).unwrap();
        assert_eq!(back.kind, EvidenceKind::Alert);
        assert_eq!(back.tags, vec!["paging"]);
    }
}
