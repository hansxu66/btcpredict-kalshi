//! Kraken WebSocket configuration.
//!
//! No authentication required for public market data (ticker channel).
//! Note: Kraken uses XBT instead of BTC for Bitcoin.

/// Kraken WebSocket URL
pub const KRAKEN_WS_URL: &str = "wss://ws.kraken.com";

/// Configuration for Kraken WebSocket connection
#[derive(Debug, Clone)]
pub struct KrakenConfig {
    /// Trading pair (e.g., "XBT/USD")
    /// Note: Kraken uses XBT for Bitcoin, not BTC
    pub pair: String,
}

impl KrakenConfig {
    /// Create config for BTC/USD pair (XBT/USD in Kraken notation)
    pub fn btc_usd() -> Self {
        Self {
            pair: "XBT/USD".to_string(),
        }
    }

    /// Create config for a custom pair
    pub fn new(pair: impl Into<String>) -> Self {
        Self {
            pair: pair.into(),
        }
    }

    /// Get the WebSocket URL
    pub fn ws_url(&self) -> &'static str {
        KRAKEN_WS_URL
    }

    /// Get the subscription message for ticker channel
    pub fn ticker_subscribe_msg(&self) -> String {
        serde_json::json!({
            "event": "subscribe",
            "pair": [&self.pair],
            "subscription": {
                "name": "ticker"
            }
        })
        .to_string()
    }
}
