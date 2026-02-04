//! Type definitions for Crypto.com Exchange WebSocket messages.

use serde::Deserialize;

// =============================================================================
// Incoming Message Types
// =============================================================================

/// Top-level WebSocket response from Crypto.com
#[derive(Debug, Clone, Deserialize)]
pub struct CryptocomResponse {
    pub id: Option<i64>,
    pub method: String,
    pub code: Option<i32>,
    #[serde(default)]
    pub result: Option<CryptocomResult>,
}

/// Result object in Crypto.com response
#[derive(Debug, Clone, Deserialize)]
pub struct CryptocomResult {
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub subscription: String,
    #[serde(default)]
    pub instrument_name: String,
    #[serde(default)]
    pub data: Vec<CryptocomTickerData>,
}

/// Ticker data from Crypto.com
#[derive(Debug, Clone, Deserialize)]
pub struct CryptocomTickerData {
    /// Instrument name (e.g., "BTC_USDT")
    #[serde(default)]
    pub i: String,
    /// Current best bid price (can be null)
    pub b: Option<f64>,
    /// Current best ask price (can be null)
    pub k: Option<f64>,
    /// Latest trade price
    pub a: Option<f64>,
    /// 24h highest trade price
    pub h: Option<f64>,
    /// 24h lowest trade price
    pub l: Option<f64>,
    /// Total 24h traded volume
    pub v: Option<f64>,
    /// Total 24h traded volume value (USD)
    pub vv: Option<f64>,
    /// 24h price change
    pub c: Option<f64>,
    /// Published timestamp (milliseconds)
    pub t: Option<u64>,
}

impl CryptocomTickerData {
    /// Get best bid price
    pub fn bid_price(&self) -> Option<f64> {
        self.b
    }

    /// Get best ask price
    pub fn ask_price(&self) -> Option<f64> {
        self.k
    }

    /// Calculate mid-price from best bid and ask
    pub fn mid_price(&self) -> Option<f64> {
        let bid = self.b?;
        let ask = self.k?;
        Some((bid + ask) / 2.0)
    }

    /// Get last trade price
    pub fn last_price(&self) -> Option<f64> {
        self.a
    }

    /// Get instrument name
    pub fn instrument(&self) -> &str {
        &self.i
    }
}

