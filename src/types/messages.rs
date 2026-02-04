//! Message types for communication between monitors, calculator, and market maker.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::crypto_aggregator::CryptoAggregatorEvent;
use crate::types::{OrderAction, OrderSide, ProbabilityUpdate};

// =============================================================================
// MONITOR → CALCULATOR
// =============================================================================

/// Updates sent from monitors to the calculator
#[derive(Debug, Clone, Serialize)]
pub enum MonitorUpdate {
    /// Probability update from Kalshi
    Kalshi(ProbabilityUpdate),
    /// Aggregated crypto price update from all exchanges
    Crypto(CryptoAggregatorEvent),
}

// =============================================================================
// CALCULATOR → MARKET MAKER
// =============================================================================

/// Snapshot of calculator state sent to Market Maker
#[derive(Debug, Clone, Serialize)]
pub struct CalculatorStateSnapshot {
    /// Market ticker
    pub ticker: String,
    /// Current BTC mid price from aggregator
    pub btc_mid_price: f64,
    /// Current BTC bid price
    pub btc_bid_price: f64,
    /// Current BTC ask price
    pub btc_ask_price: f64,
    /// Number of exchanges contributing to price
    pub exchange_count: usize,
    /// Best YES bid on Kalshi (in cents, e.g., 55)
    pub yes_bid: u16,
    /// Best NO bid on Kalshi (in cents, e.g., 42)
    pub no_bid: u16,
    /// Quantity at best YES bid
    pub yes_qty: i64,
    /// Quantity at best NO bid
    pub no_qty: i64,
    /// Model fair probability (from Black-Scholes)
    pub model_fair_prob: f64,
    /// Blended fair probability (model + market weighted by confidence)
    pub blended_fair_prob: f64,
    /// Hours until market expiry
    pub hours_to_expiry: f64,
    /// Timestamp of this snapshot
    pub timestamp: DateTime<Utc>,
}

// =============================================================================
// FILL MONITOR → MARKET MAKER
// =============================================================================

/// Fill notification from Kalshi WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillUpdate {
    /// Order ID that was filled
    pub order_id: String,
    /// Market ticker
    pub ticker: String,
    /// Side of the order (Yes or No)
    pub side: OrderSide,
    /// Action (Buy or Sell)
    pub action: OrderAction,
    /// Fill price in cents
    pub price_cents: i64,
    /// Number of contracts filled
    pub count: i64,
    /// Timestamp of the fill
    pub timestamp: DateTime<Utc>,
}

// =============================================================================
// MARKET CONFIGURATION
// =============================================================================

/// Configuration for a single market (loaded from CSV or config)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketConfig {
    /// Kalshi market ticker
    pub ticker: String,
    /// Strike price (the threshold, e.g., 97250 for "BTC above $97,250")
    pub strike: f64,
    /// Expiry datetime (UTC)
    pub expiry: DateTime<Utc>,
    /// Annualized volatility for fair value calculation (e.g., 0.50 = 50%)
    pub volatility: f64,
    /// Confidence in the model (0.0 to 1.0)
    /// Higher = trust model more, Lower = trust market more
    pub confidence: f64,
}

impl MarketConfig {
    /// Create a new market config
    pub fn new(
        ticker: String,
        strike: f64,
        expiry: DateTime<Utc>,
        volatility: f64,
        confidence: f64,
    ) -> Self {
        Self {
            ticker,
            strike,
            expiry,
            volatility,
            confidence,
        }
    }

    /// Calculate hours until expiry
    pub fn hours_to_expiry(&self) -> f64 {
        let now = Utc::now();
        let duration = self.expiry.signed_duration_since(now);
        duration.num_seconds() as f64 / 3600.0
    }

    /// Check if market has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expiry
    }
}
