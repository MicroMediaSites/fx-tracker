//! `wickd candles` — historical OHLC candles from OANDA, as JSON.
//!
//!   wickd candles EUR_USD --granularity H1 --count 500
//!   wickd candles EUR_USD --granularity H4 --from 2024-01-01 --to 2024-03-01
//!   wickd candles EUR_USD --indicators ema:20,rsi:14
//!
//! This is the heart of the "data flowing in" model: the agent pulls candles
//! (optionally with a few common indicator columns) and reasons over them —
//! backtesting and pattern-matching happen in the agent, not here.

use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use serde_json::{json, Value};

use wickd_core::backtest::{
    AdxIndicator, AtrIndicator, EmaIndicator, Indicator, MfiIndicator, RsiIndicator, SmaIndicator,
};
use wickd_core::models::Candle;
use wickd_core::oanda::endpoints::{self, Granularity};
use wickd_core::oanda::OandaClient;

use crate::commands::client;
use crate::vault_store;
use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct CandlesArgs {
    /// Instrument, e.g. EUR_USD.
    pub instrument: String,
    /// Candle granularity (M1, M5, M15, H1, H4, D, ...).
    #[arg(long, default_value = "H1")]
    pub granularity: String,
    /// Start date (YYYY-MM-DD). With --to, fetches the range.
    #[arg(long)]
    pub from: Option<String>,
    /// End date (YYYY-MM-DD).
    #[arg(long)]
    pub to: Option<String>,
    /// Recent candle count when no date range is given (max 5000).
    #[arg(long)]
    pub count: Option<u32>,
    /// Comma-separated indicators to attach, e.g. `ema:20,rsi:14`.
    /// Supported: sma, ema, rsi, atr, adx, mfi (each `name:period`).
    #[arg(long, value_delimiter = ',')]
    pub indicators: Vec<String>,
    /// OANDA environment whose stored credentials are used.
    #[arg(long, default_value = "practice")]
    pub env: String,
}

pub async fn run(args: CandlesArgs, out: Out) -> ! {
    match fetch(args).await {
        Ok(value) => {
            out.ok(&value);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let code = if msg.contains("keychain") || msg.contains("credentials") {
                exit::AUTH
            } else if msg.contains("indicator") || msg.contains("granularity") {
                exit::VALIDATION
            } else {
                exit::OANDA
            };
            out.fail(code, "candles_failed", msg);
        }
    }
}

async fn fetch(args: CandlesArgs) -> Result<Value> {
    let gran = Granularity::from_str(&args.granularity)
        .map_err(|e| anyhow!("invalid granularity: {e}"))?;

    // Build indicator pipeline up-front so a bad spec fails before any network.
    let mut indicators: Vec<(String, Box<dyn Indicator>)> = Vec::new();
    for spec in &args.indicators {
        let spec = spec.trim();
        if spec.is_empty() {
            continue;
        }
        indicators.push((spec.to_string(), make_indicator(spec)?));
    }

    let (_env, client) = client::resolve(&args.env, vault_store::DEFAULT_ACCOUNT)?;
    let candles = fetch_candles(&client, &args, gran).await?;

    let mut payload = json!({
        "instrument": args.instrument,
        "granularity": args.granularity,
        "count": candles.len(),
        "candles": candles,
    });

    if !indicators.is_empty() {
        let mut indicator_series = serde_json::Map::new();
        for (label, ind) in indicators.iter_mut() {
            let series: Vec<Value> = candles
                .iter()
                .map(|c| serde_json::to_value(ind.on_candle(c)).unwrap_or(Value::Null))
                .collect();
            indicator_series.insert(label.clone(), Value::Array(series));
        }
        payload
            .as_object_mut()
            .unwrap()
            .insert("indicators".to_string(), Value::Object(indicator_series));
    }

    Ok(payload)
}

async fn fetch_candles(
    client: &OandaClient,
    args: &CandlesArgs,
    gran: Granularity,
) -> Result<Vec<Candle>> {
    let candles = if let Some(from) = args.from.as_ref() {
        // A from-range fetch paginates past OANDA's 5000-calendar-bar cap
        // (the cap counts weekends, so H1 tops out around ~7 months per
        // request): chunked requests, stitched and deduped, bounded
        // client-side by `to` (issue #292).
        let from_rfc3339 = format!("{from}T00:00:00Z");
        let to_rfc3339 = match args.to.as_ref() {
            Some(d) => {
                let today_utc = chrono::Utc::now().format("%Y-%m-%d").to_string();
                if d >= &today_utc {
                    (chrono::Utc::now() - chrono::Duration::minutes(1))
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                } else {
                    format!("{d}T23:59:59Z")
                }
            }
            // No --to: everything from `from` until now.
            None => (chrono::Utc::now() - chrono::Duration::minutes(1))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        };
        endpoints::get_candles_paginated(
            client,
            &args.instrument,
            gran,
            &from_rfc3339,
            &to_rfc3339,
        )
        .await
    } else if let Some(to) = args.to.as_ref() {
        // --to without --from: candles counting back from `to` — a single
        // request, no range to exceed. Default stays 5000 (the prior
        // contract for this path); --count narrows it.
        let to_rfc3339 = format!("{to}T23:59:59Z");
        endpoints::get_candles(
            client,
            &args.instrument,
            gran,
            Some(args.count.unwrap_or(5000).min(5000)),
            None,
            Some(&to_rfc3339),
        )
        .await
    } else {
        endpoints::get_candles(
            client,
            &args.instrument,
            gran,
            Some(args.count.unwrap_or(500).min(5000)),
            None,
            None,
        )
        .await
    };
    candles.with_context(|| "OANDA candle fetch failed")
}

/// Construct a period-based indicator from a `name:period` spec.
fn make_indicator(spec: &str) -> Result<Box<dyn Indicator>> {
    let mut parts = spec.splitn(2, ':');
    let name = parts.next().unwrap_or("").to_lowercase();
    let period: usize = match parts.next() {
        Some(p) => p
            .parse()
            .map_err(|_| anyhow!("invalid period in indicator spec '{spec}'"))?,
        None => 14,
    };
    if period == 0 {
        bail!("indicator '{spec}' has a zero period");
    }
    Ok(match name.as_str() {
        "sma" => Box::new(SmaIndicator::new(period)),
        "ema" => Box::new(EmaIndicator::new(period)),
        "rsi" => Box::new(RsiIndicator::new(period)),
        "atr" => Box::new(AtrIndicator::new(period)),
        "adx" => Box::new(AdxIndicator::new(period)),
        "mfi" => Box::new(MfiIndicator::new(period)),
        other => bail!(
            "unsupported indicator '{other}' (supported: sma, ema, rsi, atr, adx, mfi)"
        ),
    })
}
