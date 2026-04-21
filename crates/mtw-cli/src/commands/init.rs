//! `mtw init` — drop an mtw.toml into the current directory.

use std::fs;
use std::path::Path;

use super::CliResult;

pub fn run(force: bool) -> CliResult {
    let target = Path::new("mtw.toml");
    if target.exists() && !force {
        return Err("mtw.toml already exists (use --force to overwrite)".into());
    }

    fs::write(target, super::new::DEFAULT_CONFIG)?;
    println!("wrote mtw.toml");
    Ok(())
}
