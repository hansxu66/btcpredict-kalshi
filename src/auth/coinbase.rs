//! Coinbase Advanced Trade WebSocket configuration.
//!
//! No authentication required for public market data (ticker channel).

/// Coinbase Advanced Trade WebSocket URL
pub const COINBASE_WS_URL: &str = "wss://advanced-trade-ws.coinbase.com";

/// Configuration for Coinbase WebSocket connection
#[derive(Debug, Clone)]
pub struct CoinbaseConfig {
    /// Product ID (e.g., "BTC-USD")
    pub product_id: String,
}

impl CoinbaseConfig {
    /// Create config for BTC/USD pair
    pub fn btc_usd() -> Self {
        Self {
            product_id: "BTC-USD".to_string(),
        }
    }

    /// Create config for a custom product
    pub fn new(product_id: impl Into<String>) -> Self {
        Self {
            product_id: product_id.into(),
        }
    }

    /// Get the WebSocket URL
    pub fn ws_url(&self) -> &'static str {
        COINBASE_WS_URL
    }

    /// Get the subscription message for ticker channel
    pub fn ticker_subscribe_msg(&self) -> String {
        serde_json::json!({
            "type": "subscribe",
            "product_ids": [&self.product_id],
            "channel": "ticker"
        })
        .to_string()
    }
}
