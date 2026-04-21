//! `mtw install <module>` — download a module tarball from the marketplace.

use std::fs;
use std::path::PathBuf;

use mtw_registry::{RegistryClient, RegistryConfig};

use super::CliResult;

pub async fn run(module: &str) -> CliResult {
    let (name, version) = parse_module(module)?;
    let client = RegistryClient::new(load_config());

    // If no version was given, resolve "latest" by fetching metadata for the
    // wildcard — the backend is expected to treat `latest` specially.
    let resolved_version = if version == "latest" {
        client.get_module(&name, "latest").await?.version
    } else {
        version.to_string()
    };

    tracing::info!(%name, version = %resolved_version, "downloading module");
    let bytes = client.download(&name, &resolved_version).await?;

    let dir = cache_dir().join(format!("{name}-{resolved_version}"));
    fs::create_dir_all(&dir)?;
    let tarball = dir.join(format!("{name}-{resolved_version}.tar.gz"));
    fs::write(&tarball, &bytes)?;

    println!(
        "installed {name}@{resolved_version} ({} bytes) → {}",
        bytes.len(),
        tarball.display()
    );
    Ok(())
}

fn parse_module(spec: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    match spec.split_once('@') {
        Some((n, v)) if !n.is_empty() && !v.is_empty() => Ok((n.to_string(), v.to_string())),
        Some(_) => Err(format!("invalid module spec {spec:?} (expected name@version)").into()),
        None => Ok((spec.to_string(), "latest".to_string())),
    }
}

fn cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("MTW_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(home) = dirs_home() {
        return home.join(".mtw").join("cache");
    }
    PathBuf::from(".mtw-cache")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn load_config() -> RegistryConfig {
    let mut cfg = RegistryConfig::default();
    if let Ok(url) = std::env::var("MTW_REGISTRY_URL") {
        cfg.registry_url = url;
    }
    if let Ok(token) = std::env::var("MTW_REGISTRY_TOKEN") {
        cfg.auth_token = Some(token);
    }
    cfg
}
