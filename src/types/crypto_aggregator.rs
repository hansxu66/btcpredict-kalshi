//! Type definitions for the aggregated crypto price monitor.

use serde::Serialize;
use std::collections::HashMap;

/// Supported exchanges for price aggregation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Exchange {
    Binance,
    Coinbase,
    Kraken,
    Cryptocom,
}

impl std::fmt::Display for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Exchange::Binance => write!(f, "binance"),
            Exchange::Coinbase => write!(f, "coinbase"),
            Exchange::Kraken => write!(f, "kraken"),
            Exchange::Cryptocom => write!(f, "cryptocom"),
        }
    }
}

/// Price data from a single exchange
#[derive(Debug, Clone, Serialize)]
pub struct ExchangePrice {
    pub exchange: Exchange,
    pub bid_price: f64,
    pub ask_price: f64,
    pub mid_price: f64,
    pub timestamp: String,
}

/// Aggregated price state across all exchanges
#[derive(Debug, Clone, Default)]
pub struct AggregatorState {
    /// Latest price from each exchange
    pub prices: HashMap<Exchange, ExchangePrice>,
}

impl AggregatorState {
    pub fn new() -> Self {
        Self {
            prices: HashMap::new(),
        }
    }

    /// Update price for a specific exchange
    pub fn update(&mut self, price: ExchangePrice) {
        self.prices.insert(price.exchange, price);
    }

    /// Calculate mean mid-price across all exchanges with data
    pub fn mean_mid_price(&self) -> Option<f64> {
        if self.prices.is_empty() {
            return None;
        }
        let sum: f64 = self.prices.values().map(|p| p.mid_price).sum();
        Some(sum / self.prices.len() as f64)
    }

    /// Calculate mean bid price
    pub fn mean_bid_price(&self) -> Option<f64> {
        if self.prices.is_empty() {
            return None;
        }
        let sum: f64 = self.prices.values().map(|p| p.bid_price).sum();
        Some(sum / self.prices.len() as f64)
    }

    /// Calculate mean ask price
    pub fn mean_ask_price(&self) -> Option<f64> {
        if self.prices.is_empty() {
            return None;
        }
        let sum: f64 = self.prices.values().map(|p| p.ask_price).sum();
        Some(sum / self.prices.len() as f64)
    }

    /// Get number of exchanges with data
    pub fn exchange_count(&self) -> usize {
        self.prices.len()
    }
}

/// Events emitted by the aggregated crypto monitor
#[derive(Debug, Clone, Serialize)]
pub enum CryptoAggregatorEvent {
    /// Successfully connected to an exchange
    ExchangeConnected(Exchange),
    /// Disconnected from an exchange
    ExchangeDisconnected(Exchange),
    /// Aggregated price update (emitted on any price change)
    PriceUpdate(AggregatedPriceUpdate),
}

/// Aggregated BTC price update
#[derive(Debug, Clone, Serialize)]
pub struct AggregatedPriceUpdate {
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// Mean mid-price across all exchanges
    pub mean_mid_price: f64,
    /// Mean bid price
    pub mean_bid_price: f64,
    /// Mean ask price
    pub mean_ask_price: f64,
    /// Number of exchanges contributing to the mean
    pub exchange_count: usize,
    /// Which exchange triggered this update
    pub triggered_by: Exchange,
    /// Individual exchange prices
    pub exchange_prices: HashMap<Exchange, f64>,
}
