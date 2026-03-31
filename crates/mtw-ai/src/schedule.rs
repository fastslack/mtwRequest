use dashmap::DashMap;
use mtw_core::MtwError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSchedule {
    pub id: String, pub agent_id: String, pub interval_ms: u64,
    pub cron_expression: String, pub goal_override: Option<String>,
    pub next_run_at: String, pub last_run_at: Option<String>,
    pub active: bool, pub created_at: String,
}

pub struct ScheduleManager { schedules: DashMap<String, AgentSchedule> }

impl ScheduleManager {
    pub fn new() -> Self { Self { schedules: DashMap::new() } }
    pub fn add(&self, schedule: AgentSchedule) { self.schedules.insert(schedule.id.clone(), schedule); }
    pub fn remove(&self, id: &str) -> bool { self.schedules.remove(id).is_some() }
    pub fn list(&self) -> Vec<AgentSchedule> { self.schedules.iter().map(|e| e.value().clone()).collect() }

    pub fn list_due(&self, now: &str) -> Vec<AgentSchedule> {
        self.schedules.iter().filter(|e| e.active && e.next_run_at.as_str() <= now).map(|e| e.value().clone()).collect()
    }

    pub fn mark_run(&self, id: &str) -> Result<(), MtwError> {
        let mut s = self.schedules.get_mut(id).ok_or_else(|| MtwError::Agent(format!("schedule not found: {}", id)))?;
        let now_secs: u64 = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        s.last_run_at = Some(now_secs.to_string());
        let next = now_secs + s.interval_ms / 1000;
        s.next_run_at = next.to_string();
        Ok(())
    }

    pub fn set_active(&self, id: &str, active: bool) -> Result<(), MtwError> {
        let mut s = self.schedules.get_mut(id).ok_or_else(|| MtwError::Agent(format!("schedule not found: {}", id)))?;
        s.active = active; Ok(())
    }
}

impl Default for ScheduleManager { fn default() -> Self { Self::new() } }

/// Represents a schedule that is due for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DueSchedule {
    pub schedule_id: String,
    pub agent_id: String,
    pub goal: String,
}

/// Engine that polls the schedule manager on a regular interval
/// and returns schedules that are due for execution.
pub struct SchedulerEngine {
    schedule_manager: Arc<ScheduleManager>,
    /// Poll interval in seconds (default 30).
    pub poll_interval_secs: u64,
}

impl SchedulerEngine {
    /// Create a new scheduler engine with the given manager and poll interval.
    pub fn new(schedule_manager: Arc<ScheduleManager>, poll_interval_secs: u64) -> Self {
        Self {
            schedule_manager,
            poll_interval_secs,
        }
    }

    /// Called each poll interval. Returns schedules that are due (next_run_at <= now).
    ///
    /// For each due schedule, creates a `DueSchedule` with the resolved goal
    /// (from `goal_override` or a default), then marks the schedule as run
    /// which updates its `next_run_at`.
    pub fn tick(&self, now: &str) -> Vec<DueSchedule> {
        let due_schedules = self.schedule_manager.list_due(now);
        let mut result = Vec::new();

        for schedule in due_schedules {
            let goal = schedule
                .goal_override
                .clone()
                .unwrap_or_else(|| format!("Run scheduled task for agent {}", schedule.agent_id));

            result.push(DueSchedule {
                schedule_id: schedule.id.clone(),
                agent_id: schedule.agent_id.clone(),
                goal,
            });

            // Mark as run to update next_run_at
            let _ = self.schedule_manager.mark_run(&schedule.id);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_schedule(id: &str, agent_id: &str, next_run_at: &str) -> AgentSchedule {
        AgentSchedule {
            id: id.into(),
            agent_id: agent_id.into(),
            interval_ms: 60000,
            cron_expression: "".into(),
            goal_override: None,
            next_run_at: next_run_at.into(),
            last_run_at: None,
            active: true,
            created_at: "0".into(),
        }
    }

    #[test]
    fn test_schedule() {
        let mgr = ScheduleManager::new();
        mgr.add(AgentSchedule { id: "s1".into(), agent_id: "a1".into(), interval_ms: 60000,
            cron_expression: "".into(), goal_override: None, next_run_at: "0".into(),
            last_run_at: None, active: true, created_at: "0".into() });
        assert_eq!(mgr.list_due("999999999999").len(), 1);
        mgr.mark_run("s1").unwrap();
        let s = mgr.list()[0].clone();
        assert!(s.last_run_at.is_some());
    }

    #[test]
    fn test_scheduler_engine_tick_returns_due() {
        let mgr = Arc::new(ScheduleManager::new());
        mgr.add(make_schedule("s1", "a1", "100"));
        mgr.add(make_schedule("s2", "a2", "200"));

        let engine = SchedulerEngine::new(mgr, 30);

        // now=150: only s1 is due (next_run_at "100" <= "150")
        let due = engine.tick("150");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].schedule_id, "s1");
        assert_eq!(due[0].agent_id, "a1");
    }

    #[test]
    fn test_scheduler_engine_tick_returns_all_due() {
        let mgr = Arc::new(ScheduleManager::new());
        mgr.add(make_schedule("s1", "a1", "100"));
        mgr.add(make_schedule("s2", "a2", "200"));

        let engine = SchedulerEngine::new(mgr, 30);

        // now=999: both are due
        let due = engine.tick("999");
        assert_eq!(due.len(), 2);
    }

    #[test]
    fn test_scheduler_engine_tick_none_due() {
        let mgr = Arc::new(ScheduleManager::new());
        mgr.add(make_schedule("s1", "a1", "500"));

        let engine = SchedulerEngine::new(mgr, 30);

        // now=100: none due (next_run_at "500" > "100")
        let due = engine.tick("100");
        assert!(due.is_empty());
    }

    #[test]
    fn test_scheduler_engine_tick_updates_next_run() {
        let mgr = Arc::new(ScheduleManager::new());
        mgr.add(make_schedule("s1", "a1", "0"));

        let engine = SchedulerEngine::new(Arc::clone(&mgr), 30);

        let due = engine.tick("999999999999");
        assert_eq!(due.len(), 1);

        // After tick, next_run_at should be updated (no longer "0")
        let schedules = mgr.list();
        assert_ne!(schedules[0].next_run_at, "0");
        assert!(schedules[0].last_run_at.is_some());
    }

    #[test]
    fn test_scheduler_engine_goal_override() {
        let mgr = Arc::new(ScheduleManager::new());
        let mut schedule = make_schedule("s1", "a1", "0");
        schedule.goal_override = Some("Custom goal".into());
        mgr.add(schedule);

        let engine = SchedulerEngine::new(mgr, 30);

        let due = engine.tick("999999999999");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].goal, "Custom goal");
    }

    #[test]
    fn test_scheduler_engine_default_goal() {
        let mgr = Arc::new(ScheduleManager::new());
        mgr.add(make_schedule("s1", "a1", "0"));

        let engine = SchedulerEngine::new(mgr, 30);

        let due = engine.tick("999999999999");
        assert_eq!(due[0].goal, "Run scheduled task for agent a1");
    }

    #[test]
    fn test_scheduler_engine_inactive_schedule_not_due() {
        let mgr = Arc::new(ScheduleManager::new());
        let mut schedule = make_schedule("s1", "a1", "0");
        schedule.active = false;
        mgr.add(schedule);

        let engine = SchedulerEngine::new(mgr, 30);

        let due = engine.tick("999999999999");
        assert!(due.is_empty());
    }
}
