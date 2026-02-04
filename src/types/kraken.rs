//! Type definitions for Kraken WebSocket messages.

use serde::Deserialize;
use serde_json::Value;

// =============================================================================
// Incoming Message Types
// =============================================================================

/// Kraken sends different message types - we need to handle them dynamically
#[derive(Debug, Clone)]
pub enum KrakenMessage {
    /// Heartbeat message
    Heartbeat,
    /// System status message
    SystemStatus(KrakenSystemStatus),
    /// Subscription status message
    SubscriptionStatus(KrakenSubscriptionStatus),
    /// Ticker data (array format)
    Ticker(KrakenTickerData),
    /// Unknown message
    Unknown(String),
}

/// System status message
#[derive(Debug, Clone, Deserialize)]
pub struct KrakenSystemStatus {
    pub event: String,
    #[serde(rename = "connectionID")]
    pub connection_id: Option<u64>,
    pub status: String,
    pub version: Option<String>,
}

/// Subscription status message
#[derive(Debug, Clone, Deserialize)]
pub struct KrakenSubscriptionStatus {
    pub event: String,
    #[serde(rename = "channelID")]
    pub channel_id: Option<u64>,
    #[serde(rename = "channelName")]
    pub channel_name: Option<String>,
    pub pair: Option<String>,
    pub status: String,
    pub subscription: Option<KrakenSubscription>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KrakenSubscription {
    pub name: String,
}

/// Parsed ticker data from Kraken
#[derive(Debug, Clone)]
pub struct KrakenTickerData {
    pub channel_id: i64,
    pub pair: String,
    pub bid_price: f64,
    pub ask_price: f64,
    pub last_price: f64,
    pub volume_today: f64,
}

impl KrakenTickerData {
    /// Calculate mid-price from best bid and ask
    pub fn mid_price(&self) -> f64 {
        (self.bid_price + self.ask_price) / 2.0
    }
}

/// Parse a Kraken WebSocket message
pub fn parse_kraken_message(text: &str) -> KrakenMessage {
    // Try to parse as JSON
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return KrakenMessage::Unknown(text.to_string()),
    };

    // Check if it's an object with "event" field
    if let Some(event) = value.get("event").and_then(|e| e.as_str()) {
        match event {
            "heartbeat" => return KrakenMessage::Heartbeat,
            "systemStatus" => {
                if let Ok(status) = serde_json::from_value(value.clone()) {
                    return KrakenMessage::SystemStatus(status);
                }
            }
            "subscriptionStatus" => {
                if let Ok(status) = serde_json::from_value(value.clone()) {
                    return KrakenMessage::SubscriptionStatus(status);
                }
            }
            _ => {}
        }
    }

    // Check if it's an array (ticker data)
    // Format: [channelId, {ticker data}, "pair", "ticker"]
    if let Some(arr) = value.as_array() {
        if arr.len() >= 4 {
            let channel_id = arr[0].as_i64().unwrap_or(0);
            let pair = arr[2].as_str().unwrap_or("").to_string();
            let channel_name = arr[3].as_str().unwrap_or("");

            if channel_name == "ticker" {
                if let Some(ticker_obj) = arr[1].as_object() {
                    // Parse bid: b[0] is price
                    let bid_price = ticker_obj
                        .get("b")
                        .and_then(|b| b.as_array())
                        .and_then(|b| b.first())
                        .and_then(|p| p.as_str())
                        .and_then(|p| p.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    // Parse ask: a[0] is price
                    let ask_price = ticker_obj
                        .get("a")
                        .and_then(|a| a.as_array())
                        .and_then(|a| a.first())
                        .and_then(|p| p.as_str())
                        .and_then(|p| p.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    // Parse last trade: c[0] is price
                    let last_price = ticker_obj
                        .get("c")
                        .and_then(|c| c.as_array())
                        .and_then(|c| c.first())
                        .and_then(|p| p.as_str())
                        .and_then(|p| p.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    // Parse volume: v[0] is today's volume
                    let volume_today = ticker_obj
                        .get("v")
                        .and_then(|v| v.as_array())
                        .and_then(|v| v.first())
                        .and_then(|p| p.as_str())
                        .and_then(|p| p.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    return KrakenMessage::Ticker(KrakenTickerData {
                        channel_id,
                        pair,
                        bid_price,
                        ask_price,
                        last_price,
                        volume_today,
                    });
                }
            }
        }
    }

    KrakenMessage::Unknown(text.to_string())
}

