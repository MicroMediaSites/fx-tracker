//! Tauri command handlers organized by domain.
//!
//! This module extracts command handlers from main.rs for better organization.

pub mod backtest;
pub mod chat;
pub mod credentials;
pub mod daemon;
pub mod economic_calendar;
pub mod data;
pub mod local_store;
pub mod oanda;
pub mod spread_stats;
pub mod strategy_store;
pub mod streaming;
pub mod trading;
pub mod updater;
pub mod window;
