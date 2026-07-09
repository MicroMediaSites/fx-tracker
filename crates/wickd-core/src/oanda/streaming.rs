use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest::Client;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::io::StreamReader;

use crate::config::OandaEnvironment;
use crate::error::Result;
use crate::event_sink::EventSink;
use crate::strategy::CandleBoundaryService;
use super::types::{StreamMessage, StreamPrice};

/// How long without a heartbeat before we consider the stream unhealthy
const STREAM_HEALTH_TIMEOUT_SECS: u64 = 30;

/// How often to check stream health
const HEALTH_CHECK_INTERVAL_SECS: u64 = 10;

/// Maximum number of reconnection attempts before giving up
const MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// Initial delay between reconnection attempts (in seconds)
const INITIAL_RECONNECT_DELAY_SECS: u64 = 1;

/// Maximum delay between reconnection attempts (in seconds)
const MAX_RECONNECT_DELAY_SECS: u64 = 60;



#[derive(Debug, Clone, serde::Serialize)]
pub struct PriceUpdate {
    pub instrument: String,
    pub bid: String,
    pub ask: String,
    pub spread: String,
    pub time: String,
    pub tradeable: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamError {
    pub error_type: StreamErrorType,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamErrorType {
    ParseError,
    ConnectionLost,
    StreamEnded,
    Reconnecting,
    MaxReconnectsExceeded,
}

/// Stream health status emitted periodically
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamHealthStatus {
    /// Whether the stream is currently healthy (receiving data)
    pub healthy: bool,
    /// Seconds since last heartbeat (or tick)
    pub seconds_since_heartbeat: u64,
    /// Number of subscribed instruments
    pub subscribed_instruments: usize,
    /// Whether stream is currently running
    pub running: bool,
}

impl From<StreamPrice> for PriceUpdate {
    fn from(price: StreamPrice) -> Self {
        let bid = price.bids.first().map(|b| b.price.clone()).unwrap_or_default();
        let ask = price.asks.first().map(|a| a.price.clone()).unwrap_or_default();

        let spread = match (Decimal::from_str(&bid), Decimal::from_str(&ask)) {
            (Ok(bid_d), Ok(ask_d)) if !bid_d.is_zero() && !ask_d.is_zero() => {
                (ask_d - bid_d).to_string()
            }
            _ => "0".to_string(),
        };

        Self {
            instrument: price.instrument,
            bid,
            ask,
            spread,
            time: price.time,
            tradeable: price.tradeable,
        }
    }
}

/// Centralized price stream manager that handles all OANDA price streaming.
///
/// Uses a pub/sub pattern where UI components subscribe to instruments they need.
/// Maintains a single stream to OANDA with all subscribed instruments.
///
/// # Architecture
/// ```text
/// ┌─────────────────────────────────────────────────────────────┐
/// │                    OANDA Streaming API                       │
/// │         (single connection: EUR_USD,GBP_USD,AUD_USD)        │
/// └─────────────────────────────────────────────────────────────┘
///                               │
///                               ▼
/// ┌─────────────────────────────────────────────────────────────┐
/// │                  PriceStreamManager (Rust)                   │
/// │  - Maintains ONE stream to OANDA                            │
/// │  - Tracks subscribed instruments + subscriber count          │
/// │  - Adds/removes instruments, restarts stream as needed       │
/// │  - Broadcasts price events to all windows                    │
/// └─────────────────────────────────────────────────────────────┘
///                               │
///               ┌───────────────┼───────────────┐
///               ▼               ▼               ▼
///          Chart EUR/USD   Chart GBP/USD   Main Dashboard
///          (subscriber)    (subscriber)    (subscriber)
/// ```
pub struct PriceStreamManager {
    client: Client,
    api_key: String,
    account_id: String,
    stream_url: String,
    running: Arc<AtomicBool>,
    /// Maps instrument -> subscriber count
    subscriptions: HashMap<String, usize>,
    /// Cached event sink for restarting streams
    sink: Option<Arc<dyn EventSink>>,
    /// Optional candle boundary service for detecting candle closes from tick data
    candle_boundary_service: Option<Arc<CandleBoundaryService>>,
    /// Timestamp of last received data (heartbeat or tick), stored as epoch millis
    /// Using AtomicU64 for lock-free updates from the stream task
    last_data_time: Arc<AtomicU64>,
    /// Start time for calculating relative timestamps
    start_time: Instant,
    /// Whether the health monitor task has been started
    health_monitor_started: bool,
}

impl PriceStreamManager {
    pub fn new(api_key: &str, account_id: &str, environment: &OandaEnvironment) -> Self {
        let stream_url = match environment {
            OandaEnvironment::Practice => "https://stream-fxpractice.oanda.com".to_string(),
            OandaEnvironment::Live => "https://stream-fxtrade.oanda.com".to_string(),
        };

        let start_time = Instant::now();
        Self {
            client: Client::builder()
                .min_tls_version(reqwest::tls::Version::TLS_1_2)
                .build()
                .expect("Failed to build reqwest client with TLS 1.2 minimum"),
            api_key: api_key.to_string(),
            account_id: account_id.to_string(),
            stream_url,
            running: Arc::new(AtomicBool::new(false)),
            subscriptions: HashMap::new(),
            sink: None,
            candle_boundary_service: None,
            last_data_time: Arc::new(AtomicU64::new(start_time.elapsed().as_millis() as u64)),
            start_time,
            health_monitor_started: false,
        }
    }

    /// Get the current stream health status
    pub fn health_status(&self) -> StreamHealthStatus {
        let last_data_millis = self.last_data_time.load(Ordering::SeqCst);
        let current_millis = self.start_time.elapsed().as_millis() as u64;
        let seconds_since = (current_millis.saturating_sub(last_data_millis)) / 1000;

        StreamHealthStatus {
            healthy: self.running.load(Ordering::SeqCst) && seconds_since < STREAM_HEALTH_TIMEOUT_SECS,
            seconds_since_heartbeat: seconds_since,
            subscribed_instruments: self.subscriptions.len(),
            running: self.running.load(Ordering::SeqCst),
        }
    }

    /// Check if the stream is currently healthy (receiving data)
    pub fn is_healthy(&self) -> bool {
        self.health_status().healthy
    }

    /// Update the last data timestamp (called when heartbeat or tick received)
    #[allow(dead_code)]
    fn record_data_received(&self) {
        let current_millis = self.start_time.elapsed().as_millis() as u64;
        self.last_data_time.store(current_millis, Ordering::SeqCst);
    }

    /// Start a background task that monitors stream health and emits events.
    ///
    /// Emits `stream-health` events when the stream becomes unhealthy (no data for 30+ seconds).
    /// Only emits when health status changes to avoid spamming the frontend.
    pub fn start_health_monitor(&self, sink: Arc<dyn EventSink>) {
        let running = self.running.clone();
        let last_data_time = self.last_data_time.clone();
        let start_time = self.start_time;

        tokio::spawn(async move {
            let mut was_healthy = true;

            loop {
                tokio::time::sleep(Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS)).await;

                // Only check health if stream is supposed to be running
                if !running.load(Ordering::SeqCst) {
                    was_healthy = true; // Reset when not running
                    continue;
                }

                let last_data_millis = last_data_time.load(Ordering::SeqCst);
                let current_millis = start_time.elapsed().as_millis() as u64;
                let seconds_since = (current_millis.saturating_sub(last_data_millis)) / 1000;
                let is_healthy = seconds_since < STREAM_HEALTH_TIMEOUT_SECS;

                // Only emit event on health status change
                if was_healthy && !is_healthy {
                    tracing::warn!(
                        "[StreamManager] Stream unhealthy - no data for {} seconds",
                        seconds_since
                    );
                    let status = StreamHealthStatus {
                        healthy: false,
                        seconds_since_heartbeat: seconds_since,
                        subscribed_instruments: 0, // We don't have access to subscriptions here
                        running: true,
                    };
                    sink.stream_health(&status);
                } else if !was_healthy && is_healthy {
                    tracing::info!("[StreamManager] Stream health restored");
                    let status = StreamHealthStatus {
                        healthy: true,
                        seconds_since_heartbeat: seconds_since,
                        subscribed_instruments: 0,
                        running: true,
                    };
                    sink.stream_health(&status);
                }

                was_healthy = is_healthy;
            }
        });
    }

    /// Set the candle boundary service for streaming-based candle detection.
    ///
    /// When set, the manager will notify the boundary service on each price tick,
    /// enabling sub-second candle close detection for strategy watchers.
    pub fn set_candle_boundary_service(&mut self, service: Arc<CandleBoundaryService>) {
        self.candle_boundary_service = Some(service);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn has_credentials(&self) -> bool {
        !self.api_key.is_empty()
    }

    pub fn has_account(&self) -> bool {
        !self.account_id.is_empty()
    }

    /// Get list of currently subscribed instruments
    pub fn subscribed_instruments(&self) -> Vec<String> {
        self.subscriptions.keys().cloned().collect()
    }

    /// Get the subscriber count for a specific instrument
    pub fn subscriber_count(&self, instrument: &str) -> usize {
        self.subscriptions.get(instrument).copied().unwrap_or(0)
    }

    /// Subscribe to price updates for an instrument.
    ///
    /// If this is a new instrument (not currently subscribed), the stream will be
    /// restarted to include it. If already subscribed, just increments the ref count.
    ///
    /// # Arguments
    /// * `instrument` - The instrument to subscribe to (e.g., "EUR_USD")
    /// * `sink` - Event sink for emitting events
    pub async fn subscribe(&mut self, instrument: String, sink: Arc<dyn EventSink>) -> Result<()> {
        tracing::info!("[StreamManager] subscribe() called for: {}", instrument);

        // Start the health monitor on first subscription (once per app lifetime)
        if !self.health_monitor_started {
            self.start_health_monitor(sink.clone());
            self.health_monitor_started = true;
            tracing::info!("[StreamManager] Health monitor started");
        }

        // Store sink for future restarts
        self.sink = Some(sink.clone());

        // Check if this is a new instrument
        let is_new = !self.subscriptions.contains_key(&instrument);

        // Increment subscription count
        let count = self.subscriptions.entry(instrument.clone()).or_insert(0);
        *count += 1;
        let current_count = *count;

        tracing::info!(
            "[StreamManager] Subscription count for {}: {} (total instruments: {})",
            instrument,
            current_count,
            self.subscriptions.len()
        );

        // If this is a new instrument, restart stream to include it
        if is_new {
            tracing::info!("[StreamManager] New instrument added, restarting stream");
            self.restart_stream(sink).await?;
        } else {
            tracing::info!("[StreamManager] Instrument already subscribed, no restart needed");
        }

        Ok(())
    }

    /// Unsubscribe from price updates for an instrument.
    ///
    /// Decrements the subscriber count. If count reaches 0, removes the instrument
    /// and restarts the stream without it.
    ///
    /// # Arguments
    /// * `instrument` - The instrument to unsubscribe from
    pub async fn unsubscribe(&mut self, instrument: String) -> Result<()> {
        tracing::info!("[StreamManager] unsubscribe() called for: {}", instrument);

        if let Some(count) = self.subscriptions.get_mut(&instrument) {
            *count = count.saturating_sub(1);
            tracing::info!(
                "[StreamManager] Decremented count for {}: {}",
                instrument,
                count
            );

            if *count == 0 {
                self.subscriptions.remove(&instrument);
                tracing::info!(
                    "[StreamManager] Removed {} from subscriptions (total: {})",
                    instrument,
                    self.subscriptions.len()
                );

                // Restart stream without this instrument
                if let Some(sink) = &self.sink {
                    self.restart_stream(sink.clone()).await?;
                }
            }
        } else {
            tracing::warn!(
                "[StreamManager] Attempted to unsubscribe from {} but not subscribed",
                instrument
            );
        }

        Ok(())
    }

    /// Stop the current stream
    pub fn stop(&self) {
        tracing::info!("[StreamManager] Stopping stream");
        self.running.store(false, Ordering::SeqCst);
    }

    /// Restart the stream with all currently subscribed instruments
    async fn restart_stream(&mut self, sink: Arc<dyn EventSink>) -> Result<()> {
        // Stop existing stream if running
        if self.running.load(Ordering::SeqCst) {
            tracing::info!("[StreamManager] Stopping existing stream before restart");
            self.stop();
            // Give the old stream task time to notice the stop signal
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        // Get all subscribed instruments
        let instruments: Vec<String> = self.subscriptions.keys().cloned().collect();

        if instruments.is_empty() {
            tracing::info!("[StreamManager] No instruments subscribed, not starting stream");
            return Ok(());
        }

        tracing::info!(
            "[StreamManager] Starting stream with {} instruments: {:?}",
            instruments.len(),
            instruments
        );

        self.start_stream(instruments, sink).await
    }

    /// Start the OANDA price stream for the given instruments
    ///
    /// Includes auto-reconnection with exponential backoff if the stream disconnects.
    async fn start_stream(&mut self, instruments: Vec<String>, sink: Arc<dyn EventSink>) -> Result<()> {
        // Try to acquire the running lock
        if self.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            tracing::warn!("[StreamManager] Failed to acquire running lock");
            return Ok(());
        }

        // Build the stream URL
        let url = format!(
            "{}/v3/accounts/{}/pricing/stream?instruments={}",
            self.stream_url,
            self.account_id,
            instruments.join(",")
        );

        // Masked URL for logging — never log the full account number.
        // Masking style matches OandaClient in client.rs.
        let masked_account = if self.account_id.len() > 4 {
            format!("***{}", &self.account_id[self.account_id.len() - 4..])
        } else {
            "****".to_string()
        };
        let masked_url = format!(
            "{}/v3/accounts/{}/pricing/stream?instruments={}",
            self.stream_url,
            masked_account,
            instruments.join(",")
        );

        // Clone everything needed for the reconnecting stream task
        let running = self.running.clone();
        let boundary_service = self.candle_boundary_service.clone();
        let last_data_time = self.last_data_time.clone();
        let start_time = self.start_time;
        let api_key = self.api_key.clone();
        let client = self.client.clone();

        tokio::spawn(async move {
            let mut reconnect_attempts: u32 = 0;
            let mut current_delay = INITIAL_RECONNECT_DELAY_SECS;

            // Main reconnection loop
            'reconnect: loop {
                if !running.load(Ordering::SeqCst) {
                    tracing::info!("[StreamManager] Stop signal received, exiting reconnection loop");
                    break;
                }

                // Attempt to connect
                tracing::info!("[StreamManager] Connecting to: {}", masked_url);
                let response = match client
                    .get(&url)
                    .bearer_auth(&api_key)
                    .send()
                    .await
                {
                    Ok(resp) => resp,
                    Err(e) => {
                        tracing::error!("[StreamManager] HTTP request failed: {}", e);
                        reconnect_attempts += 1;

                        if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                            tracing::error!("[StreamManager] Max reconnection attempts ({}) exceeded", MAX_RECONNECT_ATTEMPTS);
                            let error = StreamError {
                                error_type: StreamErrorType::MaxReconnectsExceeded,
                                message: format!("Failed to reconnect after {} attempts", MAX_RECONNECT_ATTEMPTS),
                            };
                            sink.stream_error(&error);
                            break 'reconnect;
                        }

                        // Emit reconnection event
                        let error = StreamError {
                            error_type: StreamErrorType::Reconnecting,
                            message: format!("Reconnecting in {} seconds (attempt {}/{})", current_delay, reconnect_attempts, MAX_RECONNECT_ATTEMPTS),
                        };
                        sink.stream_error(&error);
                        tracing::warn!("[StreamManager] Will retry in {} seconds (attempt {}/{})", current_delay, reconnect_attempts, MAX_RECONNECT_ATTEMPTS);

                        tokio::time::sleep(Duration::from_secs(current_delay)).await;
                        current_delay = std::cmp::min(current_delay * 2, MAX_RECONNECT_DELAY_SECS);
                        continue 'reconnect;
                    }
                };

                if !response.status().is_success() {
                    tracing::error!("[StreamManager] Stream connection failed: {}", response.status());
                    reconnect_attempts += 1;

                    if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                        tracing::error!("[StreamManager] Max reconnection attempts exceeded");
                        let error = StreamError {
                            error_type: StreamErrorType::MaxReconnectsExceeded,
                            message: format!("Failed to reconnect after {} attempts", MAX_RECONNECT_ATTEMPTS),
                        };
                        sink.stream_error(&error);
                        break 'reconnect;
                    }

                    let error = StreamError {
                        error_type: StreamErrorType::Reconnecting,
                        message: format!("Server returned {}, reconnecting in {} seconds", response.status(), current_delay),
                    };
                    sink.stream_error(&error);

                    tokio::time::sleep(Duration::from_secs(current_delay)).await;
                    current_delay = std::cmp::min(current_delay * 2, MAX_RECONNECT_DELAY_SECS);
                    continue 'reconnect;
                }

                // Successfully connected - reset reconnection state
                if reconnect_attempts > 0 {
                    tracing::info!("[StreamManager] Reconnected successfully after {} attempts", reconnect_attempts);
                } else {
                    tracing::info!("[StreamManager] Connection successful, starting stream reader");
                }
                reconnect_attempts = 0;
                current_delay = INITIAL_RECONNECT_DELAY_SECS;

                // Helper to update the last data timestamp
                let record_data = {
                    let last_data_time = last_data_time.clone();
                    move || {
                        let current_millis = start_time.elapsed().as_millis() as u64;
                        last_data_time.store(current_millis, Ordering::SeqCst);
                    }
                };

                // Process the stream
                let stream = response.bytes_stream();
                let stream = stream.map(|result| {
                    result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                });
                let reader = StreamReader::new(stream);
                let mut lines = BufReader::new(reader).lines();

                let disconnect_reason: Option<String> = loop {
                    if !running.load(Ordering::SeqCst) {
                        break None; // Graceful shutdown
                    }

                    match lines.next_line().await {
                        Ok(Some(line)) => {
                            if line.trim().is_empty() {
                                continue;
                            }

                            match serde_json::from_str::<StreamMessage>(&line) {
                                Ok(StreamMessage::Price(price)) => {
                                    record_data();

                                    if let Some(ref service) = boundary_service {
                                        if let Ok(tick_time) = DateTime::parse_from_rfc3339(&price.time) {
                                            let tick_time_utc = tick_time.with_timezone(&Utc);
                                            service.on_price_tick(&price.instrument, tick_time_utc).await;
                                        }
                                    }

                                    let update = PriceUpdate::from(price);
                                    sink.price_update(&update);
                                }
                                Ok(StreamMessage::Heartbeat(_)) => {
                                    record_data();
                                }
                                Err(e) => {
                                    let error = StreamError {
                                        error_type: StreamErrorType::ParseError,
                                        message: format!("Failed to parse price data: {}", e),
                                    };
                                    sink.stream_error(&error);
                                }
                            }
                        }
                        Ok(None) => {
                            break Some("Stream ended unexpectedly".to_string());
                        }
                        Err(e) => {
                            break Some(format!("Connection lost: {}", e));
                        }
                    }
                };

                // If we got here with a disconnect reason, attempt to reconnect
                if let Some(reason) = disconnect_reason {
                    if !running.load(Ordering::SeqCst) {
                        tracing::info!("[StreamManager] Stop signal during disconnect, exiting");
                        break 'reconnect;
                    }

                    reconnect_attempts += 1;
                    tracing::warn!("[StreamManager] Disconnected: {}. Attempt {}/{}", reason, reconnect_attempts, MAX_RECONNECT_ATTEMPTS);

                    if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                        let error = StreamError {
                            error_type: StreamErrorType::MaxReconnectsExceeded,
                            message: format!("Failed to reconnect after {} attempts: {}", MAX_RECONNECT_ATTEMPTS, reason),
                        };
                        sink.stream_error(&error);
                        break 'reconnect;
                    }

                    let error = StreamError {
                        error_type: StreamErrorType::Reconnecting,
                        message: format!("{} - reconnecting in {} seconds", reason, current_delay),
                    };
                    sink.stream_error(&error);

                    tokio::time::sleep(Duration::from_secs(current_delay)).await;
                    current_delay = std::cmp::min(current_delay * 2, MAX_RECONNECT_DELAY_SECS);
                    continue 'reconnect;
                }

                // Graceful shutdown (no disconnect reason means we stopped intentionally)
                break 'reconnect;
            }

            running.store(false, Ordering::SeqCst);
            tracing::info!("[StreamManager] Stream manager task ended");
        });

        Ok(())
    }

    /// Legacy method for backwards compatibility - starts stream with given instruments
    /// Prefer using subscribe/unsubscribe for new code.
    #[deprecated(note = "Use subscribe() instead for proper subscription management")]
    pub async fn start(
        &mut self,
        instruments: Vec<String>,
        sink: Arc<dyn EventSink>,
    ) -> Result<()> {
        tracing::warn!("[StreamManager] Using deprecated start() method - consider using subscribe()");

        // Store sink
        self.sink = Some(sink.clone());

        // Add all instruments as subscriptions (with count 1)
        for instrument in &instruments {
            self.subscriptions.entry(instrument.clone()).or_insert(1);
        }

        self.restart_stream(sink).await
    }
}

// Keep the old name as an alias for backwards compatibility
pub type PriceStreamer = PriceStreamManager;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oanda::types::PriceBucket;

    fn make_stream_price() -> StreamPrice {
        StreamPrice {
            instrument: "EUR_USD".to_string(),
            time: "2024-01-15T10:30:00.000000000Z".to_string(),
            tradeable: true,
            bids: vec![
                PriceBucket { price: "1.08500".to_string(), liquidity: 1000000 },
            ],
            asks: vec![
                PriceBucket { price: "1.08520".to_string(), liquidity: 1000000 },
            ],
            close_out_bid: Some("1.08495".to_string()),
            close_out_ask: Some("1.08525".to_string()),
        }
    }

    #[test]
    fn test_price_update_from_stream_price() {
        let stream_price = make_stream_price();
        let update = PriceUpdate::from(stream_price);

        assert_eq!(update.instrument, "EUR_USD");
        assert_eq!(update.bid, "1.08500");
        assert_eq!(update.ask, "1.08520");
        assert!(update.tradeable);
        assert_eq!(update.time, "2024-01-15T10:30:00.000000000Z");
    }

    #[test]
    fn test_price_update_spread_calculation() {
        let stream_price = make_stream_price();
        let update = PriceUpdate::from(stream_price);

        assert_eq!(update.spread, "0.00020");
    }

    #[test]
    fn test_price_update_empty_bids_asks() {
        let stream_price = StreamPrice {
            instrument: "EUR_USD".to_string(),
            time: "2024-01-15T10:30:00.000000000Z".to_string(),
            tradeable: false,
            bids: vec![],
            asks: vec![],
            close_out_bid: None,
            close_out_ask: None,
        };

        let update = PriceUpdate::from(stream_price);

        assert_eq!(update.bid, "");
        assert_eq!(update.ask, "");
        assert_eq!(update.spread, "0");
        assert!(!update.tradeable);
    }

    #[test]
    fn test_price_stream_manager_new_practice() {
        let manager = PriceStreamManager::new("test-key", "test-account", &OandaEnvironment::Practice);

        assert_eq!(manager.stream_url, "https://stream-fxpractice.oanda.com");
        assert_eq!(manager.api_key, "test-key");
        assert_eq!(manager.account_id, "test-account");
        assert!(!manager.is_running());
        assert!(manager.subscriptions.is_empty());
    }

    #[test]
    fn test_price_stream_manager_new_live() {
        let manager = PriceStreamManager::new("test-key", "test-account", &OandaEnvironment::Live);

        assert_eq!(manager.stream_url, "https://stream-fxtrade.oanda.com");
    }

    #[test]
    fn test_price_stream_manager_stop() {
        let manager = PriceStreamManager::new("test-key", "test-account", &OandaEnvironment::Practice);

        assert!(!manager.is_running());

        manager.running.store(true, Ordering::SeqCst);
        assert!(manager.is_running());

        manager.stop();
        assert!(!manager.is_running());
    }

    #[test]
    fn test_subscribed_instruments_empty() {
        let manager = PriceStreamManager::new("test-key", "test-account", &OandaEnvironment::Practice);
        assert!(manager.subscribed_instruments().is_empty());
    }

    #[test]
    fn test_subscriber_count() {
        let mut manager = PriceStreamManager::new("test-key", "test-account", &OandaEnvironment::Practice);

        assert_eq!(manager.subscriber_count("EUR_USD"), 0);

        manager.subscriptions.insert("EUR_USD".to_string(), 3);
        assert_eq!(manager.subscriber_count("EUR_USD"), 3);
    }

    #[test]
    fn test_price_update_with_invalid_price_strings() {
        let stream_price = StreamPrice {
            instrument: "EUR_USD".to_string(),
            time: "2024-01-15T10:30:00.000000000Z".to_string(),
            tradeable: true,
            bids: vec![
                PriceBucket { price: "invalid".to_string(), liquidity: 1000000 },
            ],
            asks: vec![
                PriceBucket { price: "invalid".to_string(), liquidity: 1000000 },
            ],
            close_out_bid: None,
            close_out_ask: None,
        };

        let update = PriceUpdate::from(stream_price);

        assert_eq!(update.spread, "0");
    }

    #[test]
    fn test_stream_error_types_serialize() {
        let parse_error = StreamError {
            error_type: StreamErrorType::ParseError,
            message: "Failed to parse".to_string(),
        };
        let json = serde_json::to_string(&parse_error).unwrap();
        assert!(json.contains("parse_error"));
        assert!(json.contains("Failed to parse"));

        let connection_lost = StreamError {
            error_type: StreamErrorType::ConnectionLost,
            message: "Connection dropped".to_string(),
        };
        let json = serde_json::to_string(&connection_lost).unwrap();
        assert!(json.contains("connection_lost"));

        let stream_ended = StreamError {
            error_type: StreamErrorType::StreamEnded,
            message: "Stream closed".to_string(),
        };
        let json = serde_json::to_string(&stream_ended).unwrap();
        assert!(json.contains("stream_ended"));
    }

    #[test]
    fn test_stream_error_debug() {
        let error = StreamError {
            error_type: StreamErrorType::ParseError,
            message: "test message".to_string(),
        };
        let debug = format!("{:?}", error);
        assert!(debug.contains("ParseError"));
        assert!(debug.contains("test message"));
    }

    #[test]
    fn test_stream_error_clone() {
        let error = StreamError {
            error_type: StreamErrorType::ConnectionLost,
            message: "original".to_string(),
        };
        let cloned = error.clone();
        assert_eq!(error.message, cloned.message);
    }

    #[test]
    fn test_price_update_debug_and_clone() {
        let stream_price = make_stream_price();
        let update = PriceUpdate::from(stream_price);

        let debug = format!("{:?}", update);
        assert!(debug.contains("EUR_USD"));

        let cloned = update.clone();
        assert_eq!(update.instrument, cloned.instrument);
        assert_eq!(update.bid, cloned.bid);
    }

    #[test]
    fn test_price_update_with_zero_bid() {
        let stream_price = StreamPrice {
            instrument: "EUR_USD".to_string(),
            time: "2024-01-15T10:30:00.000000000Z".to_string(),
            tradeable: true,
            bids: vec![
                PriceBucket { price: "0".to_string(), liquidity: 1000000 },
            ],
            asks: vec![
                PriceBucket { price: "1.08520".to_string(), liquidity: 1000000 },
            ],
            close_out_bid: None,
            close_out_ask: None,
        };

        let update = PriceUpdate::from(stream_price);
        // Should return "0" for spread when bid is zero
        assert_eq!(update.spread, "0");
    }

    #[test]
    fn test_price_update_with_zero_ask() {
        let stream_price = StreamPrice {
            instrument: "EUR_USD".to_string(),
            time: "2024-01-15T10:30:00.000000000Z".to_string(),
            tradeable: true,
            bids: vec![
                PriceBucket { price: "1.08500".to_string(), liquidity: 1000000 },
            ],
            asks: vec![
                PriceBucket { price: "0".to_string(), liquidity: 1000000 },
            ],
            close_out_bid: None,
            close_out_ask: None,
        };

        let update = PriceUpdate::from(stream_price);
        // Should return "0" for spread when ask is zero
        assert_eq!(update.spread, "0");
    }

    #[test]
    fn test_stream_url_format() {
        let manager = PriceStreamManager::new("key", "account", &OandaEnvironment::Practice);
        assert!(manager.stream_url.starts_with("https://"));
        assert!(manager.stream_url.contains("stream"));
        assert!(manager.stream_url.contains("oanda.com"));
    }

    #[test]
    fn test_health_status_initially_healthy() {
        let manager = PriceStreamManager::new("key", "account", &OandaEnvironment::Practice);
        let status = manager.health_status();

        // Not running, so not "healthy" in the streaming sense
        assert!(!status.healthy);
        assert!(!status.running);
        assert_eq!(status.subscribed_instruments, 0);
        // Should be very small since we just created it
        assert!(status.seconds_since_heartbeat < 5);
    }

    #[test]
    fn test_health_status_when_running() {
        let manager = PriceStreamManager::new("key", "account", &OandaEnvironment::Practice);
        // Simulate stream running
        manager.running.store(true, Ordering::SeqCst);

        let status = manager.health_status();
        assert!(status.healthy);
        assert!(status.running);
    }

    #[test]
    fn test_is_healthy() {
        let manager = PriceStreamManager::new("key", "account", &OandaEnvironment::Practice);

        // Not running = not healthy
        assert!(!manager.is_healthy());

        // Running with recent data = healthy
        manager.running.store(true, Ordering::SeqCst);
        assert!(manager.is_healthy());
    }

    #[test]
    fn test_record_data_received() {
        let manager = PriceStreamManager::new("key", "account", &OandaEnvironment::Practice);
        manager.running.store(true, Ordering::SeqCst);

        // Wait a tiny bit to ensure some time passes
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Record data received
        manager.record_data_received();

        let status = manager.health_status();
        assert!(status.healthy);
        // Should be very recent
        assert!(status.seconds_since_heartbeat < 2);
    }

    #[test]
    fn test_health_monitor_started_flag() {
        let manager = PriceStreamManager::new("key", "account", &OandaEnvironment::Practice);
        assert!(!manager.health_monitor_started);
    }

    #[test]
    fn test_stream_health_status_serialize() {
        let status = StreamHealthStatus {
            healthy: true,
            seconds_since_heartbeat: 5,
            subscribed_instruments: 3,
            running: true,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"healthy\":true"));
        assert!(json.contains("\"secondsSinceHeartbeat\":5"));
        assert!(json.contains("\"subscribedInstruments\":3"));
        assert!(json.contains("\"running\":true"));
    }

    #[test]
    fn test_new_error_types_serialize() {
        let reconnecting = StreamError {
            error_type: StreamErrorType::Reconnecting,
            message: "Attempting reconnection".to_string(),
        };
        let json = serde_json::to_string(&reconnecting).unwrap();
        assert!(json.contains("reconnecting"));

        let max_retries = StreamError {
            error_type: StreamErrorType::MaxReconnectsExceeded,
            message: "Max retries reached".to_string(),
        };
        let json = serde_json::to_string(&max_retries).unwrap();
        assert!(json.contains("max_reconnects_exceeded"));
    }
}
