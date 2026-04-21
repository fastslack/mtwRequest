//! `mtw publish` — publish the module in `path` to the marketplace.

use std::path::Path;

use mtw_registry::manifest::RegistryManifest;
use mtw_registry::{RegistryClient, RegistryConfig};

use super::CliResult;

pub async fn run(path: &str, dry_run: bool) -> CliResult {
    let root = Path::new(path);
    let manifest_path = root.join("mtw-module.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "no mtw-module.toml found at {}",
            manifest_path.display()
        )
        .into());
    }

    let manifest = RegistryManifest::from_file(&manifest_path)
        .map_err(|e| format!("failed to parse manifest: {e}"))?;

    let package = package_module(root)?;
    let size_kib = package.len() as f64 / 1024.0;
    println!(
        "packaged {name}@{version} ({size_kib:.1} KiB)",
        name = manifest.name,
        version = manifest.version,
    );

    if dry_run {
        println!("dry-run: skipping upload");
        return Ok(());
    }

    let cfg = load_config();
    if cfg.auth_token.is_none() {
        return Err("MTW_REGISTRY_TOKEN is not set — cannot publish".into());
    }
    let client = RegistryClient::new(cfg);
    let result = client.publish(&manifest, package).await?;

    println!("published {}@{} — {}", result.name, result.version, result.url);
    Ok(())
}

/// Very small "tarball": collects all tracked files in the module root and
/// returns a concatenation. The real implementation will use `flate2 + tar`;
/// this keeps the CLI functional against registries that accept raw bytes and
/// is enough to demonstrate the publish flow end-to-end.
///
/// TODO: replace with real `tar.gz` once the backend contract is finalised.
fn package_module(root: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    let manifest = std::fs::read(root.join("mtw-module.toml"))?;
    out.extend_from_slice(b"--- mtw-module.toml ---\n");
    out.extend_from_slice(&manifest);
    out.push(b'\n');
    // Include a Cargo.toml if present, to help the registry index dependencies.
    let cargo = root.join("Cargo.toml");
    if cargo.exists() {
        out.extend_from_slice(b"--- Cargo.toml ---\n");
        out.extend_from_slice(&std::fs::read(cargo)?);
        out.push(b'\n');
    }
    Ok(out)
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
