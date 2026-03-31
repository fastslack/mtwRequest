use serde::{Deserialize, Serialize};
use std::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackOutcome { Success, Partial, Failure, Neutral }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFeedback {
    pub id: String, pub agent_id: String, pub run_id: String,
    pub rating: u8, pub outcome: FeedbackOutcome,
    pub lesson: String, pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningType { Pattern, Avoid, Prefer, Insight }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLearning {
    pub id: String, pub agent_id: String, pub learning_type: LearningType,
    pub content: String, pub confidence: f64,
    pub source_runs: Vec<String>, pub active: bool,
    pub created_at: String, pub updated_at: String,
}

pub struct FeedbackStore {
    feedback: RwLock<Vec<AgentFeedback>>,
    learnings: RwLock<Vec<AgentLearning>>,
}

impl FeedbackStore {
    pub fn new() -> Self { Self { feedback: RwLock::new(Vec::new()), learnings: RwLock::new(Vec::new()) } }

    pub fn add_feedback(&self, fb: AgentFeedback) { self.feedback.write().unwrap().push(fb); }
    pub fn add_learning(&self, l: AgentLearning) { self.learnings.write().unwrap().push(l); }

    pub fn get_feedback_for_agent(&self, agent_id: &str) -> Vec<AgentFeedback> {
        self.feedback.read().unwrap().iter().filter(|f| f.agent_id == agent_id).cloned().collect()
    }

    pub fn get_learnings_for_agent(&self, agent_id: &str) -> Vec<AgentLearning> {
        self.learnings.read().unwrap().iter().filter(|l| l.agent_id == agent_id).cloned().collect()
    }

    pub fn get_active_learnings(&self, agent_id: &str) -> Vec<AgentLearning> {
        self.learnings.read().unwrap().iter().filter(|l| l.agent_id == agent_id && l.active).cloned().collect()
    }

    pub fn update_confidence(&self, learning_id: &str, confidence: f64) -> bool {
        let mut learnings = self.learnings.write().unwrap();
        if let Some(l) = learnings.iter_mut().find(|l| l.id == learning_id) { l.confidence = confidence; true } else { false }
    }

    pub fn deactivate_learning(&self, learning_id: &str) -> bool {
        let mut learnings = self.learnings.write().unwrap();
        if let Some(l) = learnings.iter_mut().find(|l| l.id == learning_id) { l.active = false; true } else { false }
    }

    pub fn avg_rating(&self, agent_id: &str) -> Option<f64> {
        let fb = self.get_feedback_for_agent(agent_id);
        if fb.is_empty() { return None; }
        Some(fb.iter().map(|f| f.rating as f64).sum::<f64>() / fb.len() as f64)
    }
}

impl Default for FeedbackStore { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_feedback() {
        let store = FeedbackStore::new();
        store.add_feedback(AgentFeedback { id: "f1".into(), agent_id: "a1".into(), run_id: "r1".into(),
            rating: 4, outcome: FeedbackOutcome::Success, lesson: "good".into(), created_at: "0".into() });
        store.add_feedback(AgentFeedback { id: "f2".into(), agent_id: "a1".into(), run_id: "r2".into(),
            rating: 2, outcome: FeedbackOutcome::Failure, lesson: "bad".into(), created_at: "0".into() });
        assert_eq!(store.get_feedback_for_agent("a1").len(), 2);
        assert!((store.avg_rating("a1").unwrap() - 3.0).abs() < 0.01);
    }
    #[test]
    fn test_learnings() {
        let store = FeedbackStore::new();
        store.add_learning(AgentLearning { id: "l1".into(), agent_id: "a1".into(), learning_type: LearningType::Pattern,
            content: "test".into(), confidence: 0.8, source_runs: vec![], active: true, created_at: "0".into(), updated_at: "0".into() });
        assert_eq!(store.get_active_learnings("a1").len(), 1);
        store.deactivate_learning("l1");
        assert_eq!(store.get_active_learnings("a1").len(), 0);
    }
}
