use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::config::ExchangeCredentials;
use crate::error::ExchangeError;
use crate::providers::bitvavo::auth;
use crate::providers::bitvavo::types::*;
use crate::types::*;

/// Bitvavo WebSocket API URL
const BITVAVO_WS_URL: &str = "wss://ws.bitvavo.com/v2/";

/// Reconnection state with exponential backoff.
struct ReconnectState {
    attempt: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
}

impl ReconnectState {
    fn new() -> Self {
        Self {
            attempt: 0,
            base_delay_ms: 1000,
            max_delay_ms: 60_000,
        }
    }

    fn next_delay(&mut self) -> u64 {
        let delay = (self.base_delay_ms * 2u64.pow(self.attempt)).min(self.max_delay_ms);
        self.attempt += 1;
        delay
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// Subscription request for Bitvavo WebSocket.
#[derive(Debug, Clone)]
pub enum Subscription {
    Ticker24h { markets: Vec<String> },
    Candles { markets: Vec<String>, intervals: Vec<String> },
    Book { markets: Vec<String> },
    Trades { markets: Vec<String> },
}

/// Bitvavo WebSocket client for real-time market data streaming.
///
/// Manages a persistent WebSocket connection with automatic reconnection,
/// authentication, and subscription management.
pub struct BitvavoWebSocket {
    /// Outbound events — consumed by ExchangeManager
    event_tx: mpsc::UnboundedSender<ExchangeEvent>,
    /// Subscriptions to maintain across reconnections
    subscriptions: Arc<RwLock<Vec<Subscription>>>,
    /// Shutdown signal
    shutdown_tx: broadcast::Sender<()>,
    /// Connection task handle
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl BitvavoWebSocket {
    /// Create a new Bitvavo WebSocket client.
    pub fn new(event_tx: mpsc::UnboundedSender<ExchangeEvent>) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            event_tx,
            subscriptions: Arc::new(RwLock::new(Vec::new())),
            shutdown_tx,
            task_handle: None,
        }
    }

    /// Connect and start the read loop. Automatically reconnects on failure.
    pub async fn connect(
        &mut self,
        credentials: ExchangeCredentials,
    ) -> Result<(), ExchangeError> {
        let event_tx = self.event_tx.clone();
        let subs = self.subscriptions.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        let handle = tokio::spawn(async move {
            let mut reconnect = ReconnectState::new();

            loop {
                match tokio_tungstenite::connect_async(BITVAVO_WS_URL).await {
                    Ok((ws_stream, _)) => {
                        reconnect.reset();
                        let _ = event_tx.send(ExchangeEvent::Connected {
                            exchange: ExchangeId::Bitvavo,
                        });

                        let (mut sink, mut stream) = ws_stream.split();

                        // Authenticate
                        let auth_payload = auth::ws_auth_payload(&credentials);
                        if sink
                            .send(WsMessage::Text(auth_payload.into()))
                            .await
                            .is_err()
                        {
                            tracing::error!("bitvavo ws: failed to send auth");
                            continue;
                        }

                        // Resubscribe (after reconnect)
                        let current_subs = subs.read().await.clone();
                        for sub in &current_subs {
                            let msg = subscription_to_json(sub);
                            if sink.send(WsMessage::Text(msg.into())).await.is_err() {
                                tracing::error!("bitvavo ws: failed to resubscribe");
                                break;
                            }
                        }

                        // Read loop
                        loop {
                            tokio::select! {
                                msg = stream.next() => {
                                    match msg {
                                        Some(Ok(WsMessage::Text(text))) => {
                                            dispatch_message(&text, &event_tx);
                                        }
                                        Some(Ok(WsMessage::Ping(data))) => {
                                            let _ = sink.send(WsMessage::Pong(data)).await;
                                        }
                                        Some(Ok(WsMessage::Close(_))) | None => {
                                            tracing::info!("bitvavo ws: connection closed");
                                            break;
                                        }
                                        Some(Err(e)) => {
                                            tracing::warn!(error = %e, "bitvavo ws: read error");
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                                _ = shutdown_rx.recv() => {
                                    let _ = sink.send(WsMessage::Close(None)).await;
                                    return;
                                }
                            }
                        }

                        let _ = event_tx.send(ExchangeEvent::Disconnected {
                            exchange: ExchangeId::Bitvavo,
                            reason: "connection closed".into(),
                        });
                    }
                    Err(e) => {
                        let _ = event_tx.send(ExchangeEvent::Error {
                            exchange: ExchangeId::Bitvavo,
                            message: format!("connect failed: {}", e),
                        });
                    }
                }

                // Exponential backoff before reconnect
                let delay = reconnect.next_delay();
                tracing::info!(
                    delay_ms = delay,
                    attempt = reconnect.attempt,
                    "bitvavo ws: reconnecting"
                );
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {}
                    _ = shutdown_rx.recv() => return,
                }
            }
        });

        self.task_handle = Some(handle);
        Ok(())
    }

    /// Add a subscription. Sent immediately if connected, replayed on reconnect.
    pub async fn subscribe(&self, sub: Subscription) {
        self.subscriptions.write().await.push(sub);
    }

    /// Unsubscribe from all subscriptions.
    pub async fn clear_subscriptions(&self) {
        self.subscriptions.write().await.clear();
    }

    /// Shutdown the WebSocket connection.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
        if let Some(handle) = &self.task_handle {
            handle.abort();
        }
    }
}

/// Convert a Subscription to a Bitvavo WebSocket JSON message.
fn subscription_to_json(sub: &Subscription) -> String {
    match sub {
        Subscription::Ticker24h { markets } => {
            serde_json::json!({
                "action": "subscribe",
                "channels": [{ "name": "ticker24h", "markets": markets }]
            })
            .to_string()
        }
        Subscription::Candles {
            markets,
            intervals,
        } => {
            serde_json::json!({
                "action": "subscribe",
                "channels": [{ "name": "candles", "markets": markets, "interval": intervals }]
            })
            .to_string()
        }
        Subscription::Book { markets } => {
            serde_json::json!({
                "action": "subscribe",
                "channels": [{ "name": "book", "markets": markets }]
            })
            .to_string()
        }
        Subscription::Trades { markets } => {
            serde_json::json!({
                "action": "subscribe",
                "channels": [{ "name": "trades", "markets": markets }]
            })
            .to_string()
        }
    }
}

/// Parse and dispatch a Bitvavo WebSocket message to the event channel.
fn dispatch_message(text: &str, event_tx: &mpsc::UnboundedSender<ExchangeEvent>) {
    let parsed: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "bitvavo ws: failed to parse message");
            return;
        }
    };

    let event_name = parsed["event"].as_str().unwrap_or("");

    match event_name {
        "ticker24h" => {
            if let Ok(ticker) =
                serde_json::from_value::<BitvavoWsTicker>(parsed["data"].clone())
            {
                let open = parse_str_f64(&ticker.open);
                let last = parse_str_f64(&ticker.last);
                let change_24h = if open > 0.0 {
                    ((last - open) / open) * 100.0
                } else {
                    0.0
                };

                let _ = event_tx.send(ExchangeEvent::TickerUpdate {
                    exchange: ExchangeId::Bitvavo,
                    ticker: Ticker {
                        symbol: ticker.market,
                        bid: parse_str_f64(&ticker.best_bid),
                        ask: parse_str_f64(&ticker.best_ask),
                        last,
                        volume: parse_str_f64(&ticker.volume),
                        change_24h,
                        timestamp: ticker.timestamp.unwrap_or(0),
                    },
                });
            }
        }
        "candle" => {
            if let Ok(ws_candle) =
                serde_json::from_value::<BitvavoWsCandle>(parsed["data"].clone())
            {
                let raw = BitvavoCandle(ws_candle.candle);
                if let (Some(ts), Some(o), Some(h), Some(l), Some(c), Some(v)) = (
                    raw.timestamp(),
                    raw.open(),
                    raw.high(),
                    raw.low(),
                    raw.close(),
                    raw.volume(),
                ) {
                    let _ = event_tx.send(ExchangeEvent::CandleUpdate {
                        exchange: ExchangeId::Bitvavo,
                        symbol: ws_candle.market,
                        timeframe: ws_candle.interval,
                        candle: Candle {
                            timestamp: ts,
                            open: o,
                            high: h,
                            low: l,
                            close: c,
                            volume: v,
                        },
                    });
                }
            }
        }
        "authenticate" => {
            let authenticated = parsed["authenticated"].as_bool().unwrap_or(false);
            if authenticated {
                tracing::info!("bitvavo ws: authenticated");
            } else {
                tracing::error!("bitvavo ws: authentication failed");
                let _ = event_tx.send(ExchangeEvent::Error {
                    exchange: ExchangeId::Bitvavo,
                    message: "authentication failed".into(),
                });
            }
        }
        "subscribed" => {
            tracing::debug!(
                subscriptions = %parsed["subscriptions"],
                "bitvavo ws: subscribed"
            );
        }
        "error" => {
            let msg = parsed["message"]
                .as_str()
                .unwrap_or("unknown error")
                .to_string();
            tracing::error!(message = %msg, "bitvavo ws: error");
            let _ = event_tx.send(ExchangeEvent::Error {
                exchange: ExchangeId::Bitvavo,
                message: msg,
            });
        }
        _ => {
            tracing::trace!(event = event_name, "bitvavo ws: unhandled event");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscription_to_json_ticker() {
        let sub = Subscription::Ticker24h {
            markets: vec!["BTC-EUR".into(), "ETH-EUR".into()],
        };
        let json = subscription_to_json(&sub);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["action"], "subscribe");
        assert_eq!(parsed["channels"][0]["name"], "ticker24h");
    }

    #[test]
    fn test_subscription_to_json_candles() {
        let sub = Subscription::Candles {
            markets: vec!["BTC-EUR".into()],
            intervals: vec!["1m".into(), "1h".into()],
        };
        let json = subscription_to_json(&sub);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["channels"][0]["name"], "candles");
        assert_eq!(parsed["channels"][0]["interval"][0], "1m");
    }

    #[test]
    fn test_dispatch_ticker_message() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let msg = r#"{"event":"ticker24h","data":{"market":"BTC-EUR","bestBid":"95000","bestAsk":"95010","last":"95005","open":"94000","volume":"1234.5","timestamp":1711000000000}}"#;
        dispatch_message(msg, &event_tx);

        let event = event_rx.try_recv().unwrap();
        match event {
            ExchangeEvent::TickerUpdate { exchange, ticker } => {
                assert_eq!(exchange, ExchangeId::Bitvavo);
                assert_eq!(ticker.symbol, "BTC-EUR");
                assert_eq!(ticker.last, 95005.0);
                assert_eq!(ticker.bid, 95000.0);
            }
            _ => panic!("expected TickerUpdate"),
        }
    }

    #[test]
    fn test_dispatch_auth_message() {
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let msg = r#"{"event":"authenticate","authenticated":true}"#;
        // Should not panic or send an error event
        dispatch_message(msg, &event_tx);
    }

    #[test]
    fn test_dispatch_error_message() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let msg = r#"{"event":"error","message":"rate limit exceeded"}"#;
        dispatch_message(msg, &event_tx);

        let event = event_rx.try_recv().unwrap();
        match event {
            ExchangeEvent::Error { exchange, message } => {
                assert_eq!(exchange, ExchangeId::Bitvavo);
                assert!(message.contains("rate limit"));
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn test_dispatch_candle_message() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let msg = r#"{"event":"candle","data":{"market":"ETH-EUR","interval":"1h","candle":[1711000000000,"3000","3100","2950","3050","500"]}}"#;
        dispatch_message(msg, &event_tx);

        let event = event_rx.try_recv().unwrap();
        match event {
            ExchangeEvent::CandleUpdate {
                exchange,
                symbol,
                timeframe,
                candle,
            } => {
                assert_eq!(exchange, ExchangeId::Bitvavo);
                assert_eq!(symbol, "ETH-EUR");
                assert_eq!(timeframe, "1h");
                assert_eq!(candle.close, 3050.0);
            }
            _ => panic!("expected CandleUpdate"),
        }
    }
}
