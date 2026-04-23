//! NATS client using `async-nats`. Maps channel → NATS subject directly.
//! Single-subject per client (all bench scenarios use one channel).

use crate::{BenchClient, BenchMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;

pub struct NatsClient {
    client: Option<async_nats::Client>,
    sub: Option<async_nats::Subscriber>,
}

impl NatsClient {
    pub fn new() -> Self {
        Self {
            client: None,
            sub: None,
        }
    }
}

impl Default for NatsClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BenchClient for NatsClient {
    async fn connect(&mut self, url: &str) -> Result<()> {
        let client = async_nats::connect(url).await.context("nats connect")?;
        self.client = Some(client);
        Ok(())
    }

    async fn subscribe(&mut self, channel: &str) -> Result<()> {
        let client = self.client.as_ref().context("not connected")?;
        let sub = client
            .subscribe(channel.to_string())
            .await
            .context("nats subscribe")?;
        self.sub = Some(sub);
        Ok(())
    }

    async fn publish(&mut self, channel: &str, payload: Bytes) -> Result<()> {
        let client = self.client.as_ref().context("not connected")?;
        client
            .publish(channel.to_string(), payload)
            .await
            .context("nats publish")?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<BenchMessage> {
        let sub = self.sub.as_mut().context("not subscribed")?;
        let msg = sub.next().await.context("nats subscription closed")?;
        Ok(BenchMessage {
            channel: msg.subject.to_string(),
            payload: msg.payload,
        })
    }

    async fn close(&mut self) -> Result<()> {
        if let Some(mut sub) = self.sub.take() {
            let _ = sub.unsubscribe().await;
        }
        if let Some(client) = self.client.take() {
            let _ = client.drain().await;
        }
        Ok(())
    }
}
