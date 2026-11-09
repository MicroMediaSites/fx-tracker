//! Symbol info: "what is this thing?" for any stock or forex pair
//! (wickd-lists AGT-2393, decisions D-5/D-6).
//!
//! Backs `wickd info <SYM>` (JSON, for agents); the TUI's info card
//! (AGT-2394) is meant to reuse it.
//!
//! - **Stocks**: Alpaca's asset details (exchange, class, and what Alpaca lets
//!   you do: trade, short, margin, fractional shares); a plain-language
//!   summary from Wikipedia, found by searching the company name and accepted
//!   only when the article title clearly matches (never a guess); and, for
//!   SEC filers, the industry (SIC), headquarters, state of incorporation and
//!   fiscal year end from EDGAR.
//! - **Pairs** (`EUR_USD`): what the quote means, a Wikipedia summary of each
//!   currency, and OANDA's instrument facts (pip size, precision, margin rate
//!   and leverage, minimum trade size).
//!
//! Results are cached under `~/.wickd/cache/info/<SYM>.json` for 30 days
//! (`--refresh` refetches). A fetch where a source failed (network, HTTP
//! error) is returned but not cached, so a transient error never sticks for
//! a month; missing keys don't block caching. An unknown symbol (Alpaca 404,
//! or a pair OANDA doesn't offer) is a [`NotFound`] error.
//!
//! Caller IDs: SEC requires a contact in the User-Agent; Matt authorised
//! `pardini.matthew@gmail.com` for SEC only (2026-11-08). Wikipedia gets the
//! app name and repo URL, no email.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use wickd_core::equities::alpaca::{AlpacaClient, AlpacaError, AssetDetail, QuoteFeed};
use wickd_core::oanda::endpoints;
use wickd_core::oanda::types::OandaInstrument;

/// How long a cached card is trusted.
const INFO_TTL_DAYS: i64 = 30;
/// How long the SEC ticker→CIK map is trusted.
const SEC_TICKERS_TTL_DAYS: i64 = 7;

const WIKI_UA: &str = concat!(
    "wickd/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/MicroMediaSites/fx-tracker)"
);
/// SEC fair-access policy: identify the caller with a contact (authorised by
/// Matt for SEC only).
const SEC_UA: &str = "wickd pardini.matthew@gmail.com";

const WIKI_API: &str = "https://en.wikipedia.org/w/api.php";
const WIKI_SUMMARY: &str = "https://en.wikipedia.org/api/rest_v1/page/summary/";
const SEC_TICKERS: &str = "https://www.sec.gov/files/company_tickers.json";
const SEC_SUBMISSIONS: &str = "https://data.sec.gov/submissions/";

// ── the card ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub label: String,
    pub value: String,
}

/// Where the summary came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub title: String,
    pub url: String,
}

/// EDGAR company profile (US filers only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecProfile {
    pub cik: u64,
    pub industry: Option<String>,
    pub headquarters: Option<String>,
    pub incorporated: Option<String>,
    /// `MMDD`, e.g. `0131`.
    pub fiscal_year_end: Option<String>,
}

/// One side of a forex pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurrencyInfo {
    pub code: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub symbol: String,
    pub name: String,
    /// `stock` or `forex`.
    pub kind: String,
    pub exchange: Option<String>,
    pub facts: Vec<Fact>,
    /// Plain-language description (Wikipedia for stocks; what the quote
    /// means for pairs).
    pub summary: Option<String>,
    pub source: Option<Source>,
    pub sec: Option<SecProfile>,
    /// Pairs: base then quote currency.
    #[serde(default)]
    pub currencies: Vec<CurrencyInfo>,
    /// Sources that couldn't be reached or had nothing (shown, not fatal).
    #[serde(default)]
    pub notes: Vec<String>,
    pub fetched_at: DateTime<Utc>,
}

impl SymbolInfo {
    fn fact(&mut self, label: &str, value: impl Into<String>) {
        self.facts.push(Fact {
            label: label.to_string(),
            value: value.into(),
        });
    }
}

/// `eur/usd` → `EUR_USD`, `nvda` → `NVDA`.
pub fn normalize(symbol: &str) -> String {
    symbol.trim().to_ascii_uppercase().replace('/', "_")
}

pub fn is_fx(symbol: &str) -> bool {
    symbol.contains('_')
}

// ── fetch + cache ───────────────────────────────────────────────────────────

/// The info card for `symbol`, from the cache when fresh (unless `refresh`).
pub async fn fetch(symbol: &str, refresh: bool) -> Result<SymbolInfo> {
    let symbol = normalize(symbol);
    if symbol.is_empty() {
        return Err(anyhow!("empty symbol"));
    }
    let home = home()?;
    let now = Utc::now();
    if !refresh {
        if let Some(c) = cached_in(&home, &symbol, now) {
            return Ok(c);
        }
    }
    let http = http()?;
    // Set when a source failed transiently (network, HTTP error): such a
    // card is returned but not cached. Missing keys don't count.
    let mut failed = false;
    let info = if is_fx(&symbol) {
        fetch_pair(&http, &symbol, now, &mut failed).await?
    } else {
        fetch_stock(&http, &home, &symbol, now, &mut failed).await?
    };
    if !failed {
        // Cache write failure only costs a refetch.
        let _ = save_in(&home, &info);
    }
    Ok(info)
}

/// The symbol isn't a listed stock (Alpaca 404) or isn't offered by OANDA.
#[derive(Debug)]
pub struct NotFound(pub String);

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown symbol: {}", self.0)
    }
}

impl std::error::Error for NotFound {}

fn home() -> Result<PathBuf> {
    wickd_core::paths::wickd_data_home().map_err(|e| anyhow!(e))
}

fn info_path(home: &Path, symbol: &str) -> PathBuf {
    home.join("cache")
        .join("info")
        .join(format!("{symbol}.json"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

/// A cached card if younger than the TTL.
pub fn cached_in(home: &Path, symbol: &str, now: DateTime<Utc>) -> Option<SymbolInfo> {
    let bytes = std::fs::read(info_path(home, symbol)).ok()?;
    let info: SymbolInfo = serde_json::from_slice(&bytes).ok()?;
    (now - info.fetched_at < Duration::days(INFO_TTL_DAYS)).then_some(info)
}

pub fn save_in(home: &Path, info: &SymbolInfo) -> Result<()> {
    write_atomic(
        &info_path(home, &info.symbol),
        &serde_json::to_vec_pretty(info)?,
    )
}

fn http() -> Result<Client> {
    Client::builder()
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("building HTTP client")
}

async fn get_json<T: DeserializeOwned>(
    http: &Client,
    url: &str,
    query: &[(&str, &str)],
    ua: &str,
) -> Result<T> {
    let resp = http
        .get(url)
        .header(reqwest::header::USER_AGENT, ua)
        .query(query)
        .send()
        .await
        .map_err(|e| anyhow!(e.without_url().to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP {status}"));
    }
    resp.json::<T>()
        .await
        .map_err(|e| anyhow!(e.without_url().to_string()))
}

// ── stocks ──────────────────────────────────────────────────────────────────

async fn fetch_stock(
    http: &Client,
    home: &Path,
    symbol: &str,
    now: DateTime<Utc>,
    failed: &mut bool,
) -> Result<SymbolInfo> {
    let mut info = SymbolInfo {
        symbol: symbol.to_string(),
        name: String::new(),
        kind: "stock".into(),
        exchange: None,
        facts: Vec::new(),
        summary: None,
        source: None,
        sec: None,
        currencies: Vec::new(),
        notes: Vec::new(),
        fetched_at: now,
    };
    // Name + flags from Alpaca (keys optional: fall back to the cached
    // symbol list for the name).
    match crate::alpaca_creds::resolve() {
        Ok(Some((creds, _))) => {
            match AlpacaClient::new(creds, QuoteFeed::Iex)
                .asset_detail(symbol)
                .await
            {
                Ok(a) => apply_asset(&mut info, &a),
                Err(AlpacaError::Http { status: 404, .. }) => {
                    return Err(NotFound(symbol.to_string()).into())
                }
                Err(e) => {
                    info.notes.push(format!("Alpaca: {e}"));
                    *failed = true;
                }
            }
        }
        // No keys is a setup choice, not a passing failure: still cacheable.
        _ => info
            .notes
            .push("Alpaca: no keys (`wickd alpaca login`); trading flags omitted".into()),
    }
    if info.name.is_empty() {
        if let Some(a) = cached_asset_name(home, symbol) {
            info.name = a;
        }
    }

    let (wiki, sec) = tokio::join!(
        wiki_company(http, &info.name),
        sec_profile(http, home, symbol, now)
    );
    match wiki {
        Ok(Some((summary, source))) => {
            info.summary = Some(summary);
            info.source = Some(source);
        }
        Ok(None) => {}
        Err(e) => {
            info.notes.push(format!("Wikipedia: {e:#}"));
            *failed = true;
        }
    }
    match sec {
        Ok(p) => info.sec = p,
        Err(e) => {
            info.notes.push(format!("SEC: {e:#}"));
            *failed = true;
        }
    }
    if info.name.is_empty() {
        info.name = symbol.to_string();
    }
    Ok(info)
}

fn apply_asset(info: &mut SymbolInfo, a: &AssetDetail) {
    let yn = |b: bool| if b { "yes" } else { "no" };
    info.name = a.name.clone();
    if !a.exchange.is_empty() {
        info.exchange = Some(a.exchange.clone());
        info.fact("Exchange", a.exchange.clone());
    }
    let class = match a.class.as_str() {
        "us_equity" => "US equity".to_string(),
        "crypto" => "Crypto".to_string(),
        other => other.to_string(),
    };
    if !class.is_empty() {
        info.fact("Class", class);
    }
    info.fact("Tradable", yn(a.tradable));
    info.fact("Shortable", yn(a.shortable));
    info.fact("Easy to borrow", yn(a.easy_to_borrow));
    info.fact("Marginable", yn(a.marginable));
    info.fact("Fractional shares", yn(a.fractionable));
}

/// The name from the TUI's cached Alpaca symbol list (any age).
fn cached_asset_name(home: &Path, symbol: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Cache {
        assets: Vec<wickd_core::equities::Asset>,
    }
    let bytes = std::fs::read(home.join("cache").join("alpaca-assets.json")).ok()?;
    let c: Cache = serde_json::from_slice(&bytes).ok()?;
    c.assets
        .into_iter()
        .find(|a| a.symbol == symbol)
        .map(|a| a.name)
        .filter(|n| !n.is_empty())
}

/// Search text for a listing name: drop the share-class part ("Common
/// Stock", "Class A", "Series 1", ADS…) but keep "Inc."/"Corporation", which
/// steer the search toward the company ("Apple Inc." not the fruit).
pub fn search_query(name: &str) -> String {
    const MARKERS: [&str; 8] = [
        " common stock",
        " class ",
        " ordinary shares",
        " american depositary",
        " series ",
        " units",
        " warrant",
        " depositary shares",
    ];
    let lower = name.to_ascii_lowercase();
    let cut = MARKERS
        .iter()
        .filter_map(|m| lower.find(m))
        .min()
        .unwrap_or(name.len());
    name[..cut]
        .trim()
        .trim_end_matches([',', '.', '-'])
        .trim()
        .to_string()
}

/// The word an article title must contain: the first significant word of
/// the company name ("Invesco QQQ Trust" → "invesco", "NVIDIA Corporation"
/// → "nvidia"), lowercased, letters and digits only.
pub fn match_word(query: &str) -> Option<String> {
    query
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .find(|w| w.len() >= 2 && w != "the")
}

/// Does a Wikipedia title clearly name this company?
pub fn title_matches(query: &str, title: &str) -> bool {
    let Some(word) = match_word(query) else {
        return false;
    };
    // The word must equal one or more consecutive whole title tokens
    // ("cocacola" = "Coca" + "Cola"), never part of a longer one ("meta" is
    // not "Metallica").
    let tokens: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    (0..tokens.len()).any(|i| {
        let mut acc = String::new();
        tokens[i..].iter().any(|t| {
            acc.push_str(t);
            acc == word
        })
    })
}

/// Words a company/fund summary mentions; guards against same-name articles
/// about something else (a fruit, a person, a place).
fn looks_like_business(extract: &str) -> bool {
    const WORDS: [&str; 26] = [
        "company",
        "corporation",
        "fund",
        "trust",
        "manufacturer",
        "conglomerate",
        "bank",
        "firm",
        "business",
        "exchange-traded",
        "etf",
        "index",
        "holding",
        "retailer",
        "provider",
        "developer",
        "maker",
        "operator",
        "airline",
        "insurer",
        "producer",
        "reit",
        "investment",
        "brand",
        "multinational",
        "chain",
    ];
    let e = extract.to_lowercase();
    WORDS.iter().any(|w| e.contains(w))
}

#[derive(Deserialize)]
struct WikiSearch {
    query: Option<WikiQuery>,
}

#[derive(Deserialize)]
struct WikiQuery {
    #[serde(default)]
    search: Vec<WikiHit>,
}

#[derive(Deserialize)]
struct WikiHit {
    title: String,
}

#[derive(Deserialize)]
struct WikiSummary {
    title: String,
    #[serde(default)]
    extract: String,
    #[serde(rename = "type", default)]
    kind: String,
    content_urls: Option<WikiUrls>,
}

#[derive(Deserialize)]
struct WikiUrls {
    desktop: Option<WikiPage>,
}

#[derive(Deserialize)]
struct WikiPage {
    page: String,
}

/// Candidate titles from a search response that clearly name the company.
fn matching_titles(search: WikiSearch, query: &str) -> Vec<String> {
    search
        .query
        .map(|q| q.search)
        .unwrap_or_default()
        .into_iter()
        .map(|h| h.title)
        .filter(|t| title_matches(query, t))
        .collect()
}

async fn wiki_summary(http: &Client, title: &str) -> Result<WikiSummary> {
    let url = format!("{WIKI_SUMMARY}{}", title.replace(' ', "_"));
    get_json(http, &url, &[], WIKI_UA).await
}

fn summary_parts(s: WikiSummary) -> Option<(String, Source)> {
    if s.kind == "disambiguation" || s.extract.trim().is_empty() {
        return None;
    }
    let url = s
        .content_urls
        .and_then(|u| u.desktop)
        .map(|d| d.page)
        .unwrap_or_else(|| {
            format!(
                "https://en.wikipedia.org/wiki/{}",
                s.title.replace(' ', "_")
            )
        });
    Some((
        s.extract.trim().to_string(),
        Source {
            title: s.title,
            url,
        },
    ))
}

/// A company's Wikipedia summary, or `None` when no article clearly matches.
async fn wiki_company(http: &Client, name: &str) -> Result<Option<(String, Source)>> {
    let query = search_query(name);
    if query.is_empty() {
        return Ok(None);
    }
    let search: WikiSearch = get_json(
        http,
        WIKI_API,
        &[
            ("action", "query"),
            ("list", "search"),
            ("srsearch", &query),
            ("srlimit", "5"),
            ("format", "json"),
        ],
        WIKI_UA,
    )
    .await?;
    // First matching title whose article reads like a business.
    for title in matching_titles(search, &query).into_iter().take(2) {
        let s = wiki_summary(http, &title).await?;
        if !looks_like_business(&s.extract) {
            continue;
        }
        if let Some(parts) = summary_parts(s) {
            return Ok(Some(parts));
        }
    }
    Ok(None)
}

#[derive(Deserialize)]
struct SecTicker {
    cik_str: u64,
    ticker: String,
}

#[derive(Serialize, Deserialize)]
struct SecTickerCache {
    fetched_at: DateTime<Utc>,
    /// ticker → CIK
    ciks: std::collections::HashMap<String, u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecSubmissions {
    #[serde(default)]
    sic_description: Option<String>,
    #[serde(default)]
    state_of_incorporation: Option<String>,
    #[serde(default)]
    fiscal_year_end: Option<String>,
    #[serde(default)]
    addresses: Option<SecAddresses>,
}

#[derive(Deserialize)]
struct SecAddresses {
    business: Option<SecAddress>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecAddress {
    city: Option<String>,
    state_or_country: Option<String>,
}

/// SEC writes share classes with a dash (`BRK-B`), Alpaca with a dot.
fn sec_ticker(symbol: &str) -> String {
    symbol.replace('.', "-")
}

fn parse_sec_tickers(
    raw: std::collections::HashMap<String, SecTicker>,
) -> std::collections::HashMap<String, u64> {
    raw.into_values().map(|t| (t.ticker, t.cik_str)).collect()
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn sec_profile_from(cik: u64, s: SecSubmissions) -> SecProfile {
    let hq = s.addresses.and_then(|a| a.business).and_then(|b| {
        match (non_empty(b.city), non_empty(b.state_or_country)) {
            (Some(c), Some(st)) => Some(format!("{c}, {st}")),
            (Some(c), None) => Some(c),
            (None, Some(st)) => Some(st),
            _ => None,
        }
    });
    SecProfile {
        cik,
        industry: non_empty(s.sic_description),
        headquarters: hq,
        incorporated: non_empty(s.state_of_incorporation),
        fiscal_year_end: non_empty(s.fiscal_year_end),
    }
}

async fn sec_ciks(
    http: &Client,
    home: &Path,
    now: DateTime<Utc>,
) -> Result<std::collections::HashMap<String, u64>> {
    let path = home.join("cache").join("sec-tickers.json");
    if let Some(c) = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<SecTickerCache>(&b).ok())
        .filter(|c| now - c.fetched_at < Duration::days(SEC_TICKERS_TTL_DAYS))
    {
        return Ok(c.ciks);
    }
    let raw: std::collections::HashMap<String, SecTicker> =
        get_json(http, SEC_TICKERS, &[], SEC_UA).await?;
    let ciks = parse_sec_tickers(raw);
    let _ = write_atomic(
        &path,
        &serde_json::to_vec(&SecTickerCache {
            fetched_at: now,
            ciks: ciks.clone(),
        })?,
    );
    Ok(ciks)
}

/// EDGAR profile for a US filer; `None` when the ticker has no CIK (most
/// ETFs, foreign listings).
async fn sec_profile(
    http: &Client,
    home: &Path,
    symbol: &str,
    now: DateTime<Utc>,
) -> Result<Option<SecProfile>> {
    let ciks = sec_ciks(http, home, now).await?;
    let Some(&cik) = ciks.get(&sec_ticker(symbol)) else {
        return Ok(None);
    };
    let url = format!("{SEC_SUBMISSIONS}CIK{cik:010}.json");
    let s: SecSubmissions = get_json(http, &url, &[], SEC_UA).await?;
    Ok(Some(sec_profile_from(cik, s)))
}

// ── pairs ───────────────────────────────────────────────────────────────────

/// Wikipedia article for a currency code.
pub fn currency_title(code: &str) -> Option<&'static str> {
    Some(match code {
        "EUR" => "Euro",
        "USD" => "United States dollar",
        "JPY" => "Japanese yen",
        "GBP" => "Pound sterling",
        "AUD" => "Australian dollar",
        "CAD" => "Canadian dollar",
        "CHF" => "Swiss franc",
        "NZD" => "New Zealand dollar",
        "CNH" => "Renminbi",
        "HKD" => "Hong Kong dollar",
        "SGD" => "Singapore dollar",
        "SEK" => "Swedish krona",
        "NOK" => "Norwegian krone",
        "DKK" => "Danish krone",
        "PLN" => "Polish złoty",
        "HUF" => "Hungarian forint",
        "CZK" => "Czech koruna",
        "TRY" => "Turkish lira",
        "ZAR" => "South African rand",
        "MXN" => "Mexican peso",
        "THB" => "Thai baht",
        _ => return None,
    })
}

/// OANDA facts for a pair: pip size, precision, margin + leverage, minimum
/// trade size.
fn pair_facts(i: &OandaInstrument) -> Vec<Fact> {
    let mut out = Vec::new();
    let mut push = |label: &str, value: String| {
        out.push(Fact {
            label: label.to_string(),
            value,
        })
    };
    if let Some(loc) = i.pip_location {
        // 10^loc, exact.
        let pip = if loc < 0 {
            Decimal::new(1, (-loc) as u32)
        } else {
            Decimal::from(10i64.pow(loc as u32))
        };
        push("Pip size", pip.to_string());
    }
    if let Some(p) = i.display_precision {
        push("Price decimals", p.to_string());
    }
    if let Some(m) = i
        .margin_rate
        .as_deref()
        .and_then(|m| Decimal::from_str(m).ok())
    {
        if !m.is_zero() {
            let pct = (m * Decimal::from(100)).normalize();
            let lev = (Decimal::ONE / m).round_dp(0);
            push("Margin", format!("{pct}% ({lev}:1 leverage)"));
        }
    }
    if let Some(min) = &i.minimum_trade_size {
        let unit = if min == "1" { "unit" } else { "units" };
        push("Minimum trade", format!("{min} {unit}"));
    }
    push("Type", i.instrument_type.to_lowercase());
    out
}

async fn fetch_pair(
    http: &Client,
    symbol: &str,
    now: DateTime<Utc>,
    failed: &mut bool,
) -> Result<SymbolInfo> {
    let (base, quote) = symbol
        .split_once('_')
        .ok_or_else(|| anyhow!("not a pair: {symbol}"))?;
    let mut info = SymbolInfo {
        symbol: symbol.to_string(),
        name: format!("{base}/{quote}"),
        kind: "forex".into(),
        exchange: Some("OANDA".into()),
        facts: Vec::new(),
        summary: None,
        source: None,
        sec: None,
        currencies: Vec::new(),
        notes: Vec::new(),
        fetched_at: now,
    };
    // OANDA instrument facts (practice account, read-only).
    match crate::commands::client::resolve("practice", crate::vault_store::DEFAULT_ACCOUNT) {
        Ok((_, client)) => match endpoints::get_instruments(&client).await {
            Ok(list) => match list.into_iter().find(|i| i.name == symbol) {
                Some(i) => {
                    info.name = i.display_name.clone();
                    info.facts = pair_facts(&i);
                }
                None => return Err(NotFound(symbol.to_string()).into()),
            },
            Err(e) => {
                info.notes.push(format!("OANDA: {e}"));
                *failed = true;
            }
        },
        // No credentials is a setup choice, not a passing failure.
        Err(e) => info.notes.push(format!("OANDA: {e:#}")),
    }
    // Each currency's article.
    for code in [base, quote] {
        let mut c = CurrencyInfo {
            code: code.to_string(),
            name: None,
            summary: None,
            url: None,
        };
        if let Some(title) = currency_title(code) {
            match wiki_summary(http, title).await {
                Ok(s) => {
                    if let Some((summary, src)) = summary_parts(s) {
                        c.name = Some(src.title);
                        c.summary = Some(summary);
                        c.url = Some(src.url);
                    }
                }
                Err(e) => {
                    info.notes.push(format!("Wikipedia ({code}): {e:#}"));
                    *failed = true;
                }
            }
        }
        info.currencies.push(c);
    }
    let name_of = |i: usize, code: &str| {
        info.currencies
            .get(i)
            .and_then(|c| c.name.clone())
            .unwrap_or_else(|| code.to_string())
    };
    let (bn, qn) = (name_of(0, base), name_of(1, quote));
    info.summary = Some(format!(
        "The price of one {bn} ({base}) in {qn} ({quote}): {symbol} at 1.2345 means one {base} buys 1.2345 {quote}. It rises when the {bn} strengthens against the {qn}."
    ));
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_query_drops_share_class_but_keeps_company_words() {
        assert_eq!(
            search_query("Invesco QQQ Trust, Series 1"),
            "Invesco QQQ Trust"
        );
        assert_eq!(
            search_query("NVIDIA Corporation Common Stock"),
            "NVIDIA Corporation"
        );
        assert_eq!(
            search_query("Alphabet Inc. Class A Common Stock"),
            "Alphabet Inc"
        );
        assert_eq!(
            search_query(
                "Taiwan Semiconductor Manufacturing Company Ltd. American Depositary Shares"
            ),
            "Taiwan Semiconductor Manufacturing Company Ltd"
        );
        assert_eq!(
            search_query("SPDR S&P 500 ETF Trust"),
            "SPDR S&P 500 ETF Trust"
        );
    }

    #[test]
    fn titles_must_name_the_company() {
        assert!(title_matches("Invesco QQQ Trust", "Invesco QQQ"));
        assert!(title_matches("NVIDIA Corporation", "Nvidia"));
        assert!(title_matches("Apple Inc", "Apple Inc."));
        // A different company sharing no first word: rejected.
        assert!(!title_matches("Zymeworks Inc", "Zyme (band)"));
        assert!(!title_matches("Acme Widgets Corp", "Widget (GUI)"));
        assert!(!title_matches("Meta Platforms Inc", "Metallica"));
        assert_eq!(match_word("The Coca-Cola Company"), Some("cocacola".into()));
        assert!(title_matches(
            "The Coca-Cola Company",
            "The Coca-Cola Company"
        ));
    }

    #[test]
    fn wikipedia_responses_parse_and_reject_disambiguation() {
        let search: WikiSearch = serde_json::from_str(
            r#"{"batchcomplete":"","query":{"searchinfo":{"totalhits":3},"search":[
                {"ns":0,"title":"Nasdaq-100","pageid":1},
                {"ns":0,"title":"Invesco QQQ","pageid":2}]}}"#,
        )
        .unwrap();
        assert_eq!(
            matching_titles(search, "Invesco QQQ Trust"),
            vec!["Invesco QQQ"]
        );

        let s: WikiSummary = serde_json::from_str(
            r#"{"type":"standard","title":"Invesco QQQ","extract":"Invesco QQQ is an exchange-traded fund.",
               "content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Invesco_QQQ"}}}"#,
        )
        .unwrap();
        assert!(looks_like_business(&s.extract));
        let (summary, src) = summary_parts(s).unwrap();
        assert!(summary.starts_with("Invesco QQQ is"));
        assert_eq!(src.url, "https://en.wikipedia.org/wiki/Invesco_QQQ");

        let d: WikiSummary = serde_json::from_str(
            r#"{"type":"disambiguation","title":"Apple","extract":"Apple may refer to:"}"#,
        )
        .unwrap();
        assert!(summary_parts(d).is_none());
        assert!(!looks_like_business(
            "The apple is the edible fruit of the apple tree."
        ));
    }

    #[test]
    fn sec_responses_parse() {
        let raw: std::collections::HashMap<String, SecTicker> = serde_json::from_str(
            r#"{"0":{"cik_str":1045810,"ticker":"NVDA","title":"NVIDIA CORP"},
                "1":{"cik_str":1067983,"ticker":"BRK-B","title":"BERKSHIRE HATHAWAY INC"}}"#,
        )
        .unwrap();
        let ciks = parse_sec_tickers(raw);
        assert_eq!(ciks[&sec_ticker("NVDA")], 1045810);
        assert_eq!(ciks[&sec_ticker("BRK.B")], 1067983);
        let s: SecSubmissions = serde_json::from_str(
            r#"{"cik":"1045810","name":"NVIDIA CORP","sicDescription":"Semiconductors & Related Devices",
               "stateOfIncorporation":"DE","fiscalYearEnd":"0125",
               "addresses":{"business":{"street1":"2788 San Tomas Expressway","city":"SANTA CLARA","stateOrCountry":"CA"}}}"#,
        )
        .unwrap();
        let p = sec_profile_from(1045810, s);
        assert_eq!(
            p.industry.as_deref(),
            Some("Semiconductors & Related Devices")
        );
        assert_eq!(p.headquarters.as_deref(), Some("SANTA CLARA, CA"));
        assert_eq!(p.incorporated.as_deref(), Some("DE"));
        assert_eq!(p.fiscal_year_end.as_deref(), Some("0125"));
    }

    #[test]
    fn pair_facts_show_pip_margin_and_leverage() {
        let i: OandaInstrument = serde_json::from_str(
            r#"{"name":"USD_JPY","type":"CURRENCY","displayName":"USD/JPY","pipLocation":-2,
                "displayPrecision":3,"marginRate":"0.04","minimumTradeSize":"1"}"#,
        )
        .unwrap();
        let f = pair_facts(&i);
        let get = |l: &str| f.iter().find(|x| x.label == l).map(|x| x.value.clone());
        assert_eq!(get("Pip size").as_deref(), Some("0.01"));
        assert_eq!(get("Price decimals").as_deref(), Some("3"));
        assert_eq!(get("Margin").as_deref(), Some("4% (25:1 leverage)"));
        assert_eq!(get("Minimum trade").as_deref(), Some("1 unit"));
        assert_eq!(currency_title("JPY"), Some("Japanese yen"));
        assert_eq!(currency_title("XAU"), None);
    }

    #[test]
    fn cache_respects_ttl_and_normalizes_symbols() {
        assert_eq!(normalize(" eur/usd "), "EUR_USD");
        assert_eq!(normalize("brk.b"), "BRK.B");
        let dir = tempfile::tempdir().unwrap();
        let now: DateTime<Utc> = "2026-11-09T00:00:00Z".parse().unwrap();
        let info = SymbolInfo {
            symbol: "QQQ".into(),
            name: "Invesco QQQ Trust, Series 1".into(),
            kind: "stock".into(),
            exchange: Some("NASDAQ".into()),
            facts: vec![],
            summary: Some("An ETF.".into()),
            source: None,
            sec: None,
            currencies: vec![],
            notes: vec![],
            fetched_at: now,
        };
        save_in(dir.path(), &info).unwrap();
        assert_eq!(
            cached_in(dir.path(), "QQQ", now + Duration::days(29)),
            Some(info)
        );
        assert!(cached_in(dir.path(), "QQQ", now + Duration::days(31)).is_none());
    }

    /// Live: `cargo test -p wickd live_symbol_info -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_symbol_info() {
        for sym in ["QQQ", "NVDA", "EUR_USD"] {
            let i = fetch(sym, true).await.expect("fetch");
            println!("{}", serde_json::to_string_pretty(&i).unwrap());
            assert!(i.summary.is_some(), "{sym}: no summary");
        }
    }
}
