//! Crypto.com Exchange WebSocket configuration.
//!
//! No authentication required for public market data (ticker channel).

/// Crypto.com Exchange WebSocket URL for market data
pub const CRYPTOCOM_WS_URL: &str = "wss://stream.crypto.com/exchange/v1/market";

/// Configuration for Crypto.com WebSocket connection
#[derive(Debug, Clone)]
pub struct CryptocomConfig {
    /// Instrument name (e.g., "BTC_USDT")
    pub instrument: String,
}

impl CryptocomConfig {
    /// Create config for BTC/USDT pair
    pub fn btc_usdt() -> Self {
        Self {
            instrument: "BTC_USDT".to_string(),
        }
    }

    /// Create config for a custom instrument
    pub fn new(instrument: impl Into<String>) -> Self {
        Self {
            instrument: instrument.into(),
        }
    }

    /// Get the WebSocket URL
    pub fn ws_url(&self) -> &'static str {
        CRYPTOCOM_WS_URL
    }

    /// Get the subscription message for ticker channel
    pub fn ticker_subscribe_msg(&self) -> String {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        serde_json::json!({
            "id": 1,
            "method": "subscribe",
            "params": {
                "channels": [format!("ticker.{}", self.instrument)]
            },
            "nonce": nonce
        })
        .to_string()
    }
}
