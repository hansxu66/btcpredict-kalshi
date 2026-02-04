//! Type definitions for Binance WebSocket messages.

use serde::Deserialize;

// =============================================================================
// Incoming Message Types
// =============================================================================

/// BookTicker message from Binance WebSocket stream.
///
/// Provides real-time updates to best bid/ask price and quantity.
/// Stream: `<symbol>@bookTicker`
#[derive(Debug, Clone, Deserialize)]
pub struct BookTickerMessage {
    /// Order book updateId
    pub u: u64,
    /// Symbol (e.g., "BTCUSDT")
    pub s: String,
    /// Best bid price (as string)
    pub b: String,
    /// Best bid quantity (as string)
    #[serde(rename = "B")]
    pub bid_qty: String,
    /// Best ask price (as string)
    pub a: String,
    /// Best ask quantity (as string)
    #[serde(rename = "A")]
    pub ask_qty: String,
}

impl BookTickerMessage {
    /// Parse bid price as f64
    pub fn bid_price(&self) -> Option<f64> {
        self.b.parse().ok()
    }

    /// Parse ask price as f64
    pub fn ask_price(&self) -> Option<f64> {
        self.a.parse().ok()
    }

    /// Calculate mid-price from best bid and ask
    pub fn mid_price(&self) -> Option<f64> {
        let bid = self.bid_price()?;
        let ask = self.ask_price()?;
        Some((bid + ask) / 2.0)
    }

    /// Parse bid quantity as f64
    #[allow(dead_code)]
    pub fn bid_quantity(&self) -> Option<f64> {
        self.bid_qty.parse().ok()
    }

    /// Parse ask quantity as f64
    #[allow(dead_code)]
    pub fn ask_quantity(&self) -> Option<f64> {
        self.ask_qty.parse().ok()
    }
}

