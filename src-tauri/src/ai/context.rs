//! Chat context types for different window types
//!
//! Each window type provides different contextual information to the AI.

use serde::{Deserialize, Serialize};

/// Context passed from the frontend to provide relevant information for AI responses.
/// Each variant corresponds to a window type in the application.
/// Note: We use lowercase variant names where possible and explicit rename for multi-word variants
/// to keep field names in snake_case (matching TypeScript).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChatContext {
    /// Account overview window
    #[serde(rename = "account")]
    Account {
        balance: Option<String>,
        unrealized_pl: Option<String>,
        open_trade_count: Option<u32>,
        environment: String, // "demo" or "live"
    },

    /// Charting window
    #[serde(rename = "charting")]
    Charting {
        instrument: String,
        granularity: String,
        strategy_name: Option<String>,
        strategy_id: Option<String>,
        strategy_risk_settings: Option<serde_json::Value>,
        indicators: Vec<String>,
        indicator_values: Option<std::collections::HashMap<String, String>>,
        current_price: Option<String>,
        signal_direction: Option<String>, // "long", "short", or null
    },

    /// Backtesting/Research window
    #[serde(rename = "backtesting")]
    Backtesting {
        /// Strategy ID for AI tool queries
        strategy_id: Option<String>,
        strategy_name: Option<String>,
        strategy_description: Option<String>,
        strategy_risk_settings: Option<serde_json::Value>,
        /// "rules" or "scripted"
        strategy_type: Option<String>,
        /// Rhai script source (for scripted strategies)
        script_content: Option<String>,
        methodology: Option<String>, // "simple", "walk_forward", "anchored_walk_forward"
        parameters: Vec<ParameterInfo>,
        has_results: bool,
        /// Backtest job ID for walk-forward tests (AI can use this to fetch results)
        backtest_job_id: Option<String>,
        metrics_summary: Option<String>,
        /// Holdout validation results (if available)
        holdout_summary: Option<String>,
        /// Human-readable strategy entry/exit rules
        strategy_rules: Option<String>,
        /// Full parameter definitions with min/max/step
        parameter_definitions: Option<String>,
        /// Per-window walk-forward results summary
        window_summary: Option<String>,
        /// Details of the currently viewed walk-forward window
        selected_window: Option<String>,
    },

    /// Trading ticket window
    #[serde(rename = "ticket")]
    Ticket {
        instrument: String,
        direction: Option<String>, // "buy" or "sell"
        units: Option<String>,
        stop_loss: Option<String>,
        take_profit: Option<String>,
        current_price: Option<String>,
        strategy_name: Option<String>,
        strategy_risk_settings: Option<serde_json::Value>,
    },

    /// Strategy watcher window
    #[serde(rename = "watcher")]
    Watcher {
        running_strategies: Vec<WatcherInfo>,
        pending_signals: Vec<SignalInfo>,
        /// User's configured symbols list from settings
        available_instruments: Option<Vec<String>>,
    },

    /// Trade analysis window
    #[serde(rename = "tradeAnalysis")]
    TradeAnalysis {
        trade_count: u32,
        date_range: Option<String>,
        win_rate: Option<String>,
        profit_factor: Option<String>,
        filters_active: bool,
        /// Which breakdown tab is currently active (session, day, hour, instrument, etc.)
        active_breakdown: Option<String>,
    },

    /// Trade review modal (single trade deep-dive)
    #[serde(rename = "tradeReview")]
    TradeReview {
        instrument: String,
        direction: String,
        is_winner: bool,
        entry_price: String,
        exit_price: String,
        realized_pl: String,
        duration_minutes: u32,
        // Trade quality metrics
        mae_pips: String,
        mfe_pips: String,
        capture_efficiency: Option<String>,
        r_multiple: Option<String>,
        // Entry timing
        immediate_drawdown_pips: String,
        candles_to_profit: Option<u32>,
        near_swing_point: Option<String>,
        // Market context at entry
        rsi_14: Option<String>,
        rsi_zone: Option<String>,
        trend: Option<String>,
        // Post-exit analysis
        post_exit_favorable_pips: String,
        post_exit_adverse_pips: String,
        // AI score if available
        ai_score: Option<TradeAIScore>,
        // Key insights from analysis
        key_insights: Vec<String>,
        // Indicator analysis from Score Trade (if available)
        indicator_analysis: Option<Vec<IndicatorAnalysis>>,
        // Indicators that conflicted with trade direction
        conflicting_indicators: Option<Vec<String>>,
    },

    /// Trade subset modal (filtered group of trades)
    #[serde(rename = "tradeSubset")]
    TradeSubset {
        /// Description of the subset (e.g., "Asian session trades")
        subset_description: String,
        trade_count: u32,
        wins: u32,
        losses: u32,
        win_rate: String,
        avg_win: String,
        avg_loss: String,
        expectancy: String,
        profit_factor: String,
        total_pl: String,
        /// List of instruments in this subset
        instruments: Vec<String>,
        /// Direction breakdown
        long_count: u32,
        short_count: u32,
    },

    /// Internal system operations (compaction, etc.)
    #[serde(rename = "internal")]
    Internal {
        /// Operation being performed
        operation: String,
    },
}

/// AI-generated trade score
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeAIScore {
    pub entry: u32,
    pub exit: u32,
    pub risk_management: u32,
    pub overall: u32,
}

/// Indicator analysis from Score Trade
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorAnalysis {
    pub indicator: String,
    pub assessment: String,
    pub supported_trade: bool,
    pub at_entry: String,
    pub at_exit: String,
}

/// Information about a strategy parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterInfo {
    pub name: String,
    pub current_value: String,
    pub default_value: Option<String>,
}

/// Information about a running strategy watcher
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherInfo {
    pub strategy_name: String,
    pub instruments: Vec<String>,
    pub timeframe: String,
}

/// Information about a pending signal
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalInfo {
    pub instrument: String,
    pub direction: String,
    pub strategy_name: String,
    pub entry_price: Option<String>,
}

impl ChatContext {
    /// Get a human-readable description of the context for the system prompt
    pub fn describe(&self) -> String {
        match self {
            ChatContext::Account {
                balance,
                unrealized_pl,
                open_trade_count,
                environment,
            } => {
                let mut parts = vec![format!("Environment: {}", environment)];
                if let Some(b) = balance {
                    parts.push(format!("Balance: {}", b));
                }
                if let Some(pl) = unrealized_pl {
                    parts.push(format!("Unrealized P/L: {}", pl));
                }
                if let Some(count) = open_trade_count {
                    parts.push(format!("Open trades: {}", count));
                }
                format!("ACCOUNT WINDOW\n{}", parts.join("\n"))
            }

            ChatContext::Charting {
                instrument,
                granularity,
                strategy_name,
                strategy_id: _,
                strategy_risk_settings,
                indicators,
                indicator_values,
                current_price,
                signal_direction,
            } => {
                let mut parts = vec![
                    format!("Instrument: {}", instrument),
                    format!("Timeframe: {}", granularity),
                ];
                if let Some(name) = strategy_name {
                    parts.push(format!("Strategy: {}", name));
                }
                if !indicators.is_empty() {
                    parts.push(format!("Indicators: {}", indicators.join(", ")));
                }
                if let Some(values) = indicator_values {
                    if !values.is_empty() {
                        let vals: Vec<String> = values.iter().map(|(k, v)| format!("  {}: {}", k, v)).collect();
                        parts.push(format!("Current indicator values:\n{}", vals.join("\n")));
                    }
                }
                if let Some(price) = current_price {
                    parts.push(format!("Current price: {}", price));
                }
                if let Some(dir) = signal_direction {
                    parts.push(format!("Signal: {}", dir));
                }
                if let Some(settings) = strategy_risk_settings {
                    if !settings.is_null() {
                        parts.push(format!("Strategy risk settings: {}", settings));
                    }
                }
                format!("CHART WINDOW\n{}", parts.join("\n"))
            }

            ChatContext::Backtesting {
                strategy_id,
                strategy_name,
                strategy_description,
                strategy_risk_settings,
                strategy_type,
                script_content,
                methodology,
                parameters,
                has_results,
                backtest_job_id,
                metrics_summary,
                holdout_summary,
                strategy_rules,
                parameter_definitions,
                window_summary,
                selected_window,
            } => {
                let mut parts = vec![];
                if let Some(name) = strategy_name {
                    parts.push(format!("Strategy: {}", name));
                }
                // Include strategy_id for AI tool calls
                if let Some(id) = strategy_id {
                    parts.push(format!("Strategy ID: {}", id));
                }
                if let Some(desc) = strategy_description {
                    parts.push(format!("Description: {}", desc));
                }
                if let Some(settings) = strategy_risk_settings {
                    if !settings.is_null() {
                        parts.push(format!("Strategy risk settings: {}", settings));
                    }
                }
                if let Some(st) = strategy_type {
                    if st == "scripted" {
                        parts.push("Strategy type: Scripted (Rhai)".to_string());
                        if let Some(script) = script_content {
                            let truncated = if script.len() > 2000 { &script[..2000] } else { script.as_str() };
                            parts.push(format!("Script:\n```rhai\n{}\n```", truncated));
                        }
                    }
                }
                if let Some(method) = methodology {
                    parts.push(format!("Methodology: {}", method));
                }
                if !parameters.is_empty() {
                    let param_strs: Vec<String> = parameters
                        .iter()
                        .map(|p| format!("  {} = {}", p.name, p.current_value))
                        .collect();
                    parts.push(format!("Parameters:\n{}", param_strs.join("\n")));
                }
                if *has_results {
                    parts.push("Results available: Yes".to_string());
                    // Include backtest job ID for AI tool calls
                    if let Some(job_id) = backtest_job_id {
                        parts.push(format!("Backtest Job ID: {} (use with get_backtest_results)", job_id));
                    }
                    if let Some(summary) = metrics_summary {
                        parts.push(format!("Metrics: {}", summary));
                    }
                }
                if let Some(defs) = parameter_definitions {
                    parts.push(format!("\n{}", defs));
                }
                if let Some(rules) = strategy_rules {
                    parts.push(format!("\n{}", rules));
                }
                if let Some(summary) = window_summary {
                    parts.push(format!("\n{}", summary));
                }
                if let Some(window) = selected_window {
                    parts.push(format!("\n{}", window));
                }
                if let Some(holdout) = holdout_summary {
                    parts.push(format!("\nHOLDOUT VALIDATION RESULTS:\n{}", holdout));
                }
                format!("RESEARCH WINDOW\n{}", parts.join("\n"))
            }

            ChatContext::Ticket {
                instrument,
                direction,
                units,
                stop_loss,
                take_profit,
                current_price,
                strategy_name,
                strategy_risk_settings,
            } => {
                let mut parts = vec![format!("Instrument: {}", instrument)];
                if let Some(dir) = direction {
                    parts.push(format!("Direction: {}", dir));
                }
                if let Some(u) = units {
                    parts.push(format!("Units: {}", u));
                }
                if let Some(sl) = stop_loss {
                    parts.push(format!("Stop Loss: {}", sl));
                }
                if let Some(tp) = take_profit {
                    parts.push(format!("Take Profit: {}", tp));
                }
                if let Some(price) = current_price {
                    parts.push(format!("Current price: {}", price));
                }
                if let Some(name) = strategy_name {
                    parts.push(format!("Strategy: {}", name));
                }
                if let Some(settings) = strategy_risk_settings {
                    if !settings.is_null() {
                        parts.push(format!("Strategy risk settings: {}", settings));
                    }
                }
                format!("TRADING TICKET WINDOW\n{}", parts.join("\n"))
            }

            ChatContext::Watcher {
                running_strategies,
                pending_signals,
                available_instruments,
            } => {
                let mut parts = vec![];
                if let Some(instruments) = available_instruments {
                    if !instruments.is_empty() {
                        parts.push(format!("User's symbol list: {}", instruments.join(", ")));
                    }
                }
                if !running_strategies.is_empty() {
                    let strat_strs: Vec<String> = running_strategies
                        .iter()
                        .map(|w| {
                            format!(
                                "  {} on {} ({})",
                                w.strategy_name,
                                w.instruments.join(", "),
                                w.timeframe
                            )
                        })
                        .collect();
                    parts.push(format!("Running strategies:\n{}", strat_strs.join("\n")));
                }
                if !pending_signals.is_empty() {
                    let sig_strs: Vec<String> = pending_signals
                        .iter()
                        .map(|s| {
                            format!(
                                "  {} {} on {} ({})",
                                s.direction,
                                s.instrument,
                                s.strategy_name,
                                s.entry_price.as_deref().unwrap_or("pending")
                            )
                        })
                        .collect();
                    parts.push(format!("Pending signals:\n{}", sig_strs.join("\n")));
                }
                if parts.is_empty() {
                    parts.push("No strategies running".to_string());
                }
                format!("STRATEGY WATCHER WINDOW\n{}", parts.join("\n"))
            }

            ChatContext::TradeAnalysis {
                trade_count,
                date_range,
                win_rate,
                profit_factor,
                filters_active,
                active_breakdown,
            } => {
                let mut parts = vec![format!("Trade count: {}", trade_count)];
                if let Some(range) = date_range {
                    parts.push(format!("Date range: {}", range));
                }
                if let Some(wr) = win_rate {
                    parts.push(format!("Win rate: {}", wr));
                }
                if let Some(pf) = profit_factor {
                    parts.push(format!("Profit factor: {}", pf));
                }
                if *filters_active {
                    parts.push("Filters active: Yes".to_string());
                }
                if let Some(breakdown) = active_breakdown {
                    parts.push(format!("Viewing breakdown: {}", breakdown));
                }
                format!("TRADE ANALYSIS WINDOW\n{}", parts.join("\n"))
            }

            ChatContext::TradeReview {
                instrument,
                direction,
                is_winner,
                entry_price,
                exit_price,
                realized_pl,
                duration_minutes,
                mae_pips,
                mfe_pips,
                capture_efficiency,
                r_multiple,
                immediate_drawdown_pips,
                candles_to_profit,
                near_swing_point,
                rsi_14,
                rsi_zone,
                trend,
                post_exit_favorable_pips,
                post_exit_adverse_pips,
                ai_score,
                key_insights,
                indicator_analysis,
                conflicting_indicators,
            } => {
                let result = if *is_winner { "WIN" } else { "LOSS" };
                let mut parts = vec![
                    format!("{} {} - {} (P/L: ${})", instrument, direction, result, realized_pl),
                    format!("Entry: {} → Exit: {}", entry_price, exit_price),
                    format!("Duration: {} minutes", duration_minutes),
                ];

                // Trade quality
                parts.push(format!("MAE: {} pips, MFE: {} pips", mae_pips, mfe_pips));
                if let Some(eff) = capture_efficiency {
                    parts.push(format!("Capture efficiency: {}%", eff));
                }
                if let Some(r) = r_multiple {
                    parts.push(format!("R-Multiple: {}R", r));
                }

                // Entry timing
                parts.push(format!("Initial drawdown: {} pips", immediate_drawdown_pips));
                if let Some(candles) = candles_to_profit {
                    parts.push(format!("Candles to profit: {}", candles));
                }
                if let Some(swing) = near_swing_point {
                    parts.push(format!("Near swing point: {}", swing));
                }

                // Market context
                let mut context_parts = vec![];
                if let Some(rsi) = rsi_14 {
                    let zone_str = rsi_zone.as_deref().map(|z| format!(" ({})", z)).unwrap_or_default();
                    context_parts.push(format!("RSI: {}{}", rsi, zone_str));
                }
                if let Some(t) = trend {
                    context_parts.push(format!("Trend: {}", t));
                }
                if !context_parts.is_empty() {
                    parts.push(format!("Market context: {}", context_parts.join(", ")));
                }

                // Post-exit
                parts.push(format!(
                    "Post-exit: +{} pips favorable, -{} pips adverse",
                    post_exit_favorable_pips, post_exit_adverse_pips
                ));

                // AI score if available
                if let Some(score) = ai_score {
                    parts.push(format!(
                        "AI Score - Entry: {}/10, Exit: {}/10, Risk: {}/10, Overall: {}/10",
                        score.entry, score.exit, score.risk_management, score.overall
                    ));
                }

                // Key insights
                if !key_insights.is_empty() {
                    parts.push(format!("Insights: {}", key_insights.join("; ")));
                }

                // Indicator analysis from Score Trade
                if let Some(indicators) = indicator_analysis {
                    if !indicators.is_empty() {
                        parts.push("\nINDICATOR ANALYSIS:".to_string());
                        for ind in indicators {
                            let status = if ind.supported_trade { "Supported" } else { "Conflicted" };
                            parts.push(format!(
                                "  {} ({}): {} | Entry: {} | Exit: {}",
                                ind.indicator, status, ind.assessment, ind.at_entry, ind.at_exit
                            ));
                        }
                    }
                }

                // Conflicting indicators summary
                if let Some(conflicts) = conflicting_indicators {
                    if !conflicts.is_empty() {
                        parts.push(format!("Conflicting indicators: {}", conflicts.join(", ")));
                    }
                }

                format!("TRADE REVIEW\n{}", parts.join("\n"))
            }

            ChatContext::TradeSubset {
                subset_description,
                trade_count,
                wins,
                losses,
                win_rate,
                avg_win,
                avg_loss,
                expectancy,
                profit_factor,
                total_pl,
                instruments,
                long_count,
                short_count,
            } => {
                let mut parts = vec![
                    format!("Subset: {}", subset_description),
                    format!("Trades: {} ({} wins, {} losses)", trade_count, wins, losses),
                    format!("Win rate: {}%", win_rate),
                    format!("Avg win: ${}, Avg loss: ${}", avg_win, avg_loss),
                    format!("Expectancy: ${}", expectancy),
                    format!("Profit factor: {}", profit_factor),
                    format!("Total P&L: ${}", total_pl),
                ];

                if !instruments.is_empty() {
                    parts.push(format!("Instruments: {}", instruments.join(", ")));
                }

                parts.push(format!("Direction: {} long, {} short", long_count, short_count));

                format!("TRADE SUBSET ANALYSIS\n{}", parts.join("\n"))
            }

            ChatContext::Internal { operation } => {
                format!("INTERNAL OPERATION: {}", operation)
            }
        }
    }

    /// Get the window type as a string
    pub fn window_type(&self) -> &'static str {
        match self {
            ChatContext::Account { .. } => "account",
            ChatContext::Charting { .. } => "charting",
            ChatContext::Backtesting { .. } => "backtesting",
            ChatContext::Ticket { .. } => "ticket",
            ChatContext::Watcher { .. } => "watcher",
            ChatContext::TradeAnalysis { .. } => "tradeanalysis",
            ChatContext::TradeReview { .. } => "tradereview",
            ChatContext::TradeSubset { .. } => "tradesubset",
            ChatContext::Internal { .. } => "internal",
        }
    }
}
