//! IRIS demo — runs an incident investigation against a fixture scenario
//! using the mock provider, prints the scored verdict, and then reruns with
//! a provider that falls for the red herring so you can see both outcomes.
//!
//! Run:
//!   cargo run -j 8 -p mtw-examples --bin iris-demo

use std::sync::Arc;

use mtw_iris::{IncidentAgent, MockProvider, Scenario, ScenarioHarness, ScenarioRun};

const FIXTURE: &str = include_str!("../crates/mtw-iris/tests/fixtures/rds_pool_exhausted.json");

const CORRECT: &str = r#"{
  "root_cause": "db_pool_exhausted",
  "rationale": "connection acquire timeouts plus DatabaseConnections ~= max_connections",
  "cited_tags": ["db", "saturation"],
  "required_next": []
}"#;

const RED_HERRING: &str = r#"{
  "root_cause": "cpu_spike",
  "rationale": "cpu looks high",
  "cited_tags": ["cpu"],
  "required_next": []
}"#;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scenario: Scenario = Scenario::from_json(FIXTURE)?;
    println!("scenario: {} — {}", scenario.id, scenario.title);
    println!("evidence pieces: {}\n", scenario.evidence.len());

    print_run(
        "[1] provider that reads the evidence correctly",
        run_with_provider(&scenario, CORRECT, Some("connection from pool")).await?,
    );

    print_run(
        "[2] provider that accepts the red herring",
        run_with_provider(&scenario, RED_HERRING, None).await?,
    );

    Ok(())
}

async fn run_with_provider(
    scenario: &Scenario,
    response: &str,
    rule_keyword: Option<&str>,
) -> Result<ScenarioRun, Box<dyn std::error::Error>> {
    let mut provider = MockProvider::new();
    provider = match rule_keyword {
        Some(k) => provider.with_rule(k, response),
        None => provider.with_default(response),
    };
    let agent = IncidentAgent::new(Arc::new(provider), "mock-1");
    let run = ScenarioHarness::new(scenario.clone()).run(&agent).await?;
    Ok(run)
}

fn print_run(title: &str, run: ScenarioRun) {
    println!("{title}");
    println!("  root cause  : {}", run.hypothesis.root_cause);
    println!("  cited tags  : {:?}", run.hypothesis.cited_tags);
    println!("  verdict     : {:?}", run.score.verdict);
    println!(
        "  composite   : {:.2}  (rc={:.2}, recall={:.2}, resist={:.2})",
        run.score.composite,
        run.score.root_cause_match,
        run.score.evidence_recall,
        run.score.red_herring_resistance,
    );
    if !run.score.notes.is_empty() {
        println!("  notes       : {:?}", run.score.notes);
    }
    println!();
}
