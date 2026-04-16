//! Synthetic RCA scenarios: fixtures that pair a scripted evidence stream with
//! a ground-truth root cause, required evidence tags, and a set of accepted
//! red-herring labels. The [`ScenarioHarness`] runs an agent against one and
//! emits a [`ScenarioScore`].

use serde::{Deserialize, Serialize};

use crate::agent::{IncidentAgent, RootCauseHypothesis};
use crate::evidence::{Evidence, EvidenceStream};
use crate::score::ScenarioScore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub title: String,
    /// Canonical root-cause id the agent should converge on.
    pub ground_truth_root_cause: String,
    /// Accepted synonyms / substrings that should also count as correct.
    #[serde(default)]
    pub accepted_aliases: Vec<String>,
    /// Tags that MUST appear in the agent's `cited_tags` for a pass.
    #[serde(default)]
    pub required_evidence_tags: Vec<String>,
    /// Root-cause labels that an agent gets wrong if it outputs them
    /// (i.e. names that red herrings would suggest).
    #[serde(default)]
    pub red_herring_root_causes: Vec<String>,
    /// The scripted stream of evidence pieces fed to the agent.
    pub evidence: EvidenceStream,
}

impl Scenario {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Score a hypothesis against this scenario's ground truth.
    pub fn score(&self, h: &RootCauseHypothesis) -> ScenarioScore {
        let mut notes = Vec::new();
        let rc = h.root_cause.to_lowercase();
        let truth = self.ground_truth_root_cause.to_lowercase();
        let aliases: Vec<String> = self
            .accepted_aliases
            .iter()
            .map(|a| a.to_lowercase())
            .collect();

        let matches_truth = rc == truth
            || rc.contains(&truth)
            || truth.contains(&rc)
            || aliases.iter().any(|a| rc.contains(a) || a.contains(&rc));

        let accepted_red_herring = self
            .red_herring_root_causes
            .iter()
            .any(|bad| rc.contains(&bad.to_lowercase()));

        let root_cause_match = if accepted_red_herring {
            notes.push(format!("accepted red herring: {}", h.root_cause));
            0.0
        } else if matches_truth {
            1.0
        } else {
            // Partial credit for substring overlap with truth (simple token overlap).
            token_overlap(&rc, &truth)
        };

        let evidence_recall = if self.required_evidence_tags.is_empty() {
            1.0
        } else {
            let cited: Vec<String> = h.cited_tags.iter().map(|t| t.to_lowercase()).collect();
            let hit = self
                .required_evidence_tags
                .iter()
                .filter(|req| cited.iter().any(|c| c == &req.to_lowercase()))
                .count();
            hit as f32 / self.required_evidence_tags.len() as f32
        };

        let red_herring_resistance = if accepted_red_herring { 0.0 } else { 1.0 };

        ScenarioScore::compute(
            root_cause_match,
            evidence_recall,
            red_herring_resistance,
            notes,
        )
    }
}

fn token_overlap(a: &str, b: &str) -> f32 {
    let at: std::collections::HashSet<&str> = a
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    let bt: std::collections::HashSet<&str> = b
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    let inter = at.intersection(&bt).count();
    let union = at.union(&bt).count().max(1);
    inter as f32 / union as f32
}

/// Outcome of a scenario run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioRun {
    pub scenario_id: String,
    pub hypothesis: RootCauseHypothesis,
    pub score: ScenarioScore,
    /// Evidence pieces actually presented to the agent (red herrings included).
    pub presented: Vec<Evidence>,
}

pub struct ScenarioHarness {
    scenario: Scenario,
}

impl ScenarioHarness {
    pub fn new(scenario: Scenario) -> Self {
        Self { scenario }
    }

    /// Run the agent against the scenario. Evidence is presented verbatim —
    /// delays are not awaited so tests stay fast; if needed, a future variant
    /// can honour [`Evidence::delay_ms`].
    pub async fn run(&self, agent: &IncidentAgent) -> Result<ScenarioRun, mtw_core::MtwError> {
        let hypothesis = agent.investigate(&self.scenario.evidence).await?;
        let score = self.scenario.score(&hypothesis);
        Ok(ScenarioRun {
            scenario_id: self.scenario.id.clone(),
            hypothesis,
            score,
            presented: self.scenario.evidence.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scn() -> Scenario {
        Scenario {
            id: "s1".into(),
            title: "test".into(),
            ground_truth_root_cause: "db_pool_exhausted".into(),
            accepted_aliases: vec!["pool_exhausted".into()],
            required_evidence_tags: vec!["db".into(), "saturation".into()],
            red_herring_root_causes: vec!["cpu_spike".into()],
            evidence: vec![
                Evidence::log("app", "cannot acquire connection").with_tags(["db", "saturation"]),
                Evidence::metric("cw", "cpu 99%")
                    .with_tags(["cpu"])
                    .red_herring(),
            ],
        }
    }

    #[test]
    fn perfect_hypothesis_passes() {
        let s = scn();
        let h = RootCauseHypothesis {
            root_cause: "db_pool_exhausted".into(),
            rationale: "".into(),
            cited_tags: vec!["db".into(), "saturation".into()],
            required_next: vec![],
        };
        let score = s.score(&h);
        assert_eq!(score.verdict, crate::score::Verdict::Pass);
        assert!((score.root_cause_match - 1.0).abs() < 1e-6);
        assert!((score.evidence_recall - 1.0).abs() < 1e-6);
    }

    #[test]
    fn alias_matches_truth() {
        let s = scn();
        let h = RootCauseHypothesis {
            root_cause: "pool_exhausted".into(),
            rationale: "".into(),
            cited_tags: vec!["db".into(), "saturation".into()],
            required_next: vec![],
        };
        assert!(s.score(&h).root_cause_match >= 0.99);
    }

    #[test]
    fn accepted_red_herring_fails() {
        let s = scn();
        let h = RootCauseHypothesis {
            root_cause: "cpu_spike".into(),
            rationale: "".into(),
            cited_tags: vec!["db".into(), "saturation".into()],
            required_next: vec![],
        };
        let score = s.score(&h);
        assert_eq!(score.verdict, crate::score::Verdict::Fail);
        assert!((score.red_herring_resistance - 0.0).abs() < 1e-6);
    }

    #[test]
    fn missing_evidence_drops_recall() {
        let s = scn();
        let h = RootCauseHypothesis {
            root_cause: "db_pool_exhausted".into(),
            rationale: "".into(),
            cited_tags: vec!["db".into()],
            required_next: vec![],
        };
        let score = s.score(&h);
        assert_eq!(score.verdict, crate::score::Verdict::Partial);
        assert!((score.evidence_recall - 0.5).abs() < 1e-6);
    }
}
