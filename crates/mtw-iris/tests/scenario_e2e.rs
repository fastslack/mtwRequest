//! End-to-end: load a fixture, run an incident agent backed by a mock
//! provider, score the run. Exercises the same code path a real agent would
//! take with a real LLM.

use std::sync::Arc;

use mtw_iris::{IncidentAgent, MockProvider, Scenario, ScenarioHarness, Verdict};

const FIXTURE: &str = include_str!("fixtures/rds_pool_exhausted.json");

fn correct_answer() -> &'static str {
    r#"{
        "root_cause": "db_pool_exhausted",
        "rationale": "connection acquire timeouts plus DatabaseConnections ~= max_connections",
        "cited_tags": ["db", "saturation"],
        "required_next": []
    }"#
}

fn red_herring_answer() -> &'static str {
    r#"{
        "root_cause": "cpu_spike",
        "rationale": "cpu looks high",
        "cited_tags": ["cpu"],
        "required_next": []
    }"#
}

#[tokio::test]
async fn scenario_passes_with_correct_diagnosis() {
    let scenario = Scenario::from_json(FIXTURE).expect("fixture must parse");
    let provider =
        Arc::new(MockProvider::new().with_rule("connection from pool", correct_answer()));
    let agent = IncidentAgent::new(provider, "mock-1");
    let run = ScenarioHarness::new(scenario)
        .run(&agent)
        .await
        .expect("run ok");

    assert_eq!(run.score.verdict, Verdict::Pass);
    assert!(run.score.composite >= 0.95);
    assert_eq!(run.hypothesis.root_cause, "db_pool_exhausted");
}

#[tokio::test]
async fn scenario_fails_when_agent_accepts_red_herring() {
    let scenario = Scenario::from_json(FIXTURE).unwrap();
    let provider = Arc::new(MockProvider::new().with_default(red_herring_answer()));
    let agent = IncidentAgent::new(provider, "mock-1");
    let run = ScenarioHarness::new(scenario).run(&agent).await.unwrap();

    assert_eq!(run.score.verdict, Verdict::Fail);
    assert_eq!(run.score.red_herring_resistance, 0.0);
}
