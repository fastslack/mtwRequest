use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFlow {
    pub id: String, pub name: String, pub description: String,
    pub color: String, pub active: bool, pub created_at: String, pub updated_at: String,
}

pub struct FlowManager { flows: DashMap<String, AgentFlow> }

impl FlowManager {
    pub fn new() -> Self { Self { flows: DashMap::new() } }

    pub fn create(&self, name: impl Into<String>, description: impl Into<String>) -> AgentFlow {
        let now = format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
        let flow = AgentFlow { id: ulid::Ulid::new().to_string(), name: name.into(), description: description.into(),
            color: "#6366f1".into(), active: true, created_at: now.clone(), updated_at: now };
        self.flows.insert(flow.id.clone(), flow.clone());
        flow
    }

    pub fn get(&self, id: &str) -> Option<AgentFlow> { self.flows.get(id).map(|f| f.clone()) }
    pub fn list(&self) -> Vec<AgentFlow> { self.flows.iter().map(|e| e.value().clone()).collect() }
    pub fn delete(&self, id: &str) -> bool { self.flows.remove(id).is_some() }

    pub fn set_active(&self, id: &str, active: bool) -> Result<(), MtwError> {
        let mut f = self.flows.get_mut(id).ok_or_else(|| MtwError::Agent(format!("flow not found: {}", id)))?;
        f.active = active; Ok(())
    }
}

impl Default for FlowManager { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_flow_crud() {
        let mgr = FlowManager::new();
        let f = mgr.create("Test Flow", "A test");
        assert_eq!(mgr.list().len(), 1);
        assert!(mgr.get(&f.id).is_some());
        mgr.set_active(&f.id, false).unwrap();
        assert!(!mgr.get(&f.id).unwrap().active);
        mgr.delete(&f.id);
        assert_eq!(mgr.list().len(), 0);
    }
}
