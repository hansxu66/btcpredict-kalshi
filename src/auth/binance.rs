//! Binance WebSocket configuration.
//!
//! No authentication required for public market data streams.

/// Binance WebSocket base URL
pub const BINANCE_WS_URL: &str = "wss://stream.binance.com:9443/ws";

/// Configuration for Binance WebSocket connection
#[derive(Debug, Clone)]
pub struct BinanceConfig {
    /// Trading pair symbol (lowercase, e.g., "btcusdt")
    pub symbol: String,
}

impl BinanceConfig {
    /// Create config for BTC/USDT pair
    pub fn btc_usdt() -> Self {
        Self {
            symbol: "btcusdt".to_string(),
        }
    }

    /// Create config for a custom symbol
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into().to_lowercase(),
        }
    }

    /// Get the bookTicker stream URL for real-time best bid/ask
    pub fn book_ticker_url(&self) -> String {
        format!("{}/{}@bookTicker", BINANCE_WS_URL, self.symbol)
    }

    /// Get the trade stream URL for real-time trades
    #[allow(dead_code)]
    pub fn trade_url(&self) -> String {
        format!("{}/{}@trade", BINANCE_WS_URL, self.symbol)
    }
}
