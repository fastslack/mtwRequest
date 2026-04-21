//! `mtw search <query>` — search the marketplace.

use mtw_registry::{RegistryClient, RegistryConfig, SearchFilters};

use super::CliResult;

pub async fn run(
    query: &str,
    module_type: Option<&str>,
    author: Option<&str>,
) -> CliResult {
    let config = registry_config();
    let client = RegistryClient::new(config);
    let filters = SearchFilters {
        module_type: module_type.map(str::to_string),
        author: author.map(str::to_string),
        keyword: None,
    };

    let results = client.search(query, &filters).await?;
    if results.is_empty() {
        println!("no modules matched {query:?}");
        return Ok(());
    }

    println!("{:<28}  {:<10}  {:<8}  {}", "NAME", "VERSION", "⇩", "DESCRIPTION");
    for m in results {
        println!(
            "{:<28}  {:<10}  {:<8}  {}",
            truncate(&m.name, 28),
            truncate(&m.version, 10),
            m.downloads,
            truncate(&m.description, 50),
        );
    }
    Ok(())
}

fn registry_config() -> RegistryConfig {
    let mut cfg = RegistryConfig::default();
    if let Ok(url) = std::env::var("MTW_REGISTRY_URL") {
        cfg.registry_url = url;
    }
    if let Ok(token) = std::env::var("MTW_REGISTRY_TOKEN") {
        cfg.auth_token = Some(token);
    }
    cfg
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
