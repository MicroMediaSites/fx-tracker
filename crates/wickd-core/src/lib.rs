//! wickd-core — the headless, Tauri-free trading core shared by the
//! CandleSight desktop app, the MCP server, and the `wickd` CLI.
//!
//! Extracted from `src-tauri/src/`. Tauri event emission is abstracted behind
//! the [`EventSink`] trait (see [`event_sink`]); this crate must never depend
//! on `tauri`. Front-ends provide their own sink:
//! - desktop → re-emits Tauri events (+ OS notifications)
//! - CLI → writes NDJSON to stdout

pub mod alert_queue;
pub mod backtest;
pub mod config;
pub mod crypto;
pub mod error;
pub mod event_sink;
pub mod events;
pub mod hub_client;
pub mod models;
pub mod ndjson;
pub mod oanda;
pub mod paths;
pub mod pending;
pub mod strategy;
pub mod strategy_store;
pub mod stream_hub;

pub use event_sink::EventSink;
pub use shared;

/// Crate version, surfaced by front-ends (CLI `--version`, etc.).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
