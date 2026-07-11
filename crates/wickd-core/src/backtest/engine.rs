//! Backtest execution engine
//!
//! Simulates trading a strategy on historical data and calculates
//! performance metrics.

use std::collections::HashMap;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::models::Candle;
use shared::{EntryOrderType, PositionDirection};
use super::strategy::{ExtendedSignal, PositionSnapshot, Signal, Strategy};

/// Configuration for a backtest run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestConfig {
    /// Starting account balance
    pub initial_balance: Decimal,
    /// Position size in units (or percentage if use_percentage is true)
    pub position_size: Decimal,
    /// Whether position_size is a percentage of balance (for compounding)
    pub use_percentage: bool,
    /// Risk percentage per trade (0-100). Used to calculate position size dynamically.
    /// If set, overrides position_size and uses: risk_amount / (estimated_stop_pips * pip_value)
    pub risk_percent: Option<Decimal>,
    /// Estimated stop loss in pips for risk-based position sizing
    pub estimated_stop_pips: Decimal,
    /// Spread in pips (used to simulate slippage)
    pub spread_pips: Decimal,
    /// Pip value for the instrument (e.g., 0.0001 for EUR/USD)
    pub pip_value: Decimal,
    /// Instrument symbol (e.g. "USD_JPY"). Used to convert quote-currency P&L
    /// into the account's home currency. Empty = skip conversion (legacy: P&L
    /// stays in the quote currency, correct only for *_USD pairs).
    /// `serde(default)` keeps JSON that predates this field deserializable.
    #[serde(default)]
    pub instrument: String,
    /// Number of leading candles used only to warm up indicators: the
    /// strategy sees them (state/indicators build normally) but no entries
    /// are taken and they are excluded from the equity curve and metrics.
    #[serde(default)]
    pub warmup_bars: usize,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            initial_balance: dec!(10000),
            position_size: dec!(1000),
            use_percentage: false,
            risk_percent: None,
            estimated_stop_pips: dec!(20),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            instrument: String::new(),
            warmup_bars: 0,
        }
    }
}

/// A simulated trade during backtesting
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatedTrade {
    pub entry_time: String,
    pub exit_time: Option<String>,
    pub entry_price: Decimal,
    pub exit_price: Option<Decimal>,
    pub units: Decimal,
    pub pnl: Decimal,
    /// Round-trip spread cost embedded in `pnl` (home currency): what this
    /// trade paid vs a mid-to-mid fill. Gross P&L = pnl + spread_cost.
    #[serde(default)]
    pub spread_cost: Decimal,
    pub is_long: bool,
    /// ID of the entry rule that triggered this trade
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_rule_id: Option<String>,
    /// Name of the entry rule (if set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_rule_name: Option<String>,
    /// Reason for exit (from exit rule name/id)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_reason: Option<String>,
    /// Stop loss price at entry
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss: Option<Decimal>,
    /// Take profit price at entry
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit: Option<Decimal>,
    /// Indicator values at entry (indicator_id -> value as string)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_indicators: Option<HashMap<String, String>>,
}

/// Performance metrics from a backtest
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestMetrics {
    /// Total profit/loss, NET of simulated spread costs (fills at mid ± spread)
    pub total_pnl: Decimal,
    /// Total round-trip spread cost paid across all trades (home currency)
    #[serde(default)]
    pub total_spread_cost: Decimal,
    /// Total profit/loss as if fills were mid-to-mid: total_pnl + total_spread_cost
    #[serde(default)]
    pub gross_pnl: Decimal,
    /// Return as percentage
    pub total_return_pct: Decimal,
    /// Annualized return as percentage (projected to 1 year)
    pub annualized_return_pct: Decimal,
    /// Number of winning trades
    pub winning_trades: u32,
    /// Number of losing trades
    pub losing_trades: u32,
    /// Win rate as percentage
    pub win_rate: Decimal,
    /// Average winning trade P&L
    pub avg_win: Decimal,
    /// Average losing trade P&L
    pub avg_loss: Decimal,
    /// Profit factor (gross profit / gross loss)
    pub profit_factor: Decimal,
    /// Maximum drawdown as percentage
    pub max_drawdown_pct: Decimal,
    /// Sharpe ratio, annualized by the actual candle sampling frequency
    /// (per-candle equity returns scaled by sqrt(periods per year)).
    pub sharpe_ratio: Decimal,
    /// Total number of trades
    pub total_trades: u32,
}

impl Default for BacktestMetrics {
    fn default() -> Self {
        Self {
            total_pnl: Decimal::ZERO,
            total_spread_cost: Decimal::ZERO,
            gross_pnl: Decimal::ZERO,
            total_return_pct: Decimal::ZERO,
            annualized_return_pct: Decimal::ZERO,
            winning_trades: 0,
            losing_trades: 0,
            win_rate: Decimal::ZERO,
            avg_win: Decimal::ZERO,
            avg_loss: Decimal::ZERO,
            profit_factor: Decimal::ZERO,
            max_drawdown_pct: Decimal::ZERO,
            sharpe_ratio: Decimal::ZERO,
            total_trades: 0,
        }
    }
}

/// A pending (stop/limit) order waiting to be filled by price action.
/// Created when a strategy signal includes a PendingOrderInfo.
#[derive(Debug, Clone)]
struct PendingOrder {
    /// Long or short direction
    direction: PositionDirection,
    /// The type of pending order (buy stop, sell stop, buy limit, sell limit)
    order_type: EntryOrderType,
    /// The trigger price level
    price: Decimal,
    /// Stop loss for the trade when filled
    stop_loss: Option<Decimal>,
    /// Take profit for the trade when filled
    take_profit: Option<Decimal>,
    /// ID of the entry rule that triggered this order
    entry_rule_id: Option<String>,
    /// Name of the entry rule
    entry_rule_name: Option<String>,
    /// Indicator values captured at signal time
    entry_indicators: Option<HashMap<String, String>>,
    /// Bars remaining before expiry (None = no expiry)
    bars_remaining: Option<u32>,
    /// Time the order was created (for logging/debugging)
    created_time: String,
}

impl PendingOrder {
    /// Check if this pending order should fill given the current candle's price range.
    /// Returns true if the order's trigger price was hit.
    fn should_fill(&self, candle: &Candle, spread: Decimal) -> bool {
        match self.order_type {
            EntryOrderType::BuyStop => {
                // Buy stop fills when ASK reaches the order price.
                // ASK high = mid.high + spread
                (candle.mid.high + spread) >= self.price
            }
            EntryOrderType::SellStop => {
                // Sell stop fills when BID reaches the order price.
                // BID low = mid.low - spread
                (candle.mid.low - spread) <= self.price
            }
            EntryOrderType::BuyLimit => {
                // Buy limit fills when ASK drops to order price.
                // ASK low = mid.low + spread
                (candle.mid.low + spread) <= self.price
            }
            EntryOrderType::SellLimit => {
                // Sell limit fills when BID rises to order price.
                // BID high = mid.high - spread
                (candle.mid.high - spread) >= self.price
            }
            EntryOrderType::Market => {
                // Market orders don't go through pending order mechanism
                false
            }
        }
    }
}

/// Complete backtest results
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResult {
    pub metrics: BacktestMetrics,
    pub trades: Vec<SimulatedTrade>,
    /// Equity curve (balance over time)
    pub equity_curve: Vec<Decimal>,
    pub final_balance: Decimal,
}

/// The backtest engine
pub struct BacktestEngine {
    config: BacktestConfig,
}

impl BacktestEngine {
    pub fn new(config: BacktestConfig) -> Self {
        Self { config }
    }

    /// Run a backtest with the given strategy on historical candles
    ///
    /// IMPORTANT: Entry/exit execution uses next candle's open price to avoid look-ahead bias.
    /// When a signal is generated on candle N, execution happens at candle N+1's open.
    /// Pending orders (stop/limit) wait until price reaches a specified level within a candle.
    pub fn run<S: Strategy>(&self, strategy: &mut S, candles: &[Candle]) -> BacktestResult {
        let mut balance = self.config.initial_balance;
        let mut trades: Vec<SimulatedTrade> = Vec::new();
        let mut equity_curve: Vec<Decimal> = vec![balance];
        let mut current_position: Option<SimulatedTrade> = None;

        // Pending signals to be executed on the next candle's open (market orders)
        // Store the extended signal to access stop_loss for position sizing
        let mut pending_signal: Option<ExtendedSignal> = None;

        // Pending order (stop/limit) waiting for price to reach trigger level
        // Only one pending order at a time (simplification). New pending order replaces existing.
        let mut current_pending_order: Option<PendingOrder> = None;

        // Reset strategy state
        strategy.reset();

        // Prepare strategy with full candle data (for pattern detection, etc.)
        strategy.prepare(candles);

        for (candle_idx, candle) in candles.iter().enumerate() {
            // Only process complete candles
            if !candle.complete {
                continue;
            }
            // Warmup candles feed the strategy (indicator/state build-up in
            // Step 4) but take no entries and don't count toward the equity
            // curve. No position can exist during warmup, so Steps 1–3 are
            // structurally no-ops there.
            let in_warmup = candle_idx < self.config.warmup_bars;

            // === Step 1: Check pending orders for fills ===
            // This must happen before executing pending market signals and before SL/TP checks.
            // Only fill if no position is currently open.
            if current_position.is_none() {
                if let Some(ref mut order) = current_pending_order {
                    let spread = self.spread_amount();
                    if order.should_fill(candle, spread) {
                        // Fill at the order's price directly. The should_fill() check already
                        // uses spread-adjusted prices (ASK for buys, BID for sells), so
                        // order.price IS the execution price — no additional spread adjustment.
                        let is_long = matches!(order.direction, PositionDirection::Long);
                        let entry_price = order.price;
                        let units = self.calculate_position_size_with_stop(
                            balance,
                            entry_price,
                            order.stop_loss,
                        );
                        current_position = Some(SimulatedTrade {
                            entry_time: candle.time.to_rfc3339(),
                            exit_time: None,
                            entry_price,
                            exit_price: None,
                            units,
                            pnl: Decimal::ZERO,
                            spread_cost: Decimal::ZERO,
                            is_long,
                            entry_rule_id: order.entry_rule_id.clone(),
                            entry_rule_name: order.entry_rule_name.clone(),
                            exit_reason: None,
                            stop_loss: order.stop_loss,
                            take_profit: order.take_profit,
                            entry_indicators: order.entry_indicators.clone(),
                        });
                        // Order filled, remove it
                        current_pending_order = None;
                    } else {
                        // Decrement bars remaining and check expiry
                        if let Some(ref mut remaining) = order.bars_remaining {
                            if *remaining <= 1 {
                                // Order expired
                                current_pending_order = None;
                            } else {
                                *remaining -= 1;
                            }
                        }
                    }
                }
            }

            // === Step 2: Execute any pending market signal at this candle's OPEN price ===
            // This simulates realistic execution: signal on candle N -> execute at open of candle N+1
            if let Some(ext_signal) = pending_signal.take() {
                match ext_signal.signal {
                    Signal::Buy => {
                        // Close any short position first at this candle's open
                        if let Some(pos) = current_position.take() {
                            if !pos.is_long {
                                // Closing short = buying back at ASK (open + spread)
                                let exit_price = candle.mid.open + self.spread_amount();
                                let pnl = self.calculate_pnl(&pos, exit_price);
                                balance += pnl;
                                trades.push(SimulatedTrade {
                                    exit_time: Some(candle.time.to_rfc3339()),
                                    exit_price: Some(exit_price),
                                    pnl,
                                    ..pos
                                });
                            } else {
                                // Already long, keep position
                                current_position = Some(pos);
                            }
                        }

                        // Open long if no position - buy at ASK (open + spread)
                        if current_position.is_none() {
                            let entry_price = candle.mid.open + self.spread_amount();

                            // Validate SL against actual entry price (SL was computed on
                            // the signal candle using mid.close, but actual entry is next
                            // candle's open ± spread — a gap can put SL on the wrong side)
                            if let Some(sl) = ext_signal.stop_loss {
                                if sl >= entry_price {
                                    warn!(
                                        "Skipping long entry: SL ({}) >= entry price ({}) — \
                                         gap between signal candle and execution candle \
                                         invalidated the stop loss",
                                        sl, entry_price
                                    );
                                    strategy.notify_entry_rejected();
                                    // Skip to next section (don't create position)
                                } else {
                                    let units = self.calculate_position_size_with_stop(
                                        balance,
                                        entry_price,
                                        ext_signal.stop_loss,
                                    );
                                    current_position = Some(SimulatedTrade {
                                        entry_time: candle.time.to_rfc3339(),
                                        exit_time: None,
                                        entry_price,
                                        exit_price: None,
                                        units,
                                        pnl: Decimal::ZERO,
                                        spread_cost: Decimal::ZERO,
                                        is_long: true,
                                        entry_rule_id: ext_signal.entry_rule_id.clone(),
                                        entry_rule_name: ext_signal.entry_rule_name.clone(),
                                        exit_reason: None,
                                        stop_loss: ext_signal.stop_loss,
                                        take_profit: ext_signal.take_profit,
                                        entry_indicators: ext_signal.entry_indicators.clone(),
                                    });
                                }
                            } else {
                                // No SL — proceed normally
                                let units = self.calculate_position_size_with_stop(
                                    balance,
                                    entry_price,
                                    ext_signal.stop_loss,
                                );
                                current_position = Some(SimulatedTrade {
                                    entry_time: candle.time.to_rfc3339(),
                                    exit_time: None,
                                    entry_price,
                                    exit_price: None,
                                    units,
                                    pnl: Decimal::ZERO,
                                    spread_cost: Decimal::ZERO,
                                    is_long: true,
                                    entry_rule_id: ext_signal.entry_rule_id.clone(),
                                    entry_rule_name: ext_signal.entry_rule_name.clone(),
                                    exit_reason: None,
                                    stop_loss: ext_signal.stop_loss,
                                    take_profit: ext_signal.take_profit,
                                    entry_indicators: ext_signal.entry_indicators.clone(),
                                });
                            }
                        }
                    }
                    Signal::Sell => {
                        // Close any long position first at this candle's open
                        if let Some(pos) = current_position.take() {
                            if pos.is_long {
                                // Closing long = selling at BID (open - spread)
                                let exit_price = candle.mid.open - self.spread_amount();
                                let pnl = self.calculate_pnl(&pos, exit_price);
                                balance += pnl;
                                trades.push(SimulatedTrade {
                                    exit_time: Some(candle.time.to_rfc3339()),
                                    exit_price: Some(exit_price),
                                    pnl,
                                    ..pos
                                });
                            } else {
                                // Already short, keep position
                                current_position = Some(pos);
                            }
                        }

                        // Open short if no position - sell at BID (open - spread)
                        if current_position.is_none() {
                            let entry_price = candle.mid.open - self.spread_amount();

                            // Validate SL against actual entry price (see long branch comment)
                            if let Some(sl) = ext_signal.stop_loss {
                                if sl <= entry_price {
                                    warn!(
                                        "Skipping short entry: SL ({}) <= entry price ({}) — \
                                         gap between signal candle and execution candle \
                                         invalidated the stop loss",
                                        sl, entry_price
                                    );
                                    strategy.notify_entry_rejected();
                                    // Skip to next section (don't create position)
                                } else {
                                    let units = self.calculate_position_size_with_stop(
                                        balance,
                                        entry_price,
                                        ext_signal.stop_loss,
                                    );
                                    current_position = Some(SimulatedTrade {
                                        entry_time: candle.time.to_rfc3339(),
                                        exit_time: None,
                                        entry_price,
                                        exit_price: None,
                                        units,
                                        pnl: Decimal::ZERO,
                                        spread_cost: Decimal::ZERO,
                                        is_long: false,
                                        entry_rule_id: ext_signal.entry_rule_id.clone(),
                                        entry_rule_name: ext_signal.entry_rule_name.clone(),
                                        exit_reason: None,
                                        stop_loss: ext_signal.stop_loss,
                                        take_profit: ext_signal.take_profit,
                                        entry_indicators: ext_signal.entry_indicators.clone(),
                                    });
                                }
                            } else {
                                // No SL — proceed normally
                                let units = self.calculate_position_size_with_stop(
                                    balance,
                                    entry_price,
                                    ext_signal.stop_loss,
                                );
                                current_position = Some(SimulatedTrade {
                                    entry_time: candle.time.to_rfc3339(),
                                    exit_time: None,
                                    entry_price,
                                    exit_price: None,
                                    units,
                                    pnl: Decimal::ZERO,
                                    spread_cost: Decimal::ZERO,
                                    is_long: false,
                                    entry_rule_id: ext_signal.entry_rule_id.clone(),
                                    entry_rule_name: ext_signal.entry_rule_name.clone(),
                                    exit_reason: None,
                                    stop_loss: ext_signal.stop_loss,
                                    take_profit: ext_signal.take_profit,
                                    entry_indicators: ext_signal.entry_indicators.clone(),
                                });
                            }
                        }
                    }
                    Signal::ClosePosition => {
                        if let Some(mut pos) = current_position.take() {
                            let exit_price = if pos.is_long {
                                // Closing long = selling at BID (open - spread)
                                candle.mid.open - self.spread_amount()
                            } else {
                                // Closing short = buying at ASK (open + spread)
                                candle.mid.open + self.spread_amount()
                            };
                            let pnl = self.calculate_pnl(&pos, exit_price);
                            balance += pnl;
                            // Capture exit reason from the signal
                            pos.exit_reason = ext_signal.exit_reason.clone();
                            trades.push(SimulatedTrade {
                                exit_time: Some(candle.time.to_rfc3339()),
                                exit_price: Some(exit_price),
                                pnl,
                                ..pos
                            });
                        }
                    }
                    Signal::Hold => {
                        // Do nothing
                    }
                }
            }

            // === Step 3: Intra-bar Stop Loss / Take Profit check ===
            // Uses SL/TP as they were at the START of this candle (previous candle updated the trail)
            if let Some(ref pos) = current_position {
                let sl = strategy.current_stop_loss().or(pos.stop_loss);
                let tp = strategy.current_take_profit().or(pos.take_profit);

                let spread = self.spread_amount();
                let mut sl_hit = false;
                let mut tp_hit = false;

                if let Some(sl_price) = sl {
                    if pos.is_long {
                        // Long SL: exit at BID. BID low = mid.low - spread
                        sl_hit = (candle.mid.low - spread) <= sl_price;
                    } else {
                        // Short SL: exit at ASK. ASK high = mid.high + spread
                        sl_hit = (candle.mid.high + spread) >= sl_price;
                    }
                }

                if let Some(tp_price) = tp {
                    if pos.is_long {
                        // Long TP: exit at BID. BID high = mid.high - spread
                        tp_hit = (candle.mid.high - spread) >= tp_price;
                    } else {
                        // Short TP: exit at ASK. ASK low = mid.low + spread
                        tp_hit = (candle.mid.low + spread) <= tp_price;
                    }
                }

                // SL takes priority (conservative assumption when both breached same candle)
                if sl_hit || tp_hit {
                    let mut pos = current_position.take().unwrap();
                    let (exit_price, exit_reason) = if sl_hit {
                        (sl.unwrap(), "Stop Loss".to_string())
                    } else {
                        (tp.unwrap(), "Take Profit".to_string())
                    };

                    let pnl = self.calculate_pnl(&pos, exit_price);
                    balance += pnl;
                    pos.exit_reason = Some(exit_reason);
                    trades.push(SimulatedTrade {
                        exit_time: Some(candle.time.to_rfc3339()),
                        exit_price: Some(exit_price),
                        pnl,
                        ..pos
                    });
                    // Notify strategy so its internal position state stays in sync
                    // (e.g., RulesEngine clears self.position to re-enable entry evaluation)
                    strategy.notify_position_closed();
                }
            }

            // === Step 4: Generate signal based on this candle ===
            // Push the settled position state into the strategy first (ABI v5:
            // in_position()/entry_price()/bars_since_entry()) — one call per
            // candle, after fills and SL/TP, so the script sees the truth for
            // the candle it is about to evaluate.
            strategy.sync_position_state(current_position.as_ref().map(|pos| PositionSnapshot {
                entry_price: pos.entry_price,
                is_long: pos.is_long,
            }));
            // Use on_candle_extended to get stop_loss/take_profit info for position sizing.
            // During warmup the strategy still runs (its indicators/state must
            // build), but its signals are discarded so no entry can originate
            // from a warmup candle.
            let ext_signal = strategy.on_candle_extended(candle);
            if !in_warmup && ext_signal.signal != Signal::Hold {
                if ext_signal.pending_order.is_some()
                    && matches!(ext_signal.signal, Signal::Buy | Signal::Sell)
                {
                    // Signal has a pending order — create a PendingOrder instead of market signal.
                    // Only entry signals (Buy/Sell) can create pending orders.
                    // New pending order replaces any existing one.
                    let po_info = ext_signal.pending_order.as_ref().unwrap();
                    let direction = match ext_signal.signal {
                        Signal::Buy => PositionDirection::Long,
                        Signal::Sell => PositionDirection::Short,
                        _ => unreachable!(), // guarded by matches! above
                    };
                    current_pending_order = Some(PendingOrder {
                        direction,
                        order_type: po_info.order_type,
                        price: po_info.price,
                        stop_loss: ext_signal.stop_loss,
                        take_profit: ext_signal.take_profit,
                        entry_rule_id: ext_signal.entry_rule_id.clone(),
                        entry_rule_name: ext_signal.entry_rule_name.clone(),
                        entry_indicators: ext_signal.entry_indicators.clone(),
                        bars_remaining: po_info.expiry_bars,
                        created_time: candle.time.to_rfc3339(),
                    });
                    // Clear any pending market signal since we're using a pending order
                    pending_signal = None;
                } else {
                    // No pending order — use existing market order mechanism
                    pending_signal = Some(ext_signal);
                    // Cancel any pending order when a new market signal comes
                    current_pending_order = None;
                }
            }

            // Sync trailing stop loss from strategy to position
            if let Some(ref mut pos) = current_position {
                if let Some(trailed_sl) = strategy.current_stop_loss() {
                    pos.stop_loss = Some(trailed_sl);
                }
            }

            // Update equity curve with mark-to-market using this candle's CLOSE
            // This is after execution, so it reflects the current state
            let mtm_balance = if let Some(ref pos) = current_position {
                let current_price = if pos.is_long {
                    // Would sell at BID (close - spread)
                    candle.mid.close - self.spread_amount()
                } else {
                    // Would buy at ASK (close + spread)
                    candle.mid.close + self.spread_amount()
                };
                balance + self.calculate_pnl(pos, current_price)
            } else {
                balance
            };
            // Warmup candles are excluded from the equity curve so drawdown /
            // Sharpe measure the trading span only (a long flat warmup would
            // dilute per-candle return statistics).
            if !in_warmup {
                equity_curve.push(mtm_balance);
            }
        }

        // Close any remaining position at the last candle
        if let Some(pos) = current_position.take() {
            if let Some(last_candle) = candles.last() {
                let exit_price = if pos.is_long {
                    last_candle.mid.close - self.spread_amount()
                } else {
                    last_candle.mid.close + self.spread_amount()
                };
                let pnl = self.calculate_pnl(&pos, exit_price);
                balance += pnl;
                trades.push(SimulatedTrade {
                    exit_time: Some(last_candle.time.to_rfc3339()),
                    exit_price: Some(exit_price),
                    pnl,
                    ..pos
                });
                // Notify strategy of forced close so internal state is clean
                strategy.notify_position_closed();
            }
        }

        // Stamp each closed trade with the round-trip spread cost embedded in
        // its P&L (2 × half-spread × units, converted to home currency the
        // same way the P&L itself was).
        for trade in &mut trades {
            trade.spread_cost = self.trade_spread_cost(trade);
        }

        // Metrics cover the trading span only: warmup candles are excluded
        // from time-based calculations (annualization, Sharpe frequency).
        let trading_candles = &candles[self.config.warmup_bars.min(candles.len())..];
        let metrics = self.calculate_metrics(&trades, &equity_curve, trading_candles);

        BacktestResult {
            metrics,
            trades,
            equity_curve,
            final_balance: balance,
        }
    }

    fn spread_amount(&self) -> Decimal {
        self.config.spread_pips * self.config.pip_value
    }

    /// Calculate position size using actual stop loss distance if available.
    /// Falls back to estimated_stop_pips if no stop loss is provided.
    fn calculate_position_size_with_stop(
        &self,
        balance: Decimal,
        entry_price: Decimal,
        stop_loss: Option<Decimal>,
    ) -> Decimal {
        // If risk_percent is set, calculate position size dynamically
        if let Some(risk_pct) = self.config.risk_percent {
            let risk_amount = balance * risk_pct / dec!(100);

            // Use actual stop distance if provided, otherwise fall back to estimated
            let stop_distance = if let Some(sl) = stop_loss {
                (entry_price - sl).abs()
            } else {
                self.config.estimated_stop_pips * self.config.pip_value
            };

            if stop_distance > Decimal::ZERO {
                risk_amount / stop_distance
            } else {
                self.config.position_size
            }
        } else if self.config.use_percentage {
            balance * self.config.position_size / dec!(100)
        } else {
            self.config.position_size
        }
    }

    #[allow(dead_code)]
    fn calculate_position_size(&self, balance: Decimal) -> Decimal {
        // Legacy method - uses estimated_stop_pips only
        self.calculate_position_size_with_stop(balance, Decimal::ZERO, None)
    }

    fn calculate_pnl(&self, trade: &SimulatedTrade, exit_price: Decimal) -> Decimal {
        let price_diff = if trade.is_long {
            exit_price - trade.entry_price
        } else {
            trade.entry_price - exit_price
        };
        // `price_diff * units` is the raw P&L in the pair's QUOTE currency.
        let pnl_quote = price_diff * trade.units;
        // Convert to the account's home currency (USD — every OANDA account we
        // back-test against is USD-denominated). Without this, JPY-quoted pairs
        // book P&L in yen (~150x the USD value) and swamp any cross-pair sum.
        self.to_home_currency(pnl_quote, exit_price)
    }

    /// Round-trip spread cost of a closed trade in the home currency: the
    /// engine fills each side at mid ± spread_amount, so a completed trade
    /// paid `2 × spread_amount × units` in the quote currency vs mid-to-mid
    /// fills. Converted to home the same way P&L is (at the exit price).
    /// For pending-order (stop/limit) entries the entry-side cost is an
    /// approximation: the fill happened at the order's bid/ask-crossed price,
    /// which sits within one spread of mid by construction.
    fn trade_spread_cost(&self, trade: &SimulatedTrade) -> Decimal {
        let cost_quote = dec!(2) * self.spread_amount() * trade.units;
        let reference_price = trade.exit_price.unwrap_or(trade.entry_price);
        self.to_home_currency(cost_quote, reference_price)
    }

    /// Convert a quote-currency amount to the USD home currency using the pair
    /// price at the moment of realization.
    /// - `*_USD` (EUR_USD, GBP_USD): quote is already USD → no change.
    /// - `USD_*` (USD_JPY, USD_CHF): quote is the second leg; 1 quote unit =
    ///   `1/price` USD, since the pair price is "quote per USD".
    /// - crosses (EUR_GBP): neither leg is USD; converting needs an external
    ///   rate we don't have in a single-instrument backtest, so it's left in
    ///   the quote currency (documented limitation).
    fn to_home_currency(&self, pnl_quote: Decimal, price: Decimal) -> Decimal {
        let (base, quote) = match self.config.instrument.split_once('_') {
            Some(bq) => bq,
            None => return pnl_quote, // unknown/empty instrument: legacy behavior
        };
        if quote == "USD" {
            pnl_quote
        } else if base == "USD" && !price.is_zero() {
            pnl_quote / price
        } else {
            pnl_quote // cross pair: no USD conversion rate available
        }
    }

    fn calculate_metrics(&self, trades: &[SimulatedTrade], equity_curve: &[Decimal], candles: &[Candle]) -> BacktestMetrics {
        if trades.is_empty() {
            return BacktestMetrics::default();
        }

        let mut winning_trades = 0u32;
        let mut losing_trades = 0u32;
        let mut gross_profit = Decimal::ZERO;
        let mut gross_loss = Decimal::ZERO;

        for trade in trades {
            if trade.pnl > Decimal::ZERO {
                winning_trades += 1;
                gross_profit += trade.pnl;
            } else if trade.pnl < Decimal::ZERO {
                losing_trades += 1;
                gross_loss += trade.pnl.abs();
            }
        }

        let total_trades = trades.len() as u32;
        let total_pnl: Decimal = trades.iter().map(|t| t.pnl).sum();
        let total_spread_cost: Decimal = trades.iter().map(|t| t.spread_cost).sum();
        let gross_pnl = total_pnl + total_spread_cost;
        let total_return_pct = (total_pnl / self.config.initial_balance) * dec!(100);

        // Calculate annualized return based on data range
        let annualized_return_pct = self.calculate_annualized_return(total_return_pct, candles);

        let win_rate = if total_trades > 0 {
            Decimal::from(winning_trades) / Decimal::from(total_trades) * dec!(100)
        } else {
            Decimal::ZERO
        };

        let avg_win = if winning_trades > 0 {
            gross_profit / Decimal::from(winning_trades)
        } else {
            Decimal::ZERO
        };

        let avg_loss = if losing_trades > 0 {
            gross_loss / Decimal::from(losing_trades)
        } else {
            Decimal::ZERO
        };

        let profit_factor = if gross_loss > Decimal::ZERO {
            gross_profit / gross_loss
        } else if gross_profit > Decimal::ZERO {
            dec!(999.99) // Infinite profit factor capped
        } else {
            Decimal::ZERO
        };

        // Calculate max drawdown
        let max_drawdown_pct = self.calculate_max_drawdown(equity_curve);

        // Calculate Sharpe ratio (simplified, assuming risk-free rate = 0)
        let sharpe_ratio = self.calculate_sharpe_ratio(equity_curve, candles);

        BacktestMetrics {
            total_pnl,
            total_spread_cost,
            gross_pnl,
            total_return_pct,
            annualized_return_pct,
            winning_trades,
            losing_trades,
            win_rate,
            avg_win,
            avg_loss,
            profit_factor,
            max_drawdown_pct,
            sharpe_ratio,
            total_trades,
        }
    }

    fn calculate_annualized_return(&self, total_return_pct: Decimal, candles: &[Candle]) -> Decimal {
        if candles.len() < 2 {
            return Decimal::ZERO;
        }

        // Get the time span of the data
        let first_time = candles.first().map(|c| c.time);
        let last_time = candles.last().map(|c| c.time);

        if let (Some(start), Some(end)) = (first_time, last_time) {
            let duration = end.signed_duration_since(start);
            let days = duration.num_days();

            if days <= 0 {
                return Decimal::ZERO;
            }

            // Convert total return to decimal (e.g., 10% -> 0.10)
            let return_decimal = total_return_pct / dec!(100);

            // Calculate annualized return using compound formula:
            // Annualized = (1 + total_return)^(365/days) - 1
            // Using approximation: ln(1+r) ≈ r for small r, so (1+r)^n ≈ e^(n*r) ≈ 1 + n*r for moderate values
            // For more accuracy, we use: (1 + r)^(365/days) - 1

            let days_decimal = Decimal::from(days);
            let exponent = dec!(365) / days_decimal;

            // For (1 + r)^n, use iterative calculation or approximation
            // Simple approach: annualized ≈ total_return * (365 / days) for small returns
            // More accurate: use the power function
            let base = dec!(1) + return_decimal;
            let annualized_factor = Self::decimal_pow(base, exponent);
            let annualized_return = (annualized_factor - dec!(1)) * dec!(100);

            // Cap extreme values (can happen with very short periods and high returns)
            if annualized_return > dec!(9999) {
                dec!(9999)
            } else if annualized_return < dec!(-99.99) {
                dec!(-99.99)
            } else {
                annualized_return
            }
        } else {
            Decimal::ZERO
        }
    }

    /// Calculate base^exponent using exp(exponent * ln(base))
    fn decimal_pow(base: Decimal, exponent: Decimal) -> Decimal {
        if base <= Decimal::ZERO {
            return Decimal::ZERO;
        }

        // Use natural log approximation: ln(x) ≈ 2 * sum of ((x-1)/(x+1))^(2n+1) / (2n+1)
        let ln_base = Self::decimal_ln(base);
        let product = exponent * ln_base;
        Self::decimal_exp(product)
    }

    /// Natural logarithm using series expansion
    fn decimal_ln(x: Decimal) -> Decimal {
        if x <= Decimal::ZERO {
            return Decimal::ZERO;
        }

        // For ln(x), use: ln(x) = 2 * arctanh((x-1)/(x+1))
        // arctanh(y) = y + y^3/3 + y^5/5 + ...
        let y = (x - dec!(1)) / (x + dec!(1));
        let y2 = y * y;

        let mut result = y;
        let mut term = y;

        for n in 1..50 {
            term = term * y2;
            let divisor = Decimal::from(2 * n + 1);
            result = result + term / divisor;
        }

        result * dec!(2)
    }

    /// Exponential function using Taylor series
    fn decimal_exp(x: Decimal) -> Decimal {
        // e^x = 1 + x + x^2/2! + x^3/3! + ...
        let mut result = dec!(1);
        let mut term = dec!(1);

        for n in 1..50 {
            term = term * x / Decimal::from(n);
            result = result + term;

            // Early exit if term is very small
            if term.abs() < dec!(0.0000000001) {
                break;
            }
        }

        result
    }

    fn calculate_max_drawdown(&self, equity_curve: &[Decimal]) -> Decimal {
        if equity_curve.len() < 2 {
            return Decimal::ZERO;
        }

        let mut max_equity = equity_curve[0];
        let mut max_drawdown = Decimal::ZERO;

        for &equity in equity_curve.iter() {
            if equity > max_equity {
                max_equity = equity;
            }
            // Guard against division by zero (can happen if initial_balance is zero)
            if max_equity > Decimal::ZERO {
                let drawdown = (max_equity - equity) / max_equity * dec!(100);
                if drawdown > max_drawdown {
                    max_drawdown = drawdown;
                }
            }
        }

        max_drawdown
    }

    fn calculate_sharpe_ratio(&self, equity_curve: &[Decimal], candles: &[Candle]) -> Decimal {
        if equity_curve.len() < 3 {
            return Decimal::ZERO;
        }

        // Per-period returns (skip any periods where prior equity is zero)
        let returns: Vec<Decimal> = equity_curve
            .windows(2)
            .filter(|w| w[0] > Decimal::ZERO)
            .map(|w| (w[1] - w[0]) / w[0])
            .collect();

        if returns.is_empty() {
            return Decimal::ZERO;
        }

        let n = Decimal::from(returns.len() as u32);
        let mean_return: Decimal = returns.iter().sum::<Decimal>() / n;

        // Calculate standard deviation
        let variance: Decimal = returns
            .iter()
            .map(|r| (*r - mean_return) * (*r - mean_return))
            .sum::<Decimal>() / n;

        // Simple sqrt approximation using Newton's method
        let std_dev = Self::decimal_sqrt(variance);

        if std_dev == Decimal::ZERO {
            return Decimal::ZERO;
        }

        // Annualize by the ACTUAL sampling frequency. The equity curve is
        // marked to market once per candle, so `returns` are per-candle — not
        // daily. Scaling by a hardcoded sqrt(252) assumed daily sampling and
        // understated every intraday timeframe (e.g. H4 by ~sqrt(6)). Instead
        // scale by sqrt(periods per year) = sqrt(returns.len() / years), derived
        // from the candle span, which is correct for any granularity.
        let per_period_sharpe = mean_return / std_dev;
        let span_secs = match (candles.first(), candles.last()) {
            (Some(f), Some(l)) => l.time.signed_duration_since(f.time).num_seconds(),
            _ => 0,
        };
        if span_secs <= 0 {
            // Can't determine the sampling frequency — return the un-annualized
            // ratio rather than fabricate a wrong factor.
            return per_period_sharpe;
        }
        // seconds per year (365.25 days) = 31_557_600
        let years = Decimal::from(span_secs) / dec!(31_557_600);
        let periods_per_year = Decimal::from(returns.len() as u64) / years;
        let annualization_factor = Self::decimal_sqrt(periods_per_year);

        per_period_sharpe * annualization_factor
    }

    /// Newton's method for square root
    fn decimal_sqrt(n: Decimal) -> Decimal {
        if n <= Decimal::ZERO {
            return Decimal::ZERO;
        }

        let mut x = n;
        let two = dec!(2);

        // Newton's method iterations
        for _ in 0..20 {
            let next_x = (x + n / x) / two;
            if (next_x - x).abs() < dec!(0.0000001) {
                break;
            }
            x = next_x;
        }

        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::scripted_strategy::ScriptedStrategy;
    use crate::backtest::strategy::PendingOrderInfo;
    use crate::models::{Candle, Ohlc};
    use chrono::{DateTime, Utc, Duration};

    // Simple test strategy: buy when close > open (bullish), sell when close < open (bearish)
    struct SimpleStrategy {
        in_position: bool,
        is_long: bool,
    }

    impl SimpleStrategy {
        fn new() -> Self {
            Self {
                in_position: false,
                is_long: false,
            }
        }
    }

    impl Strategy for SimpleStrategy {
        fn on_candle(&mut self, candle: &Candle) -> Signal {
            if candle.is_bullish() {
                if !self.in_position || !self.is_long {
                    self.in_position = true;
                    self.is_long = true;
                    return Signal::Buy;
                }
            } else if candle.is_bearish() {
                if !self.in_position || self.is_long {
                    self.in_position = true;
                    self.is_long = false;
                    return Signal::Sell;
                }
            }
            Signal::Hold
        }

        fn name(&self) -> &str {
            "SimpleStrategy"
        }

        fn reset(&mut self) {
            self.in_position = false;
            self.is_long = false;
        }
    }

    fn create_test_candles() -> Vec<Candle> {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        vec![
            // Bullish candle
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1050),
                    low: dec!(1.0980),
                    close: dec!(1.1040),
                },
                volume: 1000,
                complete: true,
            },
            // Bullish candle
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.1040),
                    high: dec!(1.1100),
                    low: dec!(1.1020),
                    close: dec!(1.1080),
                },
                volume: 1200,
                complete: true,
            },
            // Bearish candle
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.1080),
                    high: dec!(1.1090),
                    low: dec!(1.1000),
                    close: dec!(1.1010),
                },
                volume: 1500,
                complete: true,
            },
            // Bearish candle
            Candle {
                time: base_time + Duration::hours(3),
                mid: Ohlc {
                    open: dec!(1.1010),
                    high: dec!(1.1030),
                    low: dec!(1.0950),
                    close: dec!(1.0960),
                },
                volume: 1100,
                complete: true,
            },
            // Bullish candle
            Candle {
                time: base_time + Duration::hours(4),
                mid: Ohlc {
                    open: dec!(1.0960),
                    high: dec!(1.1050),
                    low: dec!(1.0940),
                    close: dec!(1.1030),
                },
                volume: 1300,
                complete: true,
            },
        ]
    }

    #[test]
    fn test_backtest_basic() {
        let config = BacktestConfig::default();
        let engine = BacktestEngine::new(config);
        let mut strategy = SimpleStrategy::new();
        let candles = create_test_candles();

        let result = engine.run(&mut strategy, &candles);

        assert!(result.trades.len() > 0);
        assert!(result.equity_curve.len() > 0);
    }

    #[test]
    fn test_warmup_bars_suppress_entries_and_shrink_equity_curve() {
        let candles = create_test_candles();

        // Baseline: first signal (bullish candle 0) executes at candle 1's open.
        let baseline = BacktestEngine::new(BacktestConfig::default())
            .run(&mut SimpleStrategy::new(), &candles);
        assert_eq!(
            baseline.trades[0].entry_time,
            candles[1].time.to_rfc3339(),
            "baseline sanity: entry at candle 1"
        );
        assert_eq!(baseline.equity_curve.len(), 1 + candles.len());

        // With 2 warmup candles, the earliest capturable signal is candle 2's,
        // executing at candle 3's open — and the equity curve only covers the
        // trading span.
        let config = BacktestConfig {
            warmup_bars: 2,
            ..Default::default()
        };
        let result = BacktestEngine::new(config).run(&mut SimpleStrategy::new(), &candles);
        let window_start = candles[2].time.to_rfc3339();
        for trade in &result.trades {
            assert!(
                trade.entry_time > window_start,
                "trade entered during warmup: {} <= {}",
                trade.entry_time,
                window_start
            );
        }
        assert!(!result.trades.is_empty(), "should still trade after warmup");
        assert_eq!(result.equity_curve.len(), 1 + candles.len() - 2);
    }

    #[test]
    fn test_gross_net_spread_cost_identity() {
        let candles = create_test_candles();
        let config = BacktestConfig {
            instrument: "EUR_USD".to_string(),
            ..Default::default()
        };
        let costed = BacktestEngine::new(config).run(&mut SimpleStrategy::new(), &candles);

        // Per trade: round-trip cost = 2 × spread_pips × pip_value × units.
        for trade in &costed.trades {
            assert_eq!(
                trade.spread_cost,
                dec!(2) * dec!(1) * dec!(0.0001) * trade.units,
                "per-trade round-trip spread cost"
            );
        }
        let metrics = &costed.metrics;
        assert_eq!(metrics.gross_pnl, metrics.total_pnl + metrics.total_spread_cost);
        assert!(metrics.total_spread_cost > Decimal::ZERO);

        // A zero-spread run books mid-to-mid fills at the same candles, so its
        // net P&L equals the costed run's gross P&L exactly.
        let zero_config = BacktestConfig {
            spread_pips: Decimal::ZERO,
            instrument: "EUR_USD".to_string(),
            ..Default::default()
        };
        let gross = BacktestEngine::new(zero_config).run(&mut SimpleStrategy::new(), &candles);
        assert_eq!(gross.metrics.total_pnl, costed.metrics.gross_pnl);
        assert_eq!(gross.metrics.total_spread_cost, Decimal::ZERO);
    }

    #[test]
    fn test_backtest_metrics() {
        let config = BacktestConfig {
            warmup_bars: 0,
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            use_percentage: false,
            risk_percent: None,
            estimated_stop_pips: dec!(20),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            instrument: String::new(),
        };
        let engine = BacktestEngine::new(config);
        let mut strategy = SimpleStrategy::new();
        let candles = create_test_candles();

        let result = engine.run(&mut strategy, &candles);

        assert_eq!(result.metrics.total_trades, result.trades.len() as u32);
        assert_eq!(
            result.metrics.winning_trades + result.metrics.losing_trades,
            result.metrics.total_trades
        );
    }

    #[test]
    fn test_max_drawdown() {
        let config = BacktestConfig::default();
        let engine = BacktestEngine::new(config);

        let equity_curve = vec![dec!(10000), dec!(10500), dec!(9500), dec!(10200), dec!(9000)];
        let max_dd = engine.calculate_max_drawdown(&equity_curve);

        // Max drawdown should be from 10500 to 9000 = 14.29%
        assert!(max_dd > dec!(14) && max_dd < dec!(15));
    }

    #[test]
    fn test_decimal_sqrt() {
        let result = BacktestEngine::decimal_sqrt(dec!(4));
        assert!((result - dec!(2)).abs() < dec!(0.0001));

        let result = BacktestEngine::decimal_sqrt(dec!(252));
        assert!((result - dec!(15.8745)).abs() < dec!(0.001));
    }

    // Strategy that enters long immediately, with a configurable SL/TP on the position
    struct SlTpTestStrategy {
        stop_loss: Option<Decimal>,
        take_profit: Option<Decimal>,
        entered: bool,
    }

    impl SlTpTestStrategy {
        fn new(stop_loss: Option<Decimal>, take_profit: Option<Decimal>) -> Self {
            Self { stop_loss, take_profit, entered: false }
        }
    }

    impl Strategy for SlTpTestStrategy {
        fn on_candle(&mut self, _candle: &Candle) -> Signal {
            Signal::Hold // unused, we use on_candle_extended
        }

        fn on_candle_extended(&mut self, _candle: &Candle) -> ExtendedSignal {
            if !self.entered {
                self.entered = true;
                ExtendedSignal {
                    signal: Signal::Buy,
                    stop_loss: self.stop_loss,
                    take_profit: self.take_profit,
                    ..ExtendedSignal::default()
                }
            } else {
                ExtendedSignal::default()
            }
        }

        fn name(&self) -> &str { "SlTpTestStrategy" }

        fn reset(&mut self) {
            self.entered = false;
        }
    }

    fn sl_tp_test_candles() -> Vec<Candle> {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        vec![
            // Candle 0: signal generated here (Buy)
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1010),
                    low: dec!(1.0990),
                    close: dec!(1.1005),
                },
                volume: 100,
                complete: true,
            },
            // Candle 1: entry executed at open (1.1020). Normal candle, no SL/TP breach.
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.1020),
                    high: dec!(1.1030),
                    low: dec!(1.1010),
                    close: dec!(1.1025),
                },
                volume: 100,
                complete: true,
            },
            // Candle 2: low breaches SL at 1.0950 (mid.low = 1.0940, BID low = 1.0940 - spread)
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.1020),
                    high: dec!(1.1025),
                    low: dec!(1.0940),
                    close: dec!(1.0960),
                },
                volume: 100,
                complete: true,
            },
            // Candle 3: should not be reached if SL hit
            Candle {
                time: base_time + Duration::hours(3),
                mid: Ohlc {
                    open: dec!(1.0960),
                    high: dec!(1.0970),
                    low: dec!(1.0950),
                    close: dec!(1.0965),
                },
                volume: 100,
                complete: true,
            },
        ]
    }

    #[test]
    fn test_stop_loss_enforcement() {
        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);
        // SL at 1.0950, no TP
        let mut strategy = SlTpTestStrategy::new(Some(dec!(1.0950)), None);
        let candles = sl_tp_test_candles();

        let result = engine.run(&mut strategy, &candles);

        // Should have exactly 1 trade (entered and stopped out)
        assert_eq!(result.trades.len(), 1, "Expected 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert_eq!(trade.exit_reason.as_deref(), Some("Stop Loss"));
        assert_eq!(trade.exit_price, Some(dec!(1.0950)));
        assert!(trade.pnl < Decimal::ZERO, "SL trade should be a loss");
    }

    #[test]
    fn test_take_profit_enforcement() {
        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);

        // TP at 1.1025, no SL. Candle 1 mid.high=1.1030, BID high = 1.1030 - 0.0001 = 1.1029 >= 1.1025
        let mut strategy = SlTpTestStrategy::new(None, Some(dec!(1.1025)));
        let candles = sl_tp_test_candles();

        let result = engine.run(&mut strategy, &candles);

        assert_eq!(result.trades.len(), 1, "Expected 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert_eq!(trade.exit_reason.as_deref(), Some("Take Profit"));
        assert_eq!(trade.exit_price, Some(dec!(1.1025)));
    }

    #[test]
    fn test_sl_priority_over_tp() {
        // Need a candle where both SL and TP are breached on the same bar
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let candles = vec![
            // Candle 0: signal generated (Buy)
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1010),
                    low: dec!(1.0990),
                    close: dec!(1.1005),
                },
                volume: 100,
                complete: true,
            },
            // Candle 1: entry at open (1.1020 + spread = 1.10201). Normal candle.
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.1020),
                    high: dec!(1.1025),
                    low: dec!(1.1015),
                    close: dec!(1.1020),
                },
                volume: 100,
                complete: true,
            },
            // Candle 2: huge range bar - both SL and TP breached
            // mid.high = 1.1200 => BID high = 1.1200 - 0.0001 = 1.1199 >= TP (1.1100) => hit
            // mid.low = 1.0800 => BID low = 1.0800 - 0.0001 = 1.0799 <= SL (1.0900) => hit
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.1020),
                    high: dec!(1.1200),
                    low: dec!(1.0800),
                    close: dec!(1.0900),
                },
                volume: 100,
                complete: true,
            },
        ];

        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);

        // SL at 1.0900, TP at 1.1100 — both breached on candle 2
        let mut strategy = SlTpTestStrategy::new(Some(dec!(1.0900)), Some(dec!(1.1100)));

        let result = engine.run(&mut strategy, &candles);

        // SL should take priority when both breach on the same candle
        assert_eq!(result.trades.len(), 1, "Expected 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert_eq!(trade.exit_reason.as_deref(), Some("Stop Loss"),
            "SL should take priority over TP when both hit same candle");
        assert_eq!(trade.exit_price, Some(dec!(1.0900)));
    }

    // ============================================================================
    // Pending Order Tests
    // ============================================================================

    /// Strategy that creates a pending order (buy stop / sell stop / buy limit / sell limit)
    /// on the first candle, then holds.
    struct PendingOrderTestStrategy {
        pending_order: PendingOrderInfo,
        stop_loss: Option<Decimal>,
        take_profit: Option<Decimal>,
        entered: bool,
        signal: Signal,
    }

    impl PendingOrderTestStrategy {
        fn new(
            pending_order: PendingOrderInfo,
            signal: Signal,
            stop_loss: Option<Decimal>,
            take_profit: Option<Decimal>,
        ) -> Self {
            Self { pending_order, stop_loss, take_profit, entered: false, signal }
        }
    }

    impl Strategy for PendingOrderTestStrategy {
        fn on_candle(&mut self, _candle: &Candle) -> Signal {
            Signal::Hold // unused, we use on_candle_extended
        }

        fn on_candle_extended(&mut self, _candle: &Candle) -> ExtendedSignal {
            if !self.entered {
                self.entered = true;
                ExtendedSignal {
                    signal: self.signal,
                    stop_loss: self.stop_loss,
                    take_profit: self.take_profit,
                    pending_order: Some(self.pending_order.clone()),
                    ..ExtendedSignal::default()
                }
            } else {
                ExtendedSignal::default()
            }
        }

        fn name(&self) -> &str { "PendingOrderTestStrategy" }

        fn reset(&mut self) {
            self.entered = false;
        }
    }

    fn pending_order_test_candles() -> Vec<Candle> {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        vec![
            // Candle 0: signal generated here (pending order created)
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1010),
                    low: dec!(1.0990),
                    close: dec!(1.1005),
                },
                volume: 100,
                complete: true,
            },
            // Candle 1: price rises - buy stop at 1.1050 should NOT fill (high = 1.1030)
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.1010),
                    high: dec!(1.1030),
                    low: dec!(1.1000),
                    close: dec!(1.1020),
                },
                volume: 100,
                complete: true,
            },
            // Candle 2: price rises more - buy stop at 1.1050 should fill (high = 1.1060)
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.1025),
                    high: dec!(1.1060),
                    low: dec!(1.1015),
                    close: dec!(1.1050),
                },
                volume: 100,
                complete: true,
            },
            // Candle 3: after fill, position should be open
            Candle {
                time: base_time + Duration::hours(3),
                mid: Ohlc {
                    open: dec!(1.1055),
                    high: dec!(1.1070),
                    low: dec!(1.1040),
                    close: dec!(1.1060),
                },
                volume: 100,
                complete: true,
            },
        ]
    }

    #[test]
    fn test_buy_stop_order_fills() {
        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);

        // Buy stop at 1.1050 — should fill on candle 2 (high 1.1060 + spread > 1.1050)
        let pending = PendingOrderInfo {
            order_type: EntryOrderType::BuyStop,
            price: dec!(1.1050),
            expiry_bars: None,
        };
        let mut strategy = PendingOrderTestStrategy::new(
            pending,
            Signal::Buy,
            Some(dec!(1.0950)), // SL
            Some(dec!(1.1150)), // TP
        );
        let candles = pending_order_test_candles();

        let result = engine.run(&mut strategy, &candles);

        // Should have at least 1 trade (opened via pending order fill, closed at end)
        assert!(result.trades.len() >= 1, "Expected at least 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert!(trade.is_long, "Buy stop should open a long position");
        // Entry price is the order price (spread already accounted for in should_fill trigger)
        assert_eq!(trade.entry_price, dec!(1.1050), "Entry should be at pending order price");
    }

    #[test]
    fn test_sell_stop_order_fills() {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let candles = vec![
            // Candle 0: signal generated
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1010),
                    low: dec!(1.0990),
                    close: dec!(1.0995),
                },
                volume: 100,
                complete: true,
            },
            // Candle 1: price drops - sell stop at 1.0900 should fill (low = 1.0850)
            // BID low = 1.0850 - 0.0001 = 1.0849 <= 1.0900 -> fill
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.0990),
                    high: dec!(1.1000),
                    low: dec!(1.0850),
                    close: dec!(1.0880),
                },
                volume: 100,
                complete: true,
            },
            // Candle 2: after fill
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.0870),
                    high: dec!(1.0890),
                    low: dec!(1.0860),
                    close: dec!(1.0875),
                },
                volume: 100,
                complete: true,
            },
        ];

        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);

        let pending = PendingOrderInfo {
            order_type: EntryOrderType::SellStop,
            price: dec!(1.0900),
            expiry_bars: None,
        };
        let mut strategy = PendingOrderTestStrategy::new(
            pending,
            Signal::Sell,
            Some(dec!(1.1000)), // SL
            Some(dec!(1.0800)), // TP
        );

        let result = engine.run(&mut strategy, &candles);

        assert!(result.trades.len() >= 1, "Expected at least 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert!(!trade.is_long, "Sell stop should open a short position");
        // Entry price is the order price (spread already accounted for in should_fill trigger)
        assert_eq!(trade.entry_price, dec!(1.0900), "Entry should be at sell stop price");
    }

    #[test]
    fn test_buy_limit_order_fills() {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let candles = vec![
            // Candle 0: signal generated
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1010),
                    low: dec!(1.0990),
                    close: dec!(1.1005),
                },
                volume: 100,
                complete: true,
            },
            // Candle 1: price drops - buy limit at 1.0900 should fill
            // ASK low = 1.0850 + 0.0001 = 1.0851 <= 1.0900 -> fill
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.0990),
                    high: dec!(1.1000),
                    low: dec!(1.0850),
                    close: dec!(1.0870),
                },
                volume: 100,
                complete: true,
            },
            // Candle 2: after fill
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.0880),
                    high: dec!(1.0950),
                    low: dec!(1.0870),
                    close: dec!(1.0940),
                },
                volume: 100,
                complete: true,
            },
        ];

        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);

        let pending = PendingOrderInfo {
            order_type: EntryOrderType::BuyLimit,
            price: dec!(1.0900),
            expiry_bars: None,
        };
        let mut strategy = PendingOrderTestStrategy::new(
            pending,
            Signal::Buy,
            Some(dec!(1.0800)), // SL
            Some(dec!(1.1000)), // TP
        );

        let result = engine.run(&mut strategy, &candles);

        assert!(result.trades.len() >= 1, "Expected at least 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert!(trade.is_long, "Buy limit should open a long position");
        // Entry price is the order price (spread already accounted for in should_fill trigger)
        assert_eq!(trade.entry_price, dec!(1.0900), "Entry should be at buy limit price");
    }

    #[test]
    fn test_sell_limit_order_fills() {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let candles = vec![
            // Candle 0: signal generated
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1010),
                    low: dec!(1.0990),
                    close: dec!(1.1005),
                },
                volume: 100,
                complete: true,
            },
            // Candle 1: price rises - sell limit at 1.1050 should fill
            // BID high = 1.1080 - 0.0001 = 1.1079 >= 1.1050 -> fill
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.1010),
                    high: dec!(1.1080),
                    low: dec!(1.1000),
                    close: dec!(1.1060),
                },
                volume: 100,
                complete: true,
            },
            // Candle 2: after fill
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.1055),
                    high: dec!(1.1060),
                    low: dec!(1.1030),
                    close: dec!(1.1040),
                },
                volume: 100,
                complete: true,
            },
        ];

        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);

        let pending = PendingOrderInfo {
            order_type: EntryOrderType::SellLimit,
            price: dec!(1.1050),
            expiry_bars: None,
        };
        let mut strategy = PendingOrderTestStrategy::new(
            pending,
            Signal::Sell,
            Some(dec!(1.1150)), // SL
            Some(dec!(1.0950)), // TP
        );

        let result = engine.run(&mut strategy, &candles);

        assert!(result.trades.len() >= 1, "Expected at least 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert!(!trade.is_long, "Sell limit should open a short position");
        // Entry price is the order price (spread already accounted for in should_fill trigger)
        assert_eq!(trade.entry_price, dec!(1.1050), "Entry should be at sell limit price");
    }

    #[test]
    fn test_pending_order_expires_after_n_bars() {
        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);

        // Buy stop at 1.1100 with expiry of 2 bars — price never reaches it
        let pending = PendingOrderInfo {
            order_type: EntryOrderType::BuyStop,
            price: dec!(1.1100),
            expiry_bars: Some(2),
        };
        let mut strategy = PendingOrderTestStrategy::new(
            pending,
            Signal::Buy,
            Some(dec!(1.0950)),
            Some(dec!(1.1200)),
        );
        let candles = pending_order_test_candles(); // highs are 1.1010, 1.1030, 1.1060, 1.1070

        let result = engine.run(&mut strategy, &candles);

        // No trades should be opened — order should have expired before price reached 1.1100
        assert_eq!(result.trades.len(), 0, "Order should expire without filling, got {} trades", result.trades.len());
    }

    #[test]
    fn test_pending_order_not_filled_when_price_doesnt_reach() {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Price stays in narrow range, never reaches buy stop at 1.2000
        let candles = vec![
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1010),
                    low: dec!(1.0990),
                    close: dec!(1.1005),
                },
                volume: 100,
                complete: true,
            },
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.1005),
                    high: dec!(1.1015),
                    low: dec!(1.0995),
                    close: dec!(1.1010),
                },
                volume: 100,
                complete: true,
            },
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.1010),
                    high: dec!(1.1020),
                    low: dec!(1.1000),
                    close: dec!(1.1015),
                },
                volume: 100,
                complete: true,
            },
        ];

        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);

        let pending = PendingOrderInfo {
            order_type: EntryOrderType::BuyStop,
            price: dec!(1.2000), // way above current price
            expiry_bars: None,
        };
        let mut strategy = PendingOrderTestStrategy::new(
            pending,
            Signal::Buy,
            None,
            None,
        );

        let result = engine.run(&mut strategy, &candles);

        assert_eq!(result.trades.len(), 0, "Order should never fill, got {} trades", result.trades.len());
    }

    #[test]
    fn test_market_order_still_works_no_regression() {
        // Exact same test as test_stop_loss_enforcement — market orders unchanged
        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);
        // SL at 1.0950, no TP — market order (no pending_order on ExtendedSignal)
        let mut strategy = SlTpTestStrategy::new(Some(dec!(1.0950)), None);
        let candles = sl_tp_test_candles();

        let result = engine.run(&mut strategy, &candles);

        // Should have exactly 1 trade (entered and stopped out) — same as before
        assert_eq!(result.trades.len(), 1, "Expected 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert_eq!(trade.exit_reason.as_deref(), Some("Stop Loss"));
        assert_eq!(trade.exit_price, Some(dec!(1.0950)));
        assert!(trade.pnl < Decimal::ZERO, "SL trade should be a loss");
    }

    /// Strategy that tracks internal position state and only enters when it thinks
    /// it's flat. This mimics RulesEngine's behavior where `self.position` gates
    /// whether entry rules are evaluated. Without notify_position_closed(), after
    /// the engine closes via SL/TP, this strategy's internal state stays "in position"
    /// and blocks all future entries.
    struct PositionSyncStrategy {
        internal_position: bool,
        signal_count: u32,
        notify_count: u32,
    }

    impl PositionSyncStrategy {
        fn new() -> Self {
            Self {
                internal_position: false,
                signal_count: 0,
                notify_count: 0,
            }
        }
    }

    impl Strategy for PositionSyncStrategy {
        fn on_candle(&mut self, _candle: &Candle) -> Signal {
            Signal::Hold
        }

        fn on_candle_extended(&mut self, candle: &Candle) -> ExtendedSignal {
            // Only generate entry if we think we're flat
            if !self.internal_position && candle.is_bullish() {
                self.internal_position = true;
                self.signal_count += 1;
                ExtendedSignal {
                    signal: Signal::Buy,
                    stop_loss: Some(candle.mid.low - dec!(0.0010)),
                    take_profit: None,
                    ..ExtendedSignal::default()
                }
            } else {
                ExtendedSignal::default()
            }
        }

        fn notify_position_closed(&mut self) {
            self.internal_position = false;
            self.notify_count += 1;
        }

        fn name(&self) -> &str { "PositionSyncStrategy" }

        fn reset(&mut self) {
            self.internal_position = false;
            self.signal_count = 0;
            self.notify_count = 0;
        }
    }

    #[test]
    fn test_position_sync_after_sl_close() {
        // Verify that after the engine closes a position via SL, the strategy's
        // internal position state is cleared so it can enter again.
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let candles = vec![
            // Candle 0: bullish — signal Buy with SL at 1.0970
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1020),
                    low: dec!(1.0980),
                    close: dec!(1.1015),
                },
                volume: 100,
                complete: true,
            },
            // Candle 1: entry at open 1.1010 + spread. Normal candle, no SL breach.
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.1010),
                    high: dec!(1.1030),
                    low: dec!(1.1000),
                    close: dec!(1.1020),
                },
                volume: 100,
                complete: true,
            },
            // Candle 2: SL breach — low drops well below SL (1.0970)
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.1010),
                    high: dec!(1.1015),
                    low: dec!(1.0950),
                    close: dec!(1.0960),
                },
                volume: 100,
                complete: true,
            },
            // Candle 3: bullish again — should be able to enter a NEW trade
            Candle {
                time: base_time + Duration::hours(3),
                mid: Ohlc {
                    open: dec!(1.0960),
                    high: dec!(1.0990),
                    low: dec!(1.0950),
                    close: dec!(1.0985),
                },
                volume: 100,
                complete: true,
            },
            // Candle 4: new entry executes here at open
            Candle {
                time: base_time + Duration::hours(4),
                mid: Ohlc {
                    open: dec!(1.0980),
                    high: dec!(1.1010),
                    low: dec!(1.0970),
                    close: dec!(1.1000),
                },
                volume: 100,
                complete: true,
            },
        ];

        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(1000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);
        let mut strategy = PositionSyncStrategy::new();

        let result = engine.run(&mut strategy, &candles);

        // The strategy should have generated at least 2 entry signals:
        // one for the first trade, and one after the SL close
        assert!(strategy.signal_count >= 2,
            "Strategy should have entered at least twice, but only entered {} time(s). \
             notify_position_closed is likely not being called after SL close.",
            strategy.signal_count);

        // notify_position_closed should have been called at least once (for the SL close)
        assert!(strategy.notify_count >= 1,
            "notify_position_closed was never called — engine/strategy position desync bug");

        // Should have at least 2 trades (first closed by SL, second open or closed at end)
        assert!(result.trades.len() >= 2,
            "Expected at least 2 trades but got {}. Without position sync, the strategy \
             gets stuck after the first SL close.",
            result.trades.len());

        // First trade should have been closed by SL
        assert_eq!(result.trades[0].exit_reason.as_deref(), Some("Stop Loss"));
    }

    #[test]
    fn test_wrong_side_sl_skips_long_entry() {
        // If the SL (computed on signal candle) ends up >= the actual entry price
        // due to a gap, the trade should be skipped entirely.
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Strategy that enters long with SL at 1.1005 (just above close on signal candle)
        // Entry candle gaps DOWN so entry_price < SL — should skip.
        struct WrongSideSLStrategy {
            entered: bool,
        }
        impl Strategy for WrongSideSLStrategy {
            fn on_candle(&mut self, _candle: &Candle) -> Signal { Signal::Hold }
            fn on_candle_extended(&mut self, _candle: &Candle) -> ExtendedSignal {
                if !self.entered {
                    self.entered = true;
                    ExtendedSignal {
                        signal: Signal::Buy,
                        // SL above where entry will actually happen (gap down)
                        stop_loss: Some(dec!(1.1005)),
                        take_profit: None,
                        ..ExtendedSignal::default()
                    }
                } else {
                    ExtendedSignal::default()
                }
            }
            fn name(&self) -> &str { "WrongSideSLStrategy" }
            fn reset(&mut self) { self.entered = false; }
        }

        let candles = vec![
            // Candle 0: signal candle. Close at 1.1010, SL set at 1.1005 (below close — valid on signal candle).
            Candle {
                time: base_time,
                mid: Ohlc {
                    open: dec!(1.1000),
                    high: dec!(1.1020),
                    low: dec!(1.0990),
                    close: dec!(1.1010),
                },
                volume: 100,
                complete: true,
            },
            // Candle 1: GAP DOWN — opens at 1.0990. Entry = 1.0990 + 0.0001 spread = 1.0991.
            // SL is 1.1005, which is ABOVE entry. Trade should be SKIPPED.
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc {
                    open: dec!(1.0990),
                    high: dec!(1.1000),
                    low: dec!(1.0980),
                    close: dec!(1.0995),
                },
                volume: 100,
                complete: true,
            },
            // Candle 2: filler
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc {
                    open: dec!(1.0995),
                    high: dec!(1.1010),
                    low: dec!(1.0990),
                    close: dec!(1.1005),
                },
                volume: 100,
                complete: true,
            },
        ];

        let config = BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(1000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        };
        let engine = BacktestEngine::new(config);
        let mut strategy = WrongSideSLStrategy { entered: false };

        let result = engine.run(&mut strategy, &candles);

        // The trade should be skipped entirely — no trades at all
        assert_eq!(result.trades.len(), 0,
            "Expected 0 trades (wrong-side SL should skip entry), got {}",
            result.trades.len());
    }

    // ============================================================================
    // Scripted Strategy Pending Order Tests (AGT-607)
    //
    // Confirms `pending_order` maps returned from Rhai `on_candle()` are parsed by
    // ScriptedStrategy::parse_rhai_result and fill through BacktestEngine::run() with
    // the exact same PendingOrder::should_fill semantics as native Rust strategies.
    // These reuse the deterministic candle fixtures from the native pending-order
    // tests above so the expected fill prices/directions are already hand-verified.
    // ============================================================================

    fn pending_order_backtest_config() -> BacktestConfig {
        BacktestConfig {
            initial_balance: dec!(10000),
            position_size: dec!(10000),
            spread_pips: dec!(1),
            pip_value: dec!(0.0001),
            ..BacktestConfig::default()
        }
    }

    #[test]
    fn test_scripted_buy_stop_pending_order_fills() {
        let script = r#"
let placed = false;

fn on_candle() {
    if !placed {
        placed = true;
        return #{
            signal: "buy",
            stop_loss: 1.0950,
            take_profit: 1.1150,
            pending_order: #{ order_type: "buy_stop", price: 1.1050 }
        };
    }
    #{ signal: "hold" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "script_buy_stop").unwrap();
        let candles = pending_order_test_candles();
        let engine = BacktestEngine::new(pending_order_backtest_config());

        let result = engine.run(&mut strategy, &candles);

        assert!(!result.trades.is_empty(), "Expected at least 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert!(trade.is_long, "Buy stop should open a long position");
        assert_eq!(trade.entry_price, dec!(1.1050), "Entry should be at the script's pending order price");
    }

    #[test]
    fn test_scripted_sell_stop_pending_order_fills() {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let candles = vec![
            Candle {
                time: base_time,
                mid: Ohlc { open: dec!(1.1000), high: dec!(1.1010), low: dec!(1.0990), close: dec!(1.0995) },
                volume: 100,
                complete: true,
            },
            // Price drops - sell stop at 1.0900 should fill (BID low = 1.0850 - 0.0001 <= 1.0900)
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc { open: dec!(1.0990), high: dec!(1.1000), low: dec!(1.0850), close: dec!(1.0880) },
                volume: 100,
                complete: true,
            },
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc { open: dec!(1.0870), high: dec!(1.0890), low: dec!(1.0860), close: dec!(1.0875) },
                volume: 100,
                complete: true,
            },
        ];

        let script = r#"
let placed = false;

fn on_candle() {
    if !placed {
        placed = true;
        return #{
            signal: "sell",
            stop_loss: 1.1000,
            take_profit: 1.0800,
            pending_order: #{ order_type: "sell_stop", price: 1.0900 }
        };
    }
    #{ signal: "hold" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "script_sell_stop").unwrap();
        let engine = BacktestEngine::new(pending_order_backtest_config());

        let result = engine.run(&mut strategy, &candles);

        assert!(!result.trades.is_empty(), "Expected at least 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert!(!trade.is_long, "Sell stop should open a short position");
        assert_eq!(trade.entry_price, dec!(1.0900), "Entry should be at the script's pending order price");
    }

    #[test]
    fn test_scripted_buy_limit_pending_order_fills() {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let candles = vec![
            Candle {
                time: base_time,
                mid: Ohlc { open: dec!(1.1000), high: dec!(1.1010), low: dec!(1.0990), close: dec!(1.1005) },
                volume: 100,
                complete: true,
            },
            // Price drops - buy limit at 1.0900 should fill (ASK low = 1.0850 + 0.0001 <= 1.0900)
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc { open: dec!(1.0990), high: dec!(1.1000), low: dec!(1.0850), close: dec!(1.0870) },
                volume: 100,
                complete: true,
            },
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc { open: dec!(1.0880), high: dec!(1.0950), low: dec!(1.0870), close: dec!(1.0940) },
                volume: 100,
                complete: true,
            },
        ];

        let script = r#"
let placed = false;

fn on_candle() {
    if !placed {
        placed = true;
        return #{
            signal: "buy",
            stop_loss: 1.0800,
            take_profit: 1.1000,
            pending_order: #{ order_type: "buy_limit", price: 1.0900 }
        };
    }
    #{ signal: "hold" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "script_buy_limit").unwrap();
        let engine = BacktestEngine::new(pending_order_backtest_config());

        let result = engine.run(&mut strategy, &candles);

        assert!(!result.trades.is_empty(), "Expected at least 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert!(trade.is_long, "Buy limit should open a long position");
        assert_eq!(trade.entry_price, dec!(1.0900), "Entry should be at the script's pending order price");
    }

    #[test]
    fn test_scripted_sell_limit_pending_order_fills() {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let candles = vec![
            Candle {
                time: base_time,
                mid: Ohlc { open: dec!(1.1000), high: dec!(1.1010), low: dec!(1.0990), close: dec!(1.1005) },
                volume: 100,
                complete: true,
            },
            // Price rises - sell limit at 1.1050 should fill (BID high = 1.1080 - 0.0001 >= 1.1050)
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc { open: dec!(1.1010), high: dec!(1.1080), low: dec!(1.1000), close: dec!(1.1060) },
                volume: 100,
                complete: true,
            },
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc { open: dec!(1.1055), high: dec!(1.1060), low: dec!(1.1030), close: dec!(1.1040) },
                volume: 100,
                complete: true,
            },
        ];

        let script = r#"
let placed = false;

fn on_candle() {
    if !placed {
        placed = true;
        return #{
            signal: "sell",
            stop_loss: 1.1150,
            take_profit: 1.0950,
            pending_order: #{ order_type: "sell_limit", price: 1.1050 }
        };
    }
    #{ signal: "hold" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "script_sell_limit").unwrap();
        let engine = BacktestEngine::new(pending_order_backtest_config());

        let result = engine.run(&mut strategy, &candles);

        assert!(!result.trades.is_empty(), "Expected at least 1 trade, got {}", result.trades.len());
        let trade = &result.trades[0];
        assert!(!trade.is_long, "Sell limit should open a short position");
        assert_eq!(trade.entry_price, dec!(1.1050), "Entry should be at the script's pending order price");
    }

    #[test]
    fn test_scripted_pending_order_self_tracks_position_via_on_position_closed() {
        // The Rhai ABI gives scripts no engine-provided "am I in a position?" state, so
        // scripts must self-track via a state variable that only resets in
        // on_position_closed(). This confirms the full round trip: a script-emitted
        // buy-stop pending order fills, the position is later stopped out, the script's
        // on_position_closed() callback fires and clears its internal flag, and the
        // script is then able to arm a second pending order.
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let candles = vec![
            // Candle 0: script arms a buy-stop pending order at 1.1050
            Candle {
                time: base_time,
                mid: Ohlc { open: dec!(1.1000), high: dec!(1.1010), low: dec!(1.0990), close: dec!(1.1005) },
                volume: 100,
                complete: true,
            },
            // Candle 1: buy stop fills (high 1.1060 + spread >= 1.1050)
            Candle {
                time: base_time + Duration::hours(1),
                mid: Ohlc { open: dec!(1.1025), high: dec!(1.1060), low: dec!(1.1015), close: dec!(1.1050) },
                volume: 100,
                complete: true,
            },
            // Candle 2: price collapses through the stop loss (1.0950)
            Candle {
                time: base_time + Duration::hours(2),
                mid: Ohlc { open: dec!(1.1040), high: dec!(1.1045), low: dec!(1.0900), close: dec!(1.0910) },
                volume: 100,
                complete: true,
            },
            // Candle 3: script re-arms a second buy-stop at 1.0950 (only possible if
            // on_position_closed() reset its internal `in_position` flag)
            Candle {
                time: base_time + Duration::hours(3),
                mid: Ohlc { open: dec!(1.0910), high: dec!(1.0920), low: dec!(1.0900), close: dec!(1.0915) },
                volume: 100,
                complete: true,
            },
            // Candle 4: second buy stop fills (high 1.0970 + spread >= 1.0950)
            Candle {
                time: base_time + Duration::hours(4),
                mid: Ohlc { open: dec!(1.0920), high: dec!(1.0970), low: dec!(1.0910), close: dec!(1.0960) },
                volume: 100,
                complete: true,
            },
        ];

        let script = r#"
let in_position = false;
let entries = 0;

fn on_candle() {
    if in_position {
        return #{ signal: "hold" };
    }

    in_position = true;
    entries += 1;

    if entries == 1 {
        return #{
            signal: "buy",
            stop_loss: 1.0950,
            pending_order: #{ order_type: "buy_stop", price: 1.1050 }
        };
    }

    #{
        signal: "buy",
        stop_loss: 1.0850,
        pending_order: #{ order_type: "buy_stop", price: 1.0950 }
    }
}

fn on_position_closed() {
    in_position = false;
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "script_self_tracking").unwrap();
        let engine = BacktestEngine::new(pending_order_backtest_config());

        let result = engine.run(&mut strategy, &candles);

        assert_eq!(result.trades.len(), 2,
            "Expected 2 trades (first stopped out, second re-armed via on_position_closed), got {}",
            result.trades.len());
        assert_eq!(result.trades[0].entry_price, dec!(1.1050));
        assert_eq!(result.trades[0].exit_reason.as_deref(), Some("Stop Loss"));
        assert_eq!(result.trades[1].entry_price, dec!(1.0950));
    }

    // Regression: JPY-quoted pairs must book P&L in the USD home currency, not
    // raw yen (~150x), which otherwise swamps any cross-instrument sum and made
    // USD_JPY backtests read as +190% winners.
    #[test]
    fn pnl_converts_quote_currency_to_usd_home() {
        // USD_JPY: quote is JPY. 200 yen of quote P&L at 150 JPY/USD ≈ 1.33 USD.
        let mut jpy = BacktestConfig::default();
        jpy.instrument = "USD_JPY".to_string();
        let eng = BacktestEngine::new(jpy);
        assert_eq!(eng.to_home_currency(dec!(200), dec!(150.0)), dec!(200) / dec!(150.0));

        // USD-quoted pair: already USD, no conversion.
        let mut eur = BacktestConfig::default();
        eur.instrument = "EUR_USD".to_string();
        assert_eq!(BacktestEngine::new(eur).to_home_currency(dec!(94.4), dec!(1.18)), dec!(94.4));

        // Unknown/empty instrument: legacy behavior (no conversion).
        assert_eq!(BacktestEngine::new(BacktestConfig::default()).to_home_currency(dec!(5), dec!(150.0)), dec!(5));
    }

    // Regression: the Sharpe ratio must annualize by the ACTUAL candle span, not
    // a hardcoded sqrt(252). The same per-period returns spanning 1 year vs 4
    // years should differ by exactly sqrt(4)=2x (4x the periods-per-year). The
    // old code returned identical values regardless of span (it understated
    // every intraday timeframe).
    #[test]
    fn sharpe_annualizes_by_actual_span() {
        fn candle_at(t: DateTime<Utc>) -> Candle {
            Candle { time: t, mid: Ohlc { open: dec!(1), high: dec!(1), low: dec!(1), close: dec!(1) },
                     volume: 0, complete: true }
        }
        let eng = BacktestEngine::new(BacktestConfig::default());
        let equity = vec![dec!(100), dec!(110), dec!(100), dec!(110), dec!(100), dec!(112)];
        let t0 = DateTime::parse_from_rfc3339("2022-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        let s_1yr = eng.calculate_sharpe_ratio(&equity, &[candle_at(t0), candle_at(t0 + Duration::days(365))]);
        let s_4yr = eng.calculate_sharpe_ratio(&equity, &[candle_at(t0), candle_at(t0 + Duration::days(365 * 4))]);
        // 4x the periods/year -> sqrt(4)=2x the annualized Sharpe.
        let ratio = s_1yr / s_4yr;
        assert!((ratio - dec!(2)).abs() < dec!(0.02), "expected ~2x, got {ratio} (s1={s_1yr}, s4={s_4yr})");
    }
}
