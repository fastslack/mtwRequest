use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType { Manual, Event, Schedule, Chain, Webhook }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTrigger {
    pub id: String, pub agent_id: String, pub event_name: String,
    pub filter: serde_json::Value, pub cooldown_ms: u64,
    pub last_fired: Option<String>, pub active: bool, pub created_at: String,
}

pub struct TriggerRegistry {
    triggers: DashMap<String, EventTrigger>,
    last_fired_times: DashMap<String, Instant>,
}

impl TriggerRegistry {
    pub fn new() -> Self { Self { triggers: DashMap::new(), last_fired_times: DashMap::new() } }

    pub fn register(&self, trigger: EventTrigger) { self.triggers.insert(trigger.id.clone(), trigger); }
    pub fn unregister(&self, id: &str) -> bool { self.triggers.remove(id).is_some() }

    pub fn get_triggers_for_event(&self, event_name: &str) -> Vec<EventTrigger> {
        self.triggers.iter().filter(|e| e.active && e.event_name == event_name).map(|e| e.value().clone()).collect()
    }

    pub fn can_fire(&self, trigger_id: &str) -> bool {
        let trigger = match self.triggers.get(trigger_id) { Some(t) => t, None => return false };
        if !trigger.active { return false; }
        if let Some(last) = self.last_fired_times.get(trigger_id) {
            last.elapsed() >= Duration::from_millis(trigger.cooldown_ms)
        } else { true }
    }

    pub fn mark_fired(&self, trigger_id: &str) {
        self.last_fired_times.insert(trigger_id.to_string(), Instant::now());
        if let Some(mut t) = self.triggers.get_mut(trigger_id) {
            t.last_fired = Some(format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()));
        }
    }

    pub fn list(&self) -> Vec<EventTrigger> { self.triggers.iter().map(|e| e.value().clone()).collect() }
}

impl Default for TriggerRegistry { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    fn make_trigger(id: &str, event: &str) -> EventTrigger {
        EventTrigger { id: id.into(), agent_id: "a1".into(), event_name: event.into(),
            filter: serde_json::json!({}), cooldown_ms: 1000, last_fired: None, active: true, created_at: "0".into() }
    }
    #[test]
    fn test_trigger_registry() {
        let reg = TriggerRegistry::new();
        reg.register(make_trigger("t1", "task.created"));
        reg.register(make_trigger("t2", "task.created"));
        reg.register(make_trigger("t3", "reminder.fired"));
        assert_eq!(reg.get_triggers_for_event("task.created").len(), 2);
        assert_eq!(reg.get_triggers_for_event("reminder.fired").len(), 1);
    }
    #[test]
    fn test_cooldown() {
        let reg = TriggerRegistry::new();
        reg.register(make_trigger("t1", "evt"));
        assert!(reg.can_fire("t1"));
        reg.mark_fired("t1");
        assert!(!reg.can_fire("t1")); // within cooldown
    }
}
