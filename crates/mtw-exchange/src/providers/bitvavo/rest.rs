use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;

use crate::config::ExchangeCredentials;
use crate::error::ExchangeError;
use crate::provider::{ExchangeProvider, ProviderCapabilities};
use crate::providers::bitvavo::auth;
use crate::providers::bitvavo::types::*;
use crate::rate_limit::RateLimiter;
use crate::types::*;

/// Bitvavo REST API base URL
const BITVAVO_REST_URL: &str = "https://api.bitvavo.com/v2";

/// Bitvavo REST API provider implementing `ExchangeProvider`.
///
/// All requests are rate-limited and authenticated via HMAC-SHA256.
/// HTTP connections are pooled via reqwest for efficiency.
pub struct BitvavoProvider {
    id: ExchangeId,
    client: Client,
    credentials: ExchangeCredentials,
    rate_limiter: Arc<dyn RateLimiter>,
    capabilities: ProviderCapabilities,
}

impl BitvavoProvider {
    /// Create a new Bitvavo REST provider.
    pub fn new(credentials: ExchangeCredentials, rate_limiter: Arc<dyn RateLimiter>) -> Self {
        let client = Client::builder()
            .pool_max_idle_per_host(10)
            .timeout(std::time::Duration::from_secs(10))
            .gzip(true)
            .build()
            .expect("failed to build HTTP client");

        Self {
            id: ExchangeId::Bitvavo,
            client,
            credentials,
            rate_limiter,
            capabilities: ProviderCapabilities {
                batch_tickers: true,
                websocket: true,
                order_book_stream: true,
                candle_stream: true,
                native_stop_loss: true,
                native_take_profit: true,
                sandbox: false,
            },
        }
    }

    /// Rate-limited authenticated GET request.
    async fn get(&self, path: &str) -> Result<serde_json::Value, ExchangeError> {
        self.rate_limiter.acquire().await;
        let url = format!("{}{}", BITVAVO_REST_URL, path);
        let (timestamp, signature, api_key) =
            auth::sign(&self.credentials, "GET", path, "");

        let resp = self
            .client
            .get(&url)
            .header("Bitvavo-Access-Key", &api_key)
            .header("Bitvavo-Access-Signature", &signature)
            .header("Bitvavo-Access-Timestamp", &timestamp)
            .send()
            .await
            .map_err(|e| ExchangeError::Http {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;

        let status = resp.status();
        let body = resp.text().await.map_err(|e| ExchangeError::Http {
            exchange: "bitvavo".into(),
            message: e.to_string(),
        })?;

        if !status.is_success() {
            return Err(ExchangeError::Api {
                exchange: "bitvavo".into(),
                code: Some(status.as_u16() as i32),
                message: body,
            });
        }

        serde_json::from_str(&body).map_err(|e| ExchangeError::Deserialization {
            exchange: "bitvavo".into(),
            message: format!("{}: {}", e, &body[..body.len().min(200)]),
        })
    }

    /// Rate-limited authenticated POST request.
    async fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ExchangeError> {
        self.rate_limiter.acquire().await;
        let url = format!("{}{}", BITVAVO_REST_URL, path);
        let body_str = serde_json::to_string(body).unwrap_or_default();
        let (timestamp, signature, api_key) =
            auth::sign(&self.credentials, "POST", path, &body_str);

        let resp = self
            .client
            .post(&url)
            .header("Bitvavo-Access-Key", &api_key)
            .header("Bitvavo-Access-Signature", &signature)
            .header("Bitvavo-Access-Timestamp", &timestamp)
            .header("Content-Type", "application/json")
            .body(body_str)
            .send()
            .await
            .map_err(|e| ExchangeError::Http {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;

        let status = resp.status();
        let resp_body = resp.text().await.map_err(|e| ExchangeError::Http {
            exchange: "bitvavo".into(),
            message: e.to_string(),
        })?;

        if !status.is_success() {
            return Err(ExchangeError::Api {
                exchange: "bitvavo".into(),
                code: Some(status.as_u16() as i32),
                message: resp_body,
            });
        }

        serde_json::from_str(&resp_body).map_err(|e| ExchangeError::Deserialization {
            exchange: "bitvavo".into(),
            message: format!("{}: {}", e, &resp_body[..resp_body.len().min(200)]),
        })
    }

    /// Rate-limited authenticated DELETE request.
    async fn delete(&self, path: &str) -> Result<serde_json::Value, ExchangeError> {
        self.rate_limiter.acquire().await;
        let url = format!("{}{}", BITVAVO_REST_URL, path);
        let (timestamp, signature, api_key) =
            auth::sign(&self.credentials, "DELETE", path, "");

        let resp = self
            .client
            .delete(&url)
            .header("Bitvavo-Access-Key", &api_key)
            .header("Bitvavo-Access-Signature", &signature)
            .header("Bitvavo-Access-Timestamp", &timestamp)
            .send()
            .await
            .map_err(|e| ExchangeError::Http {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;

        let status = resp.status();
        let body = resp.text().await.map_err(|e| ExchangeError::Http {
            exchange: "bitvavo".into(),
            message: e.to_string(),
        })?;

        if !status.is_success() {
            return Err(ExchangeError::Api {
                exchange: "bitvavo".into(),
                code: Some(status.as_u16() as i32),
                message: body,
            });
        }

        serde_json::from_str(&body).map_err(|e| ExchangeError::Deserialization {
            exchange: "bitvavo".into(),
            message: format!("{}: {}", e, &body[..body.len().min(200)]),
        })
    }

    /// Convert Bitvavo ticker response to unified Ticker.
    fn convert_ticker(bt: &BitvavoTicker24h) -> Ticker {
        let open = parse_str_f64(&bt.open);
        let last = parse_str_f64(&bt.last);
        let change_24h = if open > 0.0 {
            ((last - open) / open) * 100.0
        } else {
            0.0
        };

        Ticker {
            symbol: bt.market.clone(),
            bid: parse_str_f64(&bt.bid),
            ask: parse_str_f64(&bt.ask),
            last,
            volume: parse_str_f64(&bt.volume),
            change_24h,
            timestamp: bt.timestamp.unwrap_or(0),
        }
    }
}

#[async_trait]
impl ExchangeProvider for BitvavoProvider {
    fn id(&self) -> &ExchangeId {
        &self.id
    }

    fn name(&self) -> &str {
        "Bitvavo"
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    async fn get_ticker(&self, symbol: &str) -> Result<Ticker, ExchangeError> {
        let path = format!("/ticker24h?market={}", symbol);
        let data = self.get(&path).await?;
        let bt: BitvavoTicker24h =
            serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;
        Ok(Self::convert_ticker(&bt))
    }

    async fn get_tickers(&self, symbols: &[String]) -> Result<Vec<Ticker>, ExchangeError> {
        if symbols.is_empty() {
            // Bitvavo returns all tickers with a single request
            let data = self.get("/ticker24h").await?;
            let tickers: Vec<BitvavoTicker24h> =
                serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                    exchange: "bitvavo".into(),
                    message: e.to_string(),
                })?;
            return Ok(tickers.iter().map(Self::convert_ticker).collect());
        }

        // For specific symbols, use batch endpoint with filter
        // Bitvavo doesn't support multi-market in one call, so we fetch all and filter
        let data = self.get("/ticker24h").await?;
        let all_tickers: Vec<BitvavoTicker24h> =
            serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;

        let symbols_set: std::collections::HashSet<&str> =
            symbols.iter().map(|s| s.as_str()).collect();
        Ok(all_tickers
            .iter()
            .filter(|t| symbols_set.contains(t.market.as_str()))
            .map(Self::convert_ticker)
            .collect())
    }

    async fn get_candles(
        &self,
        symbol: &str,
        timeframe: &str,
        limit: usize,
    ) -> Result<Vec<Candle>, ExchangeError> {
        let path = format!(
            "/{}/candles?interval={}&limit={}",
            symbol, timeframe, limit
        );
        let data = self.get(&path).await?;
        let raw_candles: Vec<BitvavoCandle> =
            serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;

        Ok(raw_candles
            .iter()
            .filter_map(|c| {
                Some(Candle {
                    timestamp: c.timestamp()?,
                    open: c.open()?,
                    high: c.high()?,
                    low: c.low()?,
                    close: c.close()?,
                    volume: c.volume()?,
                })
            })
            .collect())
    }

    async fn get_order_book(
        &self,
        symbol: &str,
        limit: usize,
    ) -> Result<OrderBook, ExchangeError> {
        let path = format!("/{}/book?depth={}", symbol, limit);
        let data = self.get(&path).await?;
        let book: BitvavoOrderBook =
            serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;

        Ok(OrderBook {
            symbol: book.market,
            bids: book
                .bids
                .iter()
                .filter_map(|level| {
                    Some(OrderBookLevel {
                        price: level.first()?.parse().ok()?,
                        quantity: level.get(1)?.parse().ok()?,
                    })
                })
                .collect(),
            asks: book
                .asks
                .iter()
                .filter_map(|level| {
                    Some(OrderBookLevel {
                        price: level.first()?.parse().ok()?,
                        quantity: level.get(1)?.parse().ok()?,
                    })
                })
                .collect(),
            timestamp: book.nonce,
        })
    }

    async fn get_markets(&self) -> Result<Vec<String>, ExchangeError> {
        let data = self.get("/markets").await?;
        let markets: Vec<BitvavoMarket> =
            serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;
        Ok(markets
            .iter()
            .filter(|m| m.status == "trading")
            .map(|m| m.market.clone())
            .collect())
    }

    async fn get_balances(&self) -> Result<Vec<Balance>, ExchangeError> {
        let data = self.get("/balance").await?;
        let balances: Vec<BitvavoBalance> =
            serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;

        Ok(balances
            .iter()
            .map(|b| {
                let available: f64 = b.available.parse().unwrap_or(0.0);
                let in_order: f64 = b.in_order.parse().unwrap_or(0.0);
                Balance {
                    currency: b.symbol.clone(),
                    free: available,
                    used: in_order,
                    total: available + in_order,
                }
            })
            .collect())
    }

    async fn create_order(&self, params: CreateOrderParams) -> Result<Order, ExchangeError> {
        let mut body = serde_json::json!({
            "market": params.symbol,
            "side": params.side,
            "orderType": match params.order_type {
                OrderType::Market => "market",
                OrderType::Limit => "limit",
                OrderType::Stop => "stopLoss",
                OrderType::StopLimit => "stopLossLimit",
            },
            "amount": format!("{}", params.amount),
        });

        if let Some(price) = params.price {
            body["price"] = serde_json::json!(format!("{}", price));
        }
        if let Some(stop_price) = params.stop_price {
            body["triggerPrice"] = serde_json::json!(format!("{}", stop_price));
        }

        let data = self.post("/order", &body).await?;
        let order: BitvavoOrder =
            serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;

        Ok(convert_order(&order))
    }

    async fn cancel_order(&self, order_id: &str, symbol: &str) -> Result<(), ExchangeError> {
        let path = format!("/order?market={}&orderId={}", symbol, order_id);
        self.delete(&path).await?;
        Ok(())
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> Result<Vec<Order>, ExchangeError> {
        let path = match symbol {
            Some(s) => format!("/ordersOpen?market={}", s),
            None => "/ordersOpen".to_string(),
        };
        let data = self.get(&path).await?;
        let orders: Vec<BitvavoOrder> =
            serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;
        Ok(orders.iter().map(convert_order).collect())
    }

    async fn get_order(&self, order_id: &str, symbol: &str) -> Result<Order, ExchangeError> {
        let path = format!("/order?market={}&orderId={}", symbol, order_id);
        let data = self.get(&path).await?;
        let order: BitvavoOrder =
            serde_json::from_value(data).map_err(|e| ExchangeError::Deserialization {
                exchange: "bitvavo".into(),
                message: e.to_string(),
            })?;
        Ok(convert_order(&order))
    }

    async fn shutdown(&self) -> Result<(), ExchangeError> {
        Ok(())
    }
}

/// Convert BitvavoOrder to unified Order.
fn convert_order(bo: &BitvavoOrder) -> Order {
    let filled = parse_str_f64(&bo.filled_amount);
    let filled_price = parse_str_f64(&bo.filled_price);
    let fee_paid = parse_str_f64(&bo.fee_paid);

    Order {
        id: bo.order_id.clone(),
        symbol: bo.market.clone(),
        side: if bo.side == "buy" {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        },
        order_type: match bo.order_type.as_str() {
            "market" => OrderType::Market,
            "limit" => OrderType::Limit,
            "stopLoss" => OrderType::Stop,
            "stopLossLimit" => OrderType::StopLimit,
            _ => OrderType::Market,
        },
        status: match bo.status.as_str() {
            "new" | "open" => OrderStatus::Open,
            "filled" => OrderStatus::Closed,
            "canceled" | "cancelled" => OrderStatus::Canceled,
            "expired" => OrderStatus::Expired,
            "rejected" => OrderStatus::Rejected,
            _ => OrderStatus::Open,
        },
        amount: parse_str_f64(&bo.amount),
        filled,
        average: filled_price,
        cost: filled * filled_price,
        fee: if fee_paid > 0.0 {
            Some(Fee {
                cost: fee_paid,
                currency: bo.fee_currency.clone().unwrap_or_default(),
            })
        } else {
            None
        },
        timestamp: bo.created.unwrap_or(0),
    }
}
