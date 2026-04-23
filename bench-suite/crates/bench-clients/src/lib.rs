//! Unified pub/sub client abstraction over mtwRequest, Centrifugo, Socket.IO,
//! and NATS so the bench-runner can exercise each system with the same code.
//!
//! Semantics:
//! - `subscribe`: register interest in a named channel/subject/room.
//! - `publish`: send a message to a channel (broadcast semantics: the server
//!   is expected to deliver it to every subscriber *except* the publisher for
//!   mtw/centrifugo. NATS/Socket.IO deliver to self as well — the runner
//!   accounts for this per-system.)
//! - `recv`: await the next message on any subscribed channel. Must skip
//!   protocol-level acks/responses.

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;

pub mod centrifugo;
pub mod mtw;
pub mod nats;
pub mod socketio;

#[derive(Debug, Clone)]
pub struct BenchMessage {
    pub channel: String,
    pub payload: Bytes,
}

#[async_trait]
pub trait BenchClient: Send {
    async fn connect(&mut self, url: &str) -> Result<()>;
    async fn subscribe(&mut self, channel: &str) -> Result<()>;
    async fn publish(&mut self, channel: &str, payload: Bytes) -> Result<()>;
    async fn recv(&mut self) -> Result<BenchMessage>;
    async fn close(&mut self) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    Mtw,
    Centrifugo,
    Socketio,
    Nats,
}

impl System {
    pub fn as_str(&self) -> &'static str {
        match self {
            System::Mtw => "mtw",
            System::Centrifugo => "centrifugo",
            System::Socketio => "socketio",
            System::Nats => "nats",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "mtw" | "mtwrequest" => Ok(System::Mtw),
            "centrifugo" => Ok(System::Centrifugo),
            "socketio" | "socket.io" => Ok(System::Socketio),
            "nats" => Ok(System::Nats),
            other => anyhow::bail!("unknown system: {other}"),
        }
    }

    pub fn default_url(&self) -> &'static str {
        match self {
            System::Mtw => "ws://127.0.0.1:7741/ws",
            System::Centrifugo => "ws://127.0.0.1:8000/connection/websocket",
            System::Socketio => "http://127.0.0.1:13000",
            System::Nats => "nats://127.0.0.1:4222",
        }
    }

    /// Whether the publisher also receives its own messages (self-delivery).
    /// The fanout scenario subtracts these from subscriber receive counts.
    pub fn self_delivers(&self) -> bool {
        match self {
            // mtw's Channel::publish has an `exclude` parameter (the publisher
            // is excluded server-side).
            System::Mtw => false,
            // Centrifugo: a publisher that is also subscribed receives its own
            // messages by default.
            System::Centrifugo => true,
            // NATS: same-conn publishes are delivered back on matching subs.
            System::Nats => true,
            // Socket.IO: our bench server broadcasts to everyone in the room
            // including the publisher, for parity with NATS/Centrifugo.
            System::Socketio => true,
        }
    }

    pub fn new_client(&self) -> Box<dyn BenchClient> {
        match self {
            System::Mtw => Box::new(mtw::MtwClient::new()),
            System::Centrifugo => Box::new(centrifugo::CentrifugoClient::new()),
            System::Socketio => Box::new(socketio::SocketIoClient::new()),
            System::Nats => Box::new(nats::NatsClient::new()),
        }
    }
}

impl std::fmt::Display for System {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
