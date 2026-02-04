//! Calculator module that maintains state from monitors and computes fair price.
//!
//! Tracks probabilities from Kalshi and aggregated BTC price from multiple exchanges.
//! Integrates fair value estimation for BTC binary options.
//! Sends state snapshots to Market Maker for trading decisions.

use std::sync::Arc;
use chrono::Utc;
use tokio::sync::mpsc;
use tracing::info;

use crate::fair_value::{FairValueCalculator, MarketType, BtcMarketSpec, PLACEHOLDER_VOLATILITY};
use crate::redis_client::RedisClient;
use crate::types::{CalculatorStateSnapshot, MonitorUpdate, ProbabilityUpdate};

// =============================================================================
// FEE CALCULATOR
// =============================================================================

/// Kalshi product types with different fee structures
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProductType {
    #[default]
    Standard,
    /// INX Nasdaq100 index contracts have reduced taker fees
    IndexInxNasdaq100,
}

/// Fee rates for Kalshi
mod fee_rates {
    pub const STANDARD_TAKER: f64 = 0.07;
    pub const INDEX_INX_NASDAQ100_TAKER: f64 = 0.035;
    pub const MAKER: f64 = 0.0175;
}

/// Calculate Kalshi trading fee.
///
/// # Arguments
/// * `price_p` - Contract price in dollars (0.0 to 1.0, e.g., 50¢ = 0.50)
/// * `contracts_c` - Number of contracts
/// * `is_taker` - True if order is immediately matched; false if it rests on book
/// * `maker_fee_market` - Whether this market charges maker fees (most don't)
/// * `product_type` - Product type affecting fee rates
///
/// # Returns
/// Fee in dollars, rounded UP to nearest cent
pub fn calculate_fee(
    price_p: f64,
    contracts_c: i32,
    is_taker: bool,
    maker_fee_market: bool,
    product_type: ProductType,
) -> f64 {
    // Edge case: no contracts
    if contracts_c <= 0 {
        return 0.0;
    }

    // Clamp price to valid range
    let price = price_p.clamp(0.0, 1.0);

    // Edge case: price at boundary means no variance, no fee
    if price == 0.0 || price == 1.0 {
        return 0.0;
    }

    // Determine if fee applies and which rate
    let rate = if is_taker {
        // Taker always pays fee
        match product_type {
            ProductType::Standard => fee_rates::STANDARD_TAKER,
            ProductType::IndexInxNasdaq100 => fee_rates::INDEX_INX_NASDAQ100_TAKER,
        }
    } else if maker_fee_market {
        // Maker only pays if market has maker fees
        fee_rates::MAKER
    } else {
        // Maker in non-fee market pays nothing
        return 0.0;
    };

    // Calculate variance-based fee
    let variance = (contracts_c as f64) * price * (1.0 - price);
    let raw_fee = variance * rate;

    // Round UP to nearest cent: ceil(raw_fee * 100) / 100
    (raw_fee * 100.0).ceil() / 100.0
}

/// Minimum change in fair_price (as decimal) to trigger Redis publish
const PUBLISH_THRESHOLD: f64 = 0.001; // 0.1%

/// State maintained by the calculator
#[derive(Debug)]
pub struct CalculatorState {
    /// Ticker ID for this calculator instance
    pub ticker_id: String,
    /// Latest probability from Kalshi (YES side)
    pub kalshi_prob: Option<f64>,
    /// Latest probability from Kalshi (NO side)
    pub kalshi_no_prob: Option<f64>,
    /// Best YES bid in cents
    pub yes_bid: u16,
    /// Best NO bid in cents
    pub no_bid: u16,
    /// Quantity at best YES bid
    pub yes_qty: i64,
    /// Quantity at best NO bid
    pub no_qty: i64,
    /// Timestamp when Kalshi data was received
    pub kalshi_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    /// Aggregated BTC mid-price (mean of all exchanges)
    pub btc_mid_price: Option<f64>,
    /// Aggregated BTC bid price (mean)
    pub btc_bid_price: Option<f64>,
    /// Aggregated BTC ask price (mean)
    pub btc_ask_price: Option<f64>,
    /// Number of exchanges contributing to the price
    pub btc_exchange_count: usize,
    /// Timestamp when BTC price was received
    pub btc_timestamp: Option<String>,
    /// Last Kalshi prob that was published to Redis (for change detection)
    pub last_published_kalshi: Option<f64>,
    /// Last BTC price that was published to Redis (for change detection)
    pub last_published_btc: Option<f64>,
    /// Fair probability calculated from BTC price (model estimate)
    pub fair_prob: Option<f64>,
    /// Blended fair probability (model + market weighted by confidence)
    pub blended_fair_prob: Option<f64>,
    /// Confidence in model (0.0 to 1.0)
    pub confidence: f64,
    /// Fair value calculator (if market spec is parsed)
    fair_value_calc: Option<FairValueCalculator>,
    /// Strike price for this market (if known)
    pub strike_price: Option<f64>,
    /// Expiry time for this market
    pub expiry: Option<chrono::DateTime<chrono::Utc>>,
}

impl CalculatorState {
    pub fn new(ticker_id: String) -> Self {
        // Try to parse the ticker to extract market spec
        let fair_value_calc = FairValueCalculator::from_ticker(&ticker_id);
        let (strike_price, expiry) = fair_value_calc
            .as_ref()
            .map(|fv| (Some(fv.market_spec.strike), Some(fv.market_spec.expiry)))
            .unwrap_or((None, None));

        Self {
            ticker_id,
            kalshi_prob: None,
            kalshi_no_prob: None,
            yes_bid: 0,
            no_bid: 0,
            yes_qty: 0,
            no_qty: 0,
            kalshi_timestamp: None,
            btc_mid_price: None,
            btc_bid_price: None,
            btc_ask_price: None,
            btc_exchange_count: 0,
            btc_timestamp: None,
            last_published_kalshi: None,
            last_published_btc: None,
            fair_prob: None,
            blended_fair_prob: None,
            confidence: 0.5,
            fair_value_calc,
            strike_price,
            expiry,
        }
    }

    /// Create calculator with manual market specification
    /// Use this when the ticker format can't be auto-parsed
    pub fn with_market_spec(
        ticker_id: String,
        strike: f64,
        expiry: chrono::DateTime<chrono::Utc>,
        market_type: MarketType,
    ) -> Self {
        let market_spec = BtcMarketSpec {
            ticker: ticker_id.clone(),
            strike,
            expiry,
            market_type,
        };
        let fair_value_calc = Some(FairValueCalculator::new(market_spec));

        Self {
            ticker_id,
            kalshi_prob: None,
            kalshi_no_prob: None,
            yes_bid: 0,
            no_bid: 0,
            yes_qty: 0,
            no_qty: 0,
            kalshi_timestamp: None,
            btc_mid_price: None,
            btc_bid_price: None,
            btc_ask_price: None,
            btc_exchange_count: 0,
            btc_timestamp: None,
            last_published_kalshi: None,
            last_published_btc: None,
            fair_prob: None,
            blended_fair_prob: None,
            confidence: 0.5,
            fair_value_calc,
            strike_price: Some(strike),
            expiry: Some(expiry),
        }
    }

    /// Set volatility for fair value calculation
    pub fn set_volatility(&mut self, vol: f64) {
        if let Some(ref mut fv) = self.fair_value_calc {
            fv.set_volatility(vol);
        }
    }

    /// Check if state changed enough to warrant publishing
    /// Returns true if Kalshi prob changed by >= 0.1% or BTC price changed by >= $1
    pub fn should_publish(&mut self) -> bool {
        let kalshi_changed = match (self.kalshi_prob, self.last_published_kalshi) {
            (Some(_current), None) => true,
            (Some(current), Some(last)) => (current - last).abs() >= PUBLISH_THRESHOLD,
            _ => false,
        };

        let btc_changed = match (self.btc_mid_price, self.last_published_btc) {
            (Some(_current), None) => true,
            (Some(current), Some(last)) => (current - last).abs() >= 1.0, // $1 threshold
            _ => false,
        };

        if kalshi_changed || btc_changed {
            self.last_published_kalshi = self.kalshi_prob;
            self.last_published_btc = self.btc_mid_price;
            true
        } else {
            false
        }
    }

    /// Update state with new Kalshi data
    pub fn update_kalshi(&mut self, update: &ProbabilityUpdate) {
        self.kalshi_prob = Some(update.yes_prob);
        self.kalshi_no_prob = Some(update.no_prob);
        self.yes_bid = update.yes_bid;
        self.no_bid = update.no_bid;
        self.yes_qty = update.yes_qty;
        self.no_qty = update.no_qty;
        self.kalshi_timestamp = Some(update.timestamp);

        // Update blended fair value if we have model estimate
        self.update_blended_fair_prob();
    }

    /// Update blended fair probability based on model + market
    fn update_blended_fair_prob(&mut self) {
        if let (Some(model_fair), Some(market_yes)) = (self.fair_prob, self.kalshi_prob) {
            // Blend model fair value with market mid price
            // blended = confidence * model + (1 - confidence) * market
            self.blended_fair_prob = Some(
                self.confidence * model_fair + (1.0 - self.confidence) * market_yes
            );
        }
    }

    /// Set confidence level for fair value blending (0.0 to 1.0)
    pub fn set_confidence(&mut self, confidence: f64) {
        self.confidence = confidence.clamp(0.0, 1.0);
        self.update_blended_fair_prob();
    }

    /// Update state with aggregated BTC price data
    /// Also recalculates fair probability if market spec is available
    pub fn update_btc_price(
        &mut self,
        mid_price: f64,
        bid_price: f64,
        ask_price: f64,
        exchange_count: usize,
        timestamp: &str,
    ) {
        self.btc_mid_price = Some(mid_price);
        self.btc_bid_price = Some(bid_price);
        self.btc_ask_price = Some(ask_price);
        self.btc_exchange_count = exchange_count;
        self.btc_timestamp = Some(timestamp.to_string());

        // Calculate fair probability if we have a fair value calculator
        if let Some(ref mut fv) = self.fair_value_calc {
            self.fair_prob = Some(fv.calculate(mid_price));
            // Also update blended fair value
            self.update_blended_fair_prob();
        }
    }

    /// Create a snapshot for the Market Maker
    /// Returns None if required data is missing
    pub fn to_snapshot(&self) -> Option<CalculatorStateSnapshot> {
        // Require both BTC price and fair value to create snapshot
        let btc_mid = self.btc_mid_price?;
        let btc_bid = self.btc_bid_price?;
        let btc_ask = self.btc_ask_price?;
        let model_fair = self.fair_prob?;
        let blended_fair = self.blended_fair_prob.unwrap_or(model_fair);
        let expiry = self.expiry?;

        // Calculate hours to expiry
        let now = Utc::now();
        let duration = expiry.signed_duration_since(now);
        let hours_to_expiry = duration.num_seconds() as f64 / 3600.0;

        Some(CalculatorStateSnapshot {
            ticker: self.ticker_id.clone(),
            btc_mid_price: btc_mid,
            btc_bid_price: btc_bid,
            btc_ask_price: btc_ask,
            exchange_count: self.btc_exchange_count,
            yes_bid: self.yes_bid,
            no_bid: self.no_bid,
            yes_qty: self.yes_qty,
            no_qty: self.no_qty,
            model_fair_prob: model_fair,
            blended_fair_prob: blended_fair,
            hours_to_expiry,
            timestamp: now,
        })
    }

    /// Format state for logging
    pub fn format_log(&self) -> String {
        let kalshi_yes_str = self.kalshi_prob
            .map(|p| format!("{:.1}%", p * 100.0))
            .unwrap_or_else(|| "-".to_string());

        let kalshi_no_str = self.kalshi_no_prob
            .map(|p| format!("{:.1}%", p * 100.0))
            .unwrap_or_else(|| "-".to_string());

        let btc_str = self.btc_mid_price
            .map(|p| format!("${:.2} ({}ex)", p, self.btc_exchange_count))
            .unwrap_or_else(|| "-".to_string());

        let fair_str = self.fair_prob
            .map(|p| format!("{:.1}%", p * 100.0))
            .unwrap_or_else(|| "-".to_string());

        let edge_str = match (self.fair_prob, self.kalshi_prob) {
            (Some(fair), Some(mkt)) => {
                let edge = (fair - mkt) * 100.0;
                format!("{:+.1}¢", edge)
            }
            _ => "-".to_string(),
        };

        format!(
            "kalshi_yes={} | kalshi_no={} | btc={} | fair={} | edge={}",
            kalshi_yes_str, kalshi_no_str, btc_str, fair_str, edge_str
        )
    }

    /// Serialize to JSON for Redis
    pub fn to_json(&self) -> String {
        // Calculate edge if we have both fair and market prob
        let edge = match (self.fair_prob, self.kalshi_prob) {
            (Some(fair), Some(mkt)) => Some(fair - mkt),
            _ => None,
        };

        let obj = serde_json::json!({
            "ticker_id": self.ticker_id,
            "kalshi_prob": self.kalshi_prob,
            "kalshi_no_prob": self.kalshi_no_prob,
            "kalshi_timestamp": self.kalshi_timestamp.map(|t| t.to_rfc3339()),
            "btc_mid_price": self.btc_mid_price,
            "btc_bid_price": self.btc_bid_price,
            "btc_ask_price": self.btc_ask_price,
            "btc_exchange_count": self.btc_exchange_count,
            "btc_timestamp": self.btc_timestamp,
            "fair_prob": self.fair_prob,
            "strike_price": self.strike_price,
            "expiry": self.expiry.map(|t| t.to_rfc3339()),
            "edge": edge,
            "volatility": PLACEHOLDER_VOLATILITY,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        obj.to_string()
    }

    /// Get the fair value calculator (if available)
    pub fn fair_value_calculator(&self) -> Option<&FairValueCalculator> {
        self.fair_value_calc.as_ref()
    }

    /// Get mutable fair value calculator
    pub fn fair_value_calculator_mut(&mut self) -> Option<&mut FairValueCalculator> {
        self.fair_value_calc.as_mut()
    }
}

/// Run the calculator task, receiving updates from monitors
///
/// # Arguments
/// * `ticker_id` - Market ticker ID
/// * `receiver` - Channel receiving updates from monitors
/// * `csv_sender` - Optional channel for CSV logging
/// * `redis` - Optional Redis client for dashboard publishing
/// * `state_sender` - Optional channel to send state snapshots to Market Maker
/// * `confidence` - Fair value blending confidence (0.0 = trust market, 1.0 = trust model)
pub async fn run(
    ticker_id: String,
    mut receiver: mpsc::Receiver<MonitorUpdate>,
    csv_sender: Option<mpsc::Sender<MonitorUpdate>>,
    redis: Option<Arc<RedisClient>>,
    state_sender: Option<mpsc::Sender<CalculatorStateSnapshot>>,
    confidence: f64,
) {
    let mut state = CalculatorState::new(ticker_id.clone());
    state.set_confidence(confidence);

    info!("[CALC] Started for {} (confidence={:.2})", ticker_id, confidence);

    while let Some(update) = receiver.recv().await {
        // Forward to CSV writer (non-blocking)
        if let Some(ref sender) = csv_sender {
            let _ = sender.try_send(update.clone());
        }

        // Process update
        match &update {
            MonitorUpdate::Kalshi(prob_update) => {
                state.update_kalshi(prob_update);
            }
            MonitorUpdate::Crypto(crypto_event) => {
                use crate::types::crypto_aggregator::CryptoAggregatorEvent;
                if let CryptoAggregatorEvent::PriceUpdate(price) = crypto_event {
                    state.update_btc_price(
                        price.mean_mid_price,
                        price.mean_bid_price,
                        price.mean_ask_price,
                        price.exchange_count,
                        &price.timestamp,
                    );
                }
            }
        }

        // Log current state
        info!("[CALC] {} | {}", ticker_id, state.format_log());

        // Send snapshot to Market Maker if channel is connected
        if let Some(ref sender) = state_sender {
            if let Some(snapshot) = state.to_snapshot() {
                // Non-blocking send - if MM is busy, drop the update
                if sender.try_send(snapshot).is_err() {
                    // Channel full or disconnected - this is ok, MM will catch up
                }
            }
        }

        // Publish to Redis if state changed significantly
        if let Some(ref redis) = redis {
            if state.should_publish() {
                crate::redis_client::publish_calculator_update(redis, state.to_json(), &ticker_id);
            }
        }
    }

    info!("[CALC] Stopped for {}", ticker_id);
}
