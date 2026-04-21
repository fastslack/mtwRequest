//! `mtw new <name>` — scaffold a new mtwRequest project.

use std::fs;
use std::path::Path;

use super::CliResult;

pub(crate) const DEFAULT_CONFIG: &str = r#"# mtwRequest configuration

[server]
host = "0.0.0.0"
port = 7741
max_connections = 10000

[transport]
default = "websocket"

[transport.websocket]
path = "/ws"
ping_interval = 30
max_message_size = 1048576

[codec]
default = "json"
"#;

const DEFAULT_GITIGNORE: &str = "/target\nCargo.lock\n";

pub fn run(name: &str, no_config: bool) -> CliResult {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return Err(format!("invalid project name: {name:?}").into());
    }

    let root = Path::new(name);
    if root.exists() {
        return Err(format!("directory {name:?} already exists").into());
    }

    fs::create_dir_all(root.join("src"))?;

    let cargo_toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
mtw-core = "0.2"
mtw-protocol = "0.2"
mtw-transport = "0.2"
mtw-router = "0.2"
mtw-codec = "0.2"
tokio = {{ version = "1", features = ["full"] }}
tracing-subscriber = {{ version = "0.3", features = ["env-filter"] }}
"#
    );

    fs::write(root.join("Cargo.toml"), cargo_toml)?;
    fs::write(root.join(".gitignore"), DEFAULT_GITIGNORE)?;
    if !no_config {
        fs::write(root.join("mtw.toml"), DEFAULT_CONFIG)?;
    }

    fs::write(
        root.join("src/main.rs"),
        r#"//! mtwRequest starter

use mtw_transport::ws::WebSocketTransport;
use mtw_transport::MtwTransport;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("info,mtw=debug")
        .init();

    let mut transport = WebSocketTransport::new("/ws", 30);
    let mut events = transport.take_event_receiver().unwrap();
    transport.listen("0.0.0.0:7741".parse()?).await?;

    tracing::info!("listening on ws://0.0.0.0:7741/ws");

    while let Some(event) = events.recv().await {
        tracing::info!(?event);
    }
    Ok(())
}
"#,
    )?;

    println!("created project {name}/");
    println!("  cd {name}");
    println!("  cargo run");
    Ok(())
}
