use std::collections::HashMap;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use crate::types::OrderSide;

/// Reason a position was closed by the monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    StopLoss,
    TakeProfit,
    TrailingStop,
    BreakEvenStop,
    BearExit,
    Manual,
}

/// Signal emitted when a position should be closed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseSignal {
    pub trade_id: String,
    pub reason: CloseReason,
    pub trigger_price: f64,
    pub pnl_estimate: f64,
}

/// A position being tracked by the monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoredPosition {
    pub trade_id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub entry_price: f64,
    pub amount: f64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub trailing_stop_pct: Option<f64>,
    pub highest_price: f64,
    pub lowest_price: f64,
    pub opened_at: String,
}

/// Real-time SL/TP enforcement engine.
///
/// Tracks open positions and checks each price tick against
/// stop-loss, take-profit, trailing-stop, and break-even rules.
pub struct TradeMonitor {
    positions: DashMap<String, MonitoredPosition>,
    /// Percentage gain above entry at which SL is moved to entry (break-even).
    breakeven_trigger_pct: f64,
}

impl TradeMonitor {
    /// Create a new monitor with a default break-even trigger of 1.5%.
    pub fn new() -> Self {
        Self {
            positions: DashMap::new(),
            breakeven_trigger_pct: 1.5,
        }
    }

    /// Create a monitor with a custom break-even trigger percentage.
    pub fn with_breakeven_trigger(mut self, pct: f64) -> Self {
        self.breakeven_trigger_pct = pct;
        self
    }

    /// Register a position for monitoring.
    pub fn add_position(&self, position: MonitoredPosition) {
        self.positions.insert(position.trade_id.clone(), position);
    }

    /// Stop monitoring a position.
    pub fn remove_position(&self, trade_id: &str) -> Option<MonitoredPosition> {
        self.positions.remove(trade_id).map(|(_, p)| p)
    }

    /// Update highest/lowest price for all positions matching the symbol.
    pub fn update_price(&self, symbol: &str, price: f64) {
        for mut entry in self.positions.iter_mut() {
            if entry.symbol == symbol {
                if price > entry.highest_price {
                    entry.highest_price = price;
                }
                if price < entry.lowest_price {
                    entry.lowest_price = price;
                }
            }
        }
    }

    /// Core SL/TP enforcement logic for a single position.
    ///
    /// Returns a `CloseSignal` if the position should be closed at the given price.
    pub fn check_position(&self, trade_id: &str, current_price: f64) -> Option<CloseSignal> {
        let mut pos = self.positions.get_mut(trade_id)?;

        // Update extremes
        if current_price > pos.highest_price {
            pos.highest_price = current_price;
        }
        if current_price < pos.lowest_price {
            pos.lowest_price = current_price;
        }

        match pos.side {
            OrderSide::Buy => self.check_long(&pos, current_price),
            OrderSide::Sell => self.check_short(&pos, current_price),
        }
    }

    /// Check all positions against the provided price map.
    pub fn check_all(&self, prices: &HashMap<String, f64>) -> Vec<CloseSignal> {
        // First update all prices
        for (symbol, &price) in prices {
            self.update_price(symbol, price);
        }

        let mut signals = Vec::new();
        let trade_ids: Vec<String> = self.positions.iter().map(|e| e.key().clone()).collect();

        for trade_id in trade_ids {
            if let Some(pos) = self.positions.get(&trade_id) {
                if let Some(&price) = prices.get(&pos.symbol) {
                    drop(pos); // release read lock before check_position takes write lock
                    if let Some(signal) = self.check_position(&trade_id, price) {
                        signals.push(signal);
                    }
                }
            }
        }

        signals
    }

    /// Calculate PnL for a trade.
    pub fn calculate_pnl(
        entry_price: f64,
        exit_price: f64,
        amount: f64,
        side: OrderSide,
        fee_rate: f64,
    ) -> f64 {
        let gross = match side {
            OrderSide::Buy => (exit_price - entry_price) * amount,
            OrderSide::Sell => (entry_price - exit_price) * amount,
        };
        let fee_cost = (entry_price * amount + exit_price * amount) * fee_rate;
        gross - fee_cost
    }

    /// Get a clone of a monitored position.
    pub fn get_position(&self, trade_id: &str) -> Option<MonitoredPosition> {
        self.positions.get(trade_id).map(|p| p.clone())
    }

    /// List all monitored positions.
    pub fn list_positions(&self) -> Vec<MonitoredPosition> {
        self.positions.iter().map(|e| e.value().clone()).collect()
    }

    /// Number of positions being monitored.
    pub fn position_count(&self) -> usize {
        self.positions.len()
    }

    // ── Private helpers ──

    fn check_long(&self, pos: &MonitoredPosition, price: f64) -> Option<CloseSignal> {
        let pnl_no_fee = (price - pos.entry_price) * pos.amount;

        // 1. Stop loss
        if let Some(sl) = pos.stop_loss {
            if price <= sl {
                return Some(CloseSignal {
                    trade_id: pos.trade_id.clone(),
                    reason: CloseReason::StopLoss,
                    trigger_price: price,
                    pnl_estimate: pnl_no_fee,
                });
            }
        }

        // 2. Take profit
        if let Some(tp) = pos.take_profit {
            if price >= tp {
                return Some(CloseSignal {
                    trade_id: pos.trade_id.clone(),
                    reason: CloseReason::TakeProfit,
                    trigger_price: price,
                    pnl_estimate: pnl_no_fee,
                });
            }
        }

        // 3. Trailing stop
        if let Some(trail_pct) = pos.trailing_stop_pct {
            if trail_pct > 0.0 {
                let trailing_sl = pos.highest_price * (1.0 - trail_pct / 100.0);
                if price <= trailing_sl {
                    return Some(CloseSignal {
                        trade_id: pos.trade_id.clone(),
                        reason: CloseReason::TrailingStop,
                        trigger_price: price,
                        pnl_estimate: pnl_no_fee,
                    });
                }
            }
        }

        // 4. Break-even: if gain% > trigger and SL is still below entry, move SL to entry
        //    (This is informational -- we emit BreakEvenStop if price hits entry from above
        //     after having been in profit territory.)
        if self.breakeven_trigger_pct > 0.0 {
            let gain_pct = (pos.highest_price - pos.entry_price) / pos.entry_price * 100.0;
            if gain_pct >= self.breakeven_trigger_pct {
                if let Some(sl) = pos.stop_loss {
                    if sl < pos.entry_price && price <= pos.entry_price {
                        return Some(CloseSignal {
                            trade_id: pos.trade_id.clone(),
                            reason: CloseReason::BreakEvenStop,
                            trigger_price: price,
                            pnl_estimate: (price - pos.entry_price) * pos.amount,
                        });
                    }
                }
            }
        }

        None
    }

    fn check_short(&self, pos: &MonitoredPosition, price: f64) -> Option<CloseSignal> {
        let pnl_no_fee = (pos.entry_price - price) * pos.amount;

        // 1. Stop loss (short: price goes UP past SL)
        if let Some(sl) = pos.stop_loss {
            if price >= sl {
                return Some(CloseSignal {
                    trade_id: pos.trade_id.clone(),
                    reason: CloseReason::StopLoss,
                    trigger_price: price,
                    pnl_estimate: pnl_no_fee,
                });
            }
        }

        // 2. Take profit (short: price goes DOWN past TP)
        if let Some(tp) = pos.take_profit {
            if price <= tp {
                return Some(CloseSignal {
                    trade_id: pos.trade_id.clone(),
                    reason: CloseReason::TakeProfit,
                    trigger_price: price,
                    pnl_estimate: pnl_no_fee,
                });
            }
        }

        // 3. Trailing stop (short: trail from lowest price upward)
        if let Some(trail_pct) = pos.trailing_stop_pct {
            if trail_pct > 0.0 {
                let trailing_sl = pos.lowest_price * (1.0 + trail_pct / 100.0);
                if price >= trailing_sl {
                    return Some(CloseSignal {
                        trade_id: pos.trade_id.clone(),
                        reason: CloseReason::TrailingStop,
                        trigger_price: price,
                        pnl_estimate: pnl_no_fee,
                    });
                }
            }
        }

        // 4. Break-even for short: if gain% > trigger and SL is still above entry
        if self.breakeven_trigger_pct > 0.0 {
            let gain_pct = (pos.entry_price - pos.lowest_price) / pos.entry_price * 100.0;
            if gain_pct >= self.breakeven_trigger_pct {
                if let Some(sl) = pos.stop_loss {
                    if sl > pos.entry_price && price >= pos.entry_price {
                        return Some(CloseSignal {
                            trade_id: pos.trade_id.clone(),
                            reason: CloseReason::BreakEvenStop,
                            trigger_price: price,
                            pnl_estimate: (pos.entry_price - price) * pos.amount,
                        });
                    }
                }
            }
        }

        None
    }
}

impl Default for TradeMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_position(trade_id: &str, entry: f64, sl: Option<f64>, tp: Option<f64>) -> MonitoredPosition {
        MonitoredPosition {
            trade_id: trade_id.to_string(),
            symbol: "BTC/USDT".to_string(),
            side: OrderSide::Buy,
            entry_price: entry,
            amount: 1.0,
            stop_loss: sl,
            take_profit: tp,
            trailing_stop_pct: None,
            highest_price: entry,
            lowest_price: entry,
            opened_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn short_position(trade_id: &str, entry: f64, sl: Option<f64>, tp: Option<f64>) -> MonitoredPosition {
        MonitoredPosition {
            trade_id: trade_id.to_string(),
            symbol: "ETH/USDT".to_string(),
            side: OrderSide::Sell,
            entry_price: entry,
            amount: 2.0,
            stop_loss: sl,
            take_profit: tp,
            trailing_stop_pct: None,
            highest_price: entry,
            lowest_price: entry,
            opened_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_add_remove_position() {
        let monitor = TradeMonitor::new();
        monitor.add_position(long_position("t1", 50000.0, Some(49000.0), Some(55000.0)));
        assert_eq!(monitor.position_count(), 1);
        assert!(monitor.get_position("t1").is_some());
        monitor.remove_position("t1");
        assert_eq!(monitor.position_count(), 0);
    }

    #[test]
    fn test_stop_loss_long() {
        let monitor = TradeMonitor::new();
        monitor.add_position(long_position("t1", 50000.0, Some(49000.0), Some(55000.0)));

        // Price above SL -> no signal
        assert!(monitor.check_position("t1", 50500.0).is_none());

        // Price hits SL
        let sig = monitor.check_position("t1", 48900.0).unwrap();
        assert_eq!(sig.reason, CloseReason::StopLoss);
        assert!(sig.pnl_estimate < 0.0);
    }

    #[test]
    fn test_take_profit_long() {
        let monitor = TradeMonitor::new();
        monitor.add_position(long_position("t1", 50000.0, Some(49000.0), Some(55000.0)));

        let sig = monitor.check_position("t1", 55500.0).unwrap();
        assert_eq!(sig.reason, CloseReason::TakeProfit);
        assert!(sig.pnl_estimate > 0.0);
    }

    #[test]
    fn test_stop_loss_short() {
        let monitor = TradeMonitor::new();
        monitor.add_position(short_position("s1", 3000.0, Some(3100.0), Some(2800.0)));

        // Price goes up past SL
        let sig = monitor.check_position("s1", 3150.0).unwrap();
        assert_eq!(sig.reason, CloseReason::StopLoss);
        assert!(sig.pnl_estimate < 0.0);
    }

    #[test]
    fn test_take_profit_short() {
        let monitor = TradeMonitor::new();
        monitor.add_position(short_position("s1", 3000.0, Some(3100.0), Some(2800.0)));

        let sig = monitor.check_position("s1", 2750.0).unwrap();
        assert_eq!(sig.reason, CloseReason::TakeProfit);
        assert!(sig.pnl_estimate > 0.0);
    }

    #[test]
    fn test_trailing_stop_long() {
        let monitor = TradeMonitor::new();
        let mut pos = long_position("t1", 50000.0, None, None);
        pos.trailing_stop_pct = Some(2.0); // 2% trailing
        monitor.add_position(pos);

        // Price goes up to 52000 -> highest updates
        assert!(monitor.check_position("t1", 52000.0).is_none());
        // Verify highest was updated
        let p = monitor.get_position("t1").unwrap();
        assert_eq!(p.highest_price, 52000.0);

        // Trailing SL = 52000 * (1 - 0.02) = 50960
        // Price drops to 51000 -> still above trailing SL
        assert!(monitor.check_position("t1", 51000.0).is_none());

        // Price drops to 50900 -> below trailing SL
        let sig = monitor.check_position("t1", 50900.0).unwrap();
        assert_eq!(sig.reason, CloseReason::TrailingStop);
    }

    #[test]
    fn test_trailing_stop_moves_up() {
        let monitor = TradeMonitor::new();
        let mut pos = long_position("t1", 100.0, None, None);
        pos.trailing_stop_pct = Some(5.0);
        monitor.add_position(pos);

        // Price goes to 110, trailing SL = 104.5
        assert!(monitor.check_position("t1", 110.0).is_none());
        // Price goes to 120, trailing SL = 114.0
        assert!(monitor.check_position("t1", 120.0).is_none());
        // Price drops to 115 -> still above 114
        assert!(monitor.check_position("t1", 115.0).is_none());
        // Price drops to 113.5 -> below 114
        let sig = monitor.check_position("t1", 113.5).unwrap();
        assert_eq!(sig.reason, CloseReason::TrailingStop);
    }

    #[test]
    fn test_trailing_stop_short() {
        let monitor = TradeMonitor::new();
        let mut pos = short_position("s1", 3000.0, None, None);
        pos.trailing_stop_pct = Some(3.0);
        monitor.add_position(pos);

        // Price drops to 2800 -> lowest updates
        assert!(monitor.check_position("s1", 2800.0).is_none());
        // Trailing SL = 2800 * 1.03 = 2884
        // Price goes to 2850 -> still below 2884
        assert!(monitor.check_position("s1", 2850.0).is_none());
        // Price goes to 2890 -> above 2884
        let sig = monitor.check_position("s1", 2890.0).unwrap();
        assert_eq!(sig.reason, CloseReason::TrailingStop);
    }

    #[test]
    fn test_breakeven_long() {
        let monitor = TradeMonitor::with_breakeven_trigger(TradeMonitor::new(), 1.5);
        // Entry at 100, SL at 95
        monitor.add_position(long_position("t1", 100.0, Some(95.0), None));

        // Price goes to 102 (2% gain, > 1.5% trigger) -> update highest
        assert!(monitor.check_position("t1", 102.0).is_none());
        // Now price comes back to entry -> break-even stop triggers
        let sig = monitor.check_position("t1", 100.0).unwrap();
        assert_eq!(sig.reason, CloseReason::BreakEvenStop);
    }

    #[test]
    fn test_check_all() {
        let monitor = TradeMonitor::new();
        monitor.add_position(long_position("t1", 50000.0, Some(49000.0), Some(55000.0)));
        let mut pos2 = short_position("s1", 3000.0, Some(3100.0), Some(2800.0));
        pos2.symbol = "ETH/USDT".to_string();
        monitor.add_position(pos2);

        let mut prices = HashMap::new();
        prices.insert("BTC/USDT".to_string(), 48000.0); // SL hit for long
        prices.insert("ETH/USDT".to_string(), 2750.0);  // TP hit for short

        let signals = monitor.check_all(&prices);
        assert_eq!(signals.len(), 2);
    }

    #[test]
    fn test_calculate_pnl_long() {
        // Buy at 100, sell at 110, 5 units, 0.1% fee
        let pnl = TradeMonitor::calculate_pnl(100.0, 110.0, 5.0, OrderSide::Buy, 0.001);
        // Gross: (110-100)*5 = 50
        // Fee: (100*5 + 110*5) * 0.001 = 1050 * 0.001 = 1.05
        // Net: 48.95
        assert!((pnl - 48.95).abs() < 0.01);
    }

    #[test]
    fn test_calculate_pnl_short() {
        // Sell at 100, buy back at 90, 3 units, 0.1% fee
        let pnl = TradeMonitor::calculate_pnl(100.0, 90.0, 3.0, OrderSide::Sell, 0.001);
        // Gross: (100-90)*3 = 30
        // Fee: (100*3 + 90*3) * 0.001 = 570 * 0.001 = 0.57
        // Net: 29.43
        assert!((pnl - 29.43).abs() < 0.01);
    }

    #[test]
    fn test_update_price_tracking() {
        let monitor = TradeMonitor::new();
        monitor.add_position(long_position("t1", 50000.0, None, None));

        monitor.update_price("BTC/USDT", 52000.0);
        monitor.update_price("BTC/USDT", 48000.0);

        let pos = monitor.get_position("t1").unwrap();
        assert_eq!(pos.highest_price, 52000.0);
        assert_eq!(pos.lowest_price, 48000.0);
    }

    #[test]
    fn test_list_positions() {
        let monitor = TradeMonitor::new();
        monitor.add_position(long_position("t1", 50000.0, None, None));
        monitor.add_position(long_position("t2", 51000.0, None, None));
        let list = monitor.list_positions();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_no_signal_in_safe_zone() {
        let monitor = TradeMonitor::new();
        monitor.add_position(long_position("t1", 50000.0, Some(49000.0), Some(55000.0)));

        // Price comfortably between SL and TP
        assert!(monitor.check_position("t1", 52000.0).is_none());
        assert!(monitor.check_position("t1", 50500.0).is_none());
        assert!(monitor.check_position("t1", 54999.0).is_none());
    }
}
