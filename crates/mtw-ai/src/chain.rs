use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainCondition { Always, OnSuccess, OnFailure }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentChain {
    pub id: String, pub source_agent_id: String, pub target_agent_id: String,
    pub label: String, pub condition: ChainCondition, pub pass_result: bool,
    pub delay_ms: u64, pub active: bool, pub created_at: String,
}

pub struct ChainRegistry { chains: DashMap<String, AgentChain> }

impl ChainRegistry {
    pub fn new() -> Self { Self { chains: DashMap::new() } }

    pub fn add(&self, chain: AgentChain) -> Result<(), MtwError> {
        if chain.source_agent_id == chain.target_agent_id {
            return Err(MtwError::Agent("cannot chain agent to itself".into()));
        }
        self.chains.insert(chain.id.clone(), chain); Ok(())
    }

    pub fn remove(&self, id: &str) -> bool { self.chains.remove(id).is_some() }

    pub fn get_chains_for_source(&self, agent_id: &str) -> Vec<AgentChain> {
        self.chains.iter().filter(|e| e.source_agent_id == agent_id && e.active).map(|e| e.value().clone()).collect()
    }

    pub fn get_chains_for_target(&self, agent_id: &str) -> Vec<AgentChain> {
        self.chains.iter().filter(|e| e.target_agent_id == agent_id && e.active).map(|e| e.value().clone()).collect()
    }

    pub fn evaluate_condition(&self, condition: &ChainCondition, success: bool) -> bool {
        match condition {
            ChainCondition::Always => true,
            ChainCondition::OnSuccess => success,
            ChainCondition::OnFailure => !success,
        }
    }
}

impl Default for ChainRegistry { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    fn make_chain(src: &str, tgt: &str) -> AgentChain {
        AgentChain { id: ulid::Ulid::new().to_string(), source_agent_id: src.into(), target_agent_id: tgt.into(),
            label: "test".into(), condition: ChainCondition::Always, pass_result: true, delay_ms: 0, active: true,
            created_at: "0".into() }
    }
    #[test]
    fn test_chain_crud() {
        let reg = ChainRegistry::new();
        let c = make_chain("a1", "a2");
        reg.add(c.clone()).unwrap();
        assert_eq!(reg.get_chains_for_source("a1").len(), 1);
        assert_eq!(reg.get_chains_for_target("a2").len(), 1);
    }
    #[test]
    fn test_self_chain_rejected() {
        let reg = ChainRegistry::new();
        assert!(reg.add(make_chain("a1", "a1")).is_err());
    }
    #[test]
    fn test_condition_eval() {
        let reg = ChainRegistry::new();
        assert!(reg.evaluate_condition(&ChainCondition::Always, true));
        assert!(reg.evaluate_condition(&ChainCondition::OnSuccess, true));
        assert!(!reg.evaluate_condition(&ChainCondition::OnSuccess, false));
        assert!(reg.evaluate_condition(&ChainCondition::OnFailure, false));
    }
}
