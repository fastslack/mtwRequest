//! `mtw run` — launch an mtw-server process using the current config.

use std::path::Path;
use std::process::Command;

use super::CliResult;

pub fn run(config_path: Option<&str>) -> CliResult {
    let config = config_path.unwrap_or("mtw.toml");
    if !Path::new(config).exists() {
        return Err(format!(
            "no config file found at {config:?} — run `mtw init` first"
        )
        .into());
    }

    let server_bin = resolve_server_binary()?;
    let mut cmd = Command::new(server_bin);
    cmd.arg("--config").arg(config);

    tracing::info!(config = %config, "launching mtw-server");
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("mtw-server exited with {status}").into());
    }
    Ok(())
}

/// Resolve the mtw-server binary. Prefer `$MTW_SERVER`, then a sibling binary
/// next to the current `mtw` executable, then PATH.
fn resolve_server_binary() -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(explicit) = std::env::var("MTW_SERVER") {
        return Ok(explicit);
    }
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let sibling = dir.join(if cfg!(windows) {
                "mtw-server.exe"
            } else {
                "mtw-server"
            });
            if sibling.exists() {
                return Ok(sibling.to_string_lossy().into_owned());
            }
        }
    }
    Ok("mtw-server".to_string())
}
