use crate::trigger::TriggerRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Represents a trigger that has been evaluated and should fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiredTrigger {
    pub trigger_id: String,
    pub agent_id: String,
    pub event_name: String,
    pub payload: Value,
    pub goal_override: Option<String>,
}

/// Reactive engine that listens for events and fires agent triggers.
///
/// Evaluates incoming events against registered triggers, enforcing
/// cooldown periods and concurrency limits before allowing triggers to fire.
pub struct ReactiveEngine {
    trigger_registry: Arc<TriggerRegistry>,
    max_concurrent: usize,
    active_count: Arc<AtomicU32>,
}

impl ReactiveEngine {
    /// Create a new reactive engine with the given trigger registry and concurrency limit.
    pub fn new(trigger_registry: Arc<TriggerRegistry>, max_concurrent: usize) -> Self {
        Self {
            trigger_registry,
            max_concurrent,
            active_count: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Process an incoming event and return the list of triggers that should fire.
    ///
    /// For each trigger registered for this event:
    /// - Checks cooldown via `can_fire()`
    /// - Checks filter match (shallow key-value comparison)
    /// - Checks active count < max_concurrent
    /// - If all pass: marks fired, increments active count, adds to result
    pub fn on_event(&self, event_name: &str, payload: Value) -> Vec<FiredTrigger> {
        let triggers = self.trigger_registry.get_triggers_for_event(event_name);
        let mut fired = Vec::new();

        for trigger in triggers {
            // Check cooldown
            if !self.trigger_registry.can_fire(&trigger.id) {
                continue;
            }

            // Check filter match
            if !Self::filter_matches(&trigger.filter, &payload) {
                continue;
            }

            // Check concurrency limit
            let current = self.active_count.load(Ordering::SeqCst);
            if current >= self.max_concurrent as u32 {
                break;
            }

            // All checks passed: fire this trigger
            self.trigger_registry.mark_fired(&trigger.id);
            self.active_count.fetch_add(1, Ordering::SeqCst);

            fired.push(FiredTrigger {
                trigger_id: trigger.id.clone(),
                agent_id: trigger.agent_id.clone(),
                event_name: event_name.to_string(),
                payload: payload.clone(),
                goal_override: None,
            });
        }

        fired
    }

    /// Mark a fired trigger as completed, decrementing the active count.
    pub fn mark_completed(&self) {
        let prev = self.active_count.load(Ordering::SeqCst);
        if prev > 0 {
            self.active_count.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Returns the current number of active (in-flight) triggers.
    pub fn active_count(&self) -> u32 {
        self.active_count.load(Ordering::SeqCst)
    }

    /// Check if a JSON filter matches a payload.
    ///
    /// The filter is a JSON object with key-value pairs. ALL keys in the filter
    /// must exist in the payload with matching values. An empty filter or
    /// non-object filter always matches.
    pub fn filter_matches(filter: &Value, payload: &Value) -> bool {
        let filter_obj = match filter.as_object() {
            Some(obj) => obj,
            None => return true, // non-object filter always matches
        };

        if filter_obj.is_empty() {
            return true;
        }

        let payload_obj = match payload.as_object() {
            Some(obj) => obj,
            None => return false, // filter has keys but payload is not an object
        };

        for (key, filter_val) in filter_obj {
            match payload_obj.get(key) {
                Some(payload_val) if payload_val == filter_val => {}
                _ => return false,
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger::EventTrigger;

    fn make_trigger(id: &str, event: &str) -> EventTrigger {
        EventTrigger {
            id: id.into(),
            agent_id: "agent-1".into(),
            event_name: event.into(),
            filter: serde_json::json!({}),
            cooldown_ms: 1000,
            last_fired: None,
            active: true,
            created_at: "0".into(),
        }
    }

    fn make_trigger_with_filter(id: &str, event: &str, filter: Value) -> EventTrigger {
        EventTrigger {
            id: id.into(),
            agent_id: "agent-1".into(),
            event_name: event.into(),
            filter,
            cooldown_ms: 1000,
            last_fired: None,
            active: true,
            created_at: "0".into(),
        }
    }

    // --- filter_matches tests ---

    #[test]
    fn test_empty_filter_always_matches() {
        let filter = serde_json::json!({});
        let payload = serde_json::json!({"key": "value"});
        assert!(ReactiveEngine::filter_matches(&filter, &payload));
    }

    #[test]
    fn test_null_filter_always_matches() {
        let filter = Value::Null;
        let payload = serde_json::json!({"key": "value"});
        assert!(ReactiveEngine::filter_matches(&filter, &payload));
    }

    #[test]
    fn test_filter_matches_exact_key_value() {
        let filter = serde_json::json!({"status": "open"});
        let payload = serde_json::json!({"status": "open", "priority": "high"});
        assert!(ReactiveEngine::filter_matches(&filter, &payload));
    }

    #[test]
    fn test_filter_no_match_different_value() {
        let filter = serde_json::json!({"status": "closed"});
        let payload = serde_json::json!({"status": "open"});
        assert!(!ReactiveEngine::filter_matches(&filter, &payload));
    }

    #[test]
    fn test_filter_no_match_missing_key() {
        let filter = serde_json::json!({"missing_key": "value"});
        let payload = serde_json::json!({"other_key": "value"});
        assert!(!ReactiveEngine::filter_matches(&filter, &payload));
    }

    #[test]
    fn test_filter_multiple_keys_all_must_match() {
        let filter = serde_json::json!({"status": "open", "priority": "high"});
        let payload = serde_json::json!({"status": "open", "priority": "high", "extra": true});
        assert!(ReactiveEngine::filter_matches(&filter, &payload));

        let payload_partial = serde_json::json!({"status": "open", "priority": "low"});
        assert!(!ReactiveEngine::filter_matches(&filter, &payload_partial));
    }

    #[test]
    fn test_filter_against_non_object_payload() {
        let filter = serde_json::json!({"key": "val"});
        let payload = serde_json::json!("just a string");
        assert!(!ReactiveEngine::filter_matches(&filter, &payload));
    }

    // --- on_event tests ---

    #[test]
    fn test_on_event_fires_matching_trigger() {
        let registry = Arc::new(TriggerRegistry::new());
        registry.register(make_trigger("t1", "task.created"));
        let engine = ReactiveEngine::new(registry, 3);

        let fired = engine.on_event("task.created", serde_json::json!({"id": 1}));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].trigger_id, "t1");
        assert_eq!(fired[0].agent_id, "agent-1");
        assert_eq!(fired[0].event_name, "task.created");
    }

    #[test]
    fn test_on_event_no_match_for_different_event() {
        let registry = Arc::new(TriggerRegistry::new());
        registry.register(make_trigger("t1", "task.created"));
        let engine = ReactiveEngine::new(registry, 3);

        let fired = engine.on_event("task.deleted", serde_json::json!({}));
        assert!(fired.is_empty());
    }

    #[test]
    fn test_on_event_respects_filter() {
        let registry = Arc::new(TriggerRegistry::new());
        registry.register(make_trigger_with_filter(
            "t1",
            "task.created",
            serde_json::json!({"priority": "high"}),
        ));
        let engine = ReactiveEngine::new(registry, 3);

        // Should not fire: filter doesn't match
        let fired = engine.on_event("task.created", serde_json::json!({"priority": "low"}));
        assert!(fired.is_empty());

        // Should fire: filter matches
        let fired = engine.on_event("task.created", serde_json::json!({"priority": "high"}));
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn test_on_event_respects_cooldown() {
        let registry = Arc::new(TriggerRegistry::new());
        let mut trigger = make_trigger("t1", "evt");
        trigger.cooldown_ms = 60_000; // 60 second cooldown
        registry.register(trigger);
        let engine = ReactiveEngine::new(registry, 3);

        // First call fires
        let fired = engine.on_event("evt", serde_json::json!({}));
        assert_eq!(fired.len(), 1);
        engine.mark_completed();

        // Second call within cooldown should not fire
        let fired = engine.on_event("evt", serde_json::json!({}));
        assert!(fired.is_empty());
    }

    #[test]
    fn test_on_event_respects_concurrency_limit() {
        let registry = Arc::new(TriggerRegistry::new());
        // Use zero cooldown so repeated fires work
        let mut t1 = make_trigger("t1", "evt");
        t1.cooldown_ms = 0;
        let mut t2 = make_trigger("t2", "evt");
        t2.cooldown_ms = 0;
        t2.agent_id = "agent-2".into();
        let mut t3 = make_trigger("t3", "evt");
        t3.cooldown_ms = 0;
        t3.agent_id = "agent-3".into();
        let mut t4 = make_trigger("t4", "evt");
        t4.cooldown_ms = 0;
        t4.agent_id = "agent-4".into();

        registry.register(t1);
        registry.register(t2);
        registry.register(t3);
        registry.register(t4);

        let engine = ReactiveEngine::new(registry, 2); // max 2 concurrent

        let fired = engine.on_event("evt", serde_json::json!({}));
        assert_eq!(fired.len(), 2);
        assert_eq!(engine.active_count(), 2);

        // Complete one, then fire again
        engine.mark_completed();
        assert_eq!(engine.active_count(), 1);
    }

    #[test]
    fn test_mark_completed_does_not_underflow() {
        let registry = Arc::new(TriggerRegistry::new());
        let engine = ReactiveEngine::new(registry, 3);

        // Should not underflow below 0
        engine.mark_completed();
        assert_eq!(engine.active_count(), 0);
    }

    #[test]
    fn test_on_event_multiple_triggers_same_event() {
        let registry = Arc::new(TriggerRegistry::new());
        let mut t1 = make_trigger("t1", "evt");
        t1.cooldown_ms = 0;
        let mut t2 = make_trigger("t2", "evt");
        t2.cooldown_ms = 0;
        t2.agent_id = "agent-2".into();
        registry.register(t1);
        registry.register(t2);

        let engine = ReactiveEngine::new(registry, 10);
        let fired = engine.on_event("evt", serde_json::json!({}));
        assert_eq!(fired.len(), 2);
    }
}
