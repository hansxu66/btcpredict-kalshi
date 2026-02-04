//! Type definitions for Coinbase Advanced Trade WebSocket messages.

use serde::Deserialize;

// =============================================================================
// Incoming Message Types
// =============================================================================

/// Top-level WebSocket message from Coinbase
#[derive(Debug, Clone, Deserialize)]
pub struct CoinbaseMessage {
    pub channel: String,
    #[serde(default)]
    pub client_id: String,
    pub timestamp: String,
    #[serde(default)]
    pub sequence_num: u64,
    #[serde(default)]
    pub events: Vec<CoinbaseEvent>,
}

/// Event within a Coinbase message
#[derive(Debug, Clone, Deserialize)]
pub struct CoinbaseEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub tickers: Vec<CoinbaseTicker>,
}

/// Ticker data from Coinbase
#[derive(Debug, Clone, Deserialize)]
pub struct CoinbaseTicker {
    #[serde(rename = "type")]
    pub ticker_type: Option<String>,
    pub product_id: String,
    #[serde(default)]
    pub price: String,
    #[serde(default)]
    pub best_bid: String,
    #[serde(default)]
    pub best_bid_quantity: String,
    #[serde(default)]
    pub best_ask: String,
    #[serde(default)]
    pub best_ask_quantity: String,
    #[serde(default)]
    pub volume_24_h: String,
    #[serde(default)]
    pub low_24_h: String,
    #[serde(default)]
    pub high_24_h: String,
}

impl CoinbaseTicker {
    /// Parse best bid price as f64
    pub fn bid_price(&self) -> Option<f64> {
        self.best_bid.parse().ok()
    }

    /// Parse best ask price as f64
    pub fn ask_price(&self) -> Option<f64> {
        self.best_ask.parse().ok()
    }

    /// Calculate mid-price from best bid and ask
    pub fn mid_price(&self) -> Option<f64> {
        let bid = self.bid_price()?;
        let ask = self.ask_price()?;
        Some((bid + ask) / 2.0)
    }

    /// Parse last trade price as f64
    pub fn last_price(&self) -> Option<f64> {
        self.price.parse().ok()
    }
}

