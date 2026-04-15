//! Scoring primitives for synthetic incident scenarios.
//!
//! A verdict combines three axes: did we name the right root cause, did we
//! cite the evidence that actually mattered, and did we resist the
//! adversarial red herrings.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Root cause matched and required evidence was cited.
    Pass,
    /// Root cause matched but some required evidence was missing.
    Partial,
    /// Root cause wrong, or an adversarial red herring was accepted.
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioScore {
    pub verdict: Verdict,
    /// Fraction of required evidence tags cited (0.0..=1.0).
    pub evidence_recall: f32,
    /// Fraction of red herrings the agent avoided (1.0 = ignored all).
    pub red_herring_resistance: f32,
    /// Fuzzy match of the root cause string (0.0..=1.0).
    pub root_cause_match: f32,
    /// Weighted composite score (0.0..=1.0).
    pub composite: f32,
    pub notes: Vec<String>,
}

impl ScenarioScore {
    pub fn compute(
        root_cause_match: f32,
        evidence_recall: f32,
        red_herring_resistance: f32,
        notes: Vec<String>,
    ) -> Self {
        // 50% root-cause match, 30% evidence recall, 20% red-herring resistance.
        let composite =
            (0.5 * root_cause_match) + (0.3 * evidence_recall) + (0.2 * red_herring_resistance);
        let verdict = if root_cause_match >= 0.8 && evidence_recall >= 0.8 {
            Verdict::Pass
        } else if root_cause_match >= 0.8 {
            Verdict::Partial
        } else {
            Verdict::Fail
        };
        Self {
            verdict,
            evidence_recall,
            red_herring_resistance,
            root_cause_match,
            composite,
            notes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_score_is_pass() {
        let s = ScenarioScore::compute(1.0, 1.0, 1.0, vec![]);
        assert_eq!(s.verdict, Verdict::Pass);
        assert!((s.composite - 1.0).abs() < 1e-6);
    }

    #[test]
    fn missing_evidence_is_partial() {
        let s = ScenarioScore::compute(1.0, 0.5, 1.0, vec![]);
        assert_eq!(s.verdict, Verdict::Partial);
    }

    #[test]
    fn wrong_root_cause_is_fail() {
        let s = ScenarioScore::compute(0.2, 1.0, 1.0, vec![]);
        assert_eq!(s.verdict, Verdict::Fail);
    }
}
