//! Alpaca market-data client (stocks).
//!
//! Endpoints used:
//! - `GET {data}/v2/stocks/snapshots?symbols=…&feed=…` — map keyed by symbol
//! - `GET {data}/v2/stocks/bars?symbols=…&timeframe=…&start=…` — `{"bars":{SYM:[…]},"next_page_token":…}`
//! - `GET {trading}/v2/assets?status=active&asset_class=us_equity` — symbol list for search
//! - `GET {trading}/v2/clock` — open/closed + next open/close
//!
//! ## Free (Basic) plan
//!
//! Real-time quotes on the free plan are IEX-only (~3% of US volume), so
//! snapshots default to `feed=iex`; a paid (Algo Trader Plus) key can pass
//! `sip`. Historical bars always ask for `feed=sip` *without* an `end`: Alpaca
//! then defaults `end` to "now" for real-time subscribers and to 15 minutes ago
//! for everyone else, which is exactly the free plan's allowance — so history
//! is full-market (consolidated) data on every plan, just 15 minutes behind on
//! the free one.

use std::collections::HashMap;

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::{Asset, Bar, HistoryRange, MarketClock, Snapshot};

pub const DATA_BASE: &str = "https://data.alpaca.markets";
pub const PAPER_TRADING_BASE: &str = "https://paper-api.alpaca.markets";
pub const LIVE_TRADING_BASE: &str = "https://api.alpaca.markets";

/// Max bars per page Alpaca allows.
const PAGE_LIMIT: u32 = 10_000;
/// Safety stop for pagination loops.
const MAX_PAGES: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum AlpacaError {
    #[error("Alpaca rejected the API keys ({0})")]
    Auth(String),
    #[error("Alpaca refused the request: {0}")]
    Forbidden(String),
    #[error("Alpaca rate limit hit — slow down and retry")]
    RateLimited,
    #[error("Alpaca returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("network error talking to Alpaca: {0}")]
    Network(String),
    #[error("could not decode Alpaca response: {0}")]
    Decode(String),
}

/// Which quote feed snapshots use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteFeed {
    /// Free plan: real-time, IEX exchange only.
    Iex,
    /// Paid plan: real-time consolidated (all exchanges).
    Sip,
    /// Free plan alternative: consolidated, 15 minutes delayed.
    DelayedSip,
}

impl QuoteFeed {
    pub fn as_str(self) -> &'static str {
        match self {
            QuoteFeed::Iex => "iex",
            QuoteFeed::Sip => "sip",
            QuoteFeed::DelayedSip => "delayed_sip",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "iex" => Some(QuoteFeed::Iex),
            "sip" => Some(QuoteFeed::Sip),
            "delayed_sip" | "delayed-sip" => Some(QuoteFeed::DelayedSip),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct AlpacaCredentials {
    pub key_id: String,
    pub secret: String,
}

impl std::fmt::Debug for AlpacaCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlpacaCredentials")
            .field("key_id", &self.key_id)
            .field("secret", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct AlpacaClient {
    http: Client,
    creds: AlpacaCredentials,
    data_base: String,
    trading_base: String,
    feed: QuoteFeed,
}

#[derive(Deserialize)]
struct BarsPage {
    #[serde(default)]
    bars: Option<HashMap<String, Vec<Bar>>>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: String,
}

impl AlpacaClient {
    /// Client against the production data API and the paper trading API
    /// (assets + clock are identical on paper and live).
    pub fn new(creds: AlpacaCredentials, feed: QuoteFeed) -> Self {
        Self::with_bases(creds, feed, DATA_BASE, PAPER_TRADING_BASE)
    }

    /// Client with explicit base URLs (tests point these at a mock server).
    pub fn with_bases(
        creds: AlpacaCredentials,
        feed: QuoteFeed,
        data_base: &str,
        trading_base: &str,
    ) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent(concat!("wickd/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client builds with static config");
        Self {
            http,
            creds,
            data_base: data_base.trim_end_matches('/').to_string(),
            trading_base: trading_base.trim_end_matches('/').to_string(),
            feed,
        }
    }

    pub fn feed(&self) -> QuoteFeed {
        self.feed
    }

    async fn get<T: DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, String)],
    ) -> Result<T, AlpacaError> {
        let resp = self
            .http
            .get(url)
            .header("APCA-API-KEY-ID", &self.creds.key_id)
            .header("APCA-API-SECRET-KEY", &self.creds.secret)
            .query(query)
            .send()
            .await
            .map_err(|e| AlpacaError::Network(e.without_url().to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AlpacaError::Network(e.without_url().to_string()))?;
        if !status.is_success() {
            let message = serde_json::from_str::<ErrorBody>(&body)
                .map(|b| b.message)
                .ok()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(match status {
                StatusCode::UNAUTHORIZED => AlpacaError::Auth(message),
                StatusCode::FORBIDDEN => AlpacaError::Forbidden(message),
                StatusCode::TOO_MANY_REQUESTS => AlpacaError::RateLimited,
                s => AlpacaError::Http {
                    status: s.as_u16(),
                    body: message,
                },
            });
        }
        serde_json::from_str(&body).map_err(|e| AlpacaError::Decode(e.to_string()))
    }

    /// Latest snapshot per symbol. Symbols Alpaca doesn't know are omitted.
    pub async fn snapshots(
        &self,
        symbols: &[String],
    ) -> Result<HashMap<String, Snapshot>, AlpacaError> {
        if symbols.is_empty() {
            return Ok(HashMap::new());
        }
        let url = format!("{}/v2/stocks/snapshots", self.data_base);
        let raw: HashMap<String, Option<Snapshot>> = self
            .get(
                &url,
                &[
                    ("symbols", symbols.join(",")),
                    ("feed", self.feed.as_str().to_string()),
                ],
            )
            .await?;
        Ok(raw
            .into_iter()
            .filter_map(|(k, v)| v.map(|s| (k, s)))
            .collect())
    }

    /// Split-adjusted consolidated bars for several symbols from `start`,
    /// following pagination. See the module docs for why `end` is omitted.
    pub async fn bars(
        &self,
        symbols: &[String],
        timeframe: &str,
        start: DateTime<Utc>,
    ) -> Result<HashMap<String, Vec<Bar>>, AlpacaError> {
        let mut out: HashMap<String, Vec<Bar>> = HashMap::new();
        if symbols.is_empty() {
            return Ok(out);
        }
        let url = format!("{}/v2/stocks/bars", self.data_base);
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut query = vec![
                ("symbols", symbols.join(",")),
                ("timeframe", timeframe.to_string()),
                ("start", start.to_rfc3339_opts(SecondsFormat::Secs, true)),
                ("limit", PAGE_LIMIT.to_string()),
                ("adjustment", "split".to_string()),
                ("feed", "sip".to_string()),
            ];
            if let Some(tok) = &page_token {
                query.push(("page_token", tok.clone()));
            }
            let page: BarsPage = self.get(&url, &query).await?;
            for (sym, mut bars) in page.bars.unwrap_or_default() {
                out.entry(sym).or_default().append(&mut bars);
            }
            match page.next_page_token.filter(|t| !t.is_empty()) {
                Some(tok) => page_token = Some(tok),
                None => return Ok(out),
            }
        }
        Ok(out)
    }

    /// History for one symbol over a [`HistoryRange`], trimmed to the range's
    /// sessions.
    pub async fn history(
        &self,
        symbol: &str,
        range: HistoryRange,
        now: DateTime<Utc>,
    ) -> Result<Vec<Bar>, AlpacaError> {
        let sym = symbol.to_string();
        let mut map = self
            .bars(
                std::slice::from_ref(&sym),
                range.timeframe(),
                range.start(now),
            )
            .await?;
        Ok(range.trim(map.remove(&sym).unwrap_or_default()))
    }

    /// All active US equities (≈ 10–12k), for local symbol search.
    pub async fn assets(&self) -> Result<Vec<Asset>, AlpacaError> {
        let url = format!("{}/v2/assets", self.trading_base);
        self.get(
            &url,
            &[
                ("status", "active".to_string()),
                ("asset_class", "us_equity".to_string()),
            ],
        )
        .await
    }

    /// Market clock (open/closed, next open/close).
    pub async fn clock(&self) -> Result<MarketClock, AlpacaError> {
        let url = format!("{}/v2/clock", self.trading_base);
        self.get(&url, &[]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn creds() -> AlpacaCredentials {
        AlpacaCredentials {
            key_id: "KID".into(),
            secret: "SEC".into(),
        }
    }

    async fn client(server: &MockServer, feed: QuoteFeed) -> AlpacaClient {
        AlpacaClient::with_bases(creds(), feed, &server.uri(), &server.uri())
    }

    #[tokio::test]
    async fn snapshots_parse_and_send_auth_and_feed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/stocks/snapshots"))
            .and(header("APCA-API-KEY-ID", "KID"))
            .and(header("APCA-API-SECRET-KEY", "SEC"))
            .and(query_param("symbols", "AAPL,ZZZZ"))
            .and(query_param("feed", "iex"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "AAPL": {
                    "latestTrade": {"t":"2026-11-06T20:59:58Z","i":1,"x":"V","p":271.5,"s":100,"c":["@"],"z":"C"},
                    "latestQuote": {"t":"2026-11-06T20:59:59Z","bx":"V","bp":271.4,"bs":2,"ap":271.6,"as":3,"ax":"V","c":["R"],"z":"C"},
                    "minuteBar": {"t":"2026-11-06T20:59:00Z","o":271,"h":272,"l":270,"c":271.5,"v":1000,"n":10,"vw":271.2},
                    "dailyBar": {"t":"2026-11-06T05:00:00Z","o":268,"h":273,"l":267,"c":271.5,"v":50000000,"n":500000,"vw":270.1},
                    "prevDailyBar": {"t":"2026-11-05T05:00:00Z","o":265,"h":269,"l":264,"c":268.0,"v":45000000,"n":450000,"vw":267.0}
                },
                "ZZZZ": null
            })))
            .expect(1)
            .mount(&server)
            .await;

        let snaps = client(&server, QuoteFeed::Iex)
            .await
            .snapshots(&["AAPL".into(), "ZZZZ".into()])
            .await
            .unwrap();
        assert_eq!(snaps.len(), 1);
        let a = &snaps["AAPL"];
        assert_eq!(a.last(), Some(271.5));
        assert_eq!(a.prev_close(), Some(268.0));
        assert_eq!(a.latest_quote.as_ref().unwrap().ask, 271.6);
        assert_eq!(a.daily_bar.as_ref().unwrap().volume, 50_000_000.0);
    }

    #[tokio::test]
    async fn bars_follow_pagination_and_omit_end() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/stocks/bars"))
            .and(query_param("feed", "sip"))
            .and(query_param("adjustment", "split"))
            .and(query_param("timeframe", "1Day"))
            .and(query_param_is_missing("end"))
            .and(query_param_is_missing("page_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "bars": {"SPY": [{"t":"2026-11-05T05:00:00Z","o":1,"h":2,"l":0.5,"c":1.5,"v":10,"n":1,"vw":1.2}]},
                "next_page_token": "P2"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/stocks/bars"))
            .and(query_param("page_token", "P2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "bars": {"SPY": [{"t":"2026-11-06T05:00:00Z","o":1.5,"h":2.5,"l":1,"c":2,"v":20,"n":2,"vw":1.8}]},
                "next_page_token": null
            })))
            .expect(1)
            .mount(&server)
            .await;

        let c = client(&server, QuoteFeed::Iex).await;
        let bars = c
            .history(
                "SPY",
                HistoryRange::Year1,
                "2026-11-07T00:00:00Z".parse().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            bars.iter().map(|b| b.close).collect::<Vec<_>>(),
            vec![1.5, 2.0]
        );
    }

    #[tokio::test]
    async fn auth_and_rate_limit_errors_are_typed() {
        let server = MockServer::start().await;
        Mock::given(path("/v2/clock"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(serde_json::json!({"message":"unauthorized."})),
            )
            .mount(&server)
            .await;
        Mock::given(path("/v2/assets"))
            .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
            .mount(&server)
            .await;
        let c = client(&server, QuoteFeed::Sip).await;
        match c.clock().await {
            Err(AlpacaError::Auth(m)) => assert_eq!(m, "unauthorized."),
            other => panic!("expected Auth, got {other:?}"),
        }
        assert!(matches!(c.assets().await, Err(AlpacaError::RateLimited)));
    }

    #[tokio::test]
    async fn clock_and_assets_parse() {
        let server = MockServer::start().await;
        Mock::given(path("/v2/clock"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "timestamp":"2026-11-07T10:00:00-05:00","is_open":false,
                "next_open":"2026-11-09T09:30:00-05:00","next_close":"2026-11-09T16:00:00-05:00"
            })))
            .mount(&server)
            .await;
        Mock::given(path("/v2/assets"))
            .and(query_param("asset_class", "us_equity"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id":"x","class":"us_equity","exchange":"NASDAQ","symbol":"NVDA","name":"NVIDIA Corporation Common Stock","status":"active","tradable":true}
            ])))
            .mount(&server)
            .await;
        let c = client(&server, QuoteFeed::Iex).await;
        let clock = c.clock().await.unwrap();
        assert!(!clock.is_open);
        assert_eq!(clock.next_open.to_rfc3339(), "2026-11-09T14:30:00+00:00");
        let assets = c.assets().await.unwrap();
        assert_eq!(assets[0].symbol, "NVDA");
        assert!(assets[0].tradable);
    }

    #[test]
    fn debug_redacts_secret() {
        let s = format!("{:?}", creds());
        assert!(s.contains("KID") && !s.contains("SEC"));
    }
}
