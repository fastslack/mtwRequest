//! # mtw-iris
//!
//! IRIS — Incident Response & Investigation System. An incident-investigation
//! agent, a scored synthetic-scenario harness, and a fault-injection
//! middleware for training and evaluation on top of mtwRequest's real-time
//! core.

pub mod agent;
pub mod evidence;
pub mod fault;
pub mod scenario;
pub mod score;

pub use agent::{IncidentAgent, RootCauseHypothesis};
pub use evidence::{Evidence, EvidenceKind, EvidenceStream};
pub use fault::{FaultInjectorMiddleware, FaultRule};
pub use mtw_test::MockProvider;
pub use scenario::{Scenario, ScenarioHarness, ScenarioRun};
pub use score::{ScenarioScore, Verdict};
