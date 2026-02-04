//! Type definitions for Kalshi WebSocket messages, orderbook state, and order management.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// =============================================================================
// Trading Environment
// =============================================================================

/// Trading environment selection (Demo vs Production)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingEnvironment {
    Demo,
    Production,
}

impl TradingEnvironment {
    /// REST API base URL
    pub fn api_base_url(&self) -> &'static str {
        match self {
            Self::Demo => "https://demo-api.kalshi.co/trade-api/v2",
            Self::Production => "https://api.elections.kalshi.com/trade-api/v2",
        }
    }

    /// WebSocket URL
    pub fn ws_url(&self) -> &'static str {
        match self {
            Self::Demo => "wss://demo-api.kalshi.co/trade-api/ws/v2",
            Self::Production => "wss://api.elections.kalshi.com/trade-api/ws/v2",
        }
    }

    /// Host header for WebSocket connection
    pub fn ws_host(&self) -> &'static str {
        match self {
            Self::Demo => "demo-api.kalshi.co",
            Self::Production => "api.elections.kalshi.com",
        }
    }

    /// Environment variable prefix for credentials
    pub fn env_key_prefix(&self) -> &'static str {
        match self {
            Self::Demo => "KALSHI_DEMO",
            Self::Production => "KALSHI_PROD",
        }
    }

    /// Display name
    pub fn name(&self) -> &'static str {
        match self {
            Self::Demo => "DEMO",
            Self::Production => "PRODUCTION",
        }
    }
}

impl std::fmt::Display for TradingEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

// =============================================================================
// WebSocket Command Types
// =============================================================================

#[derive(Debug, Serialize)]
pub struct SubscribeCmd {
    pub id: i32,
    pub cmd: &'static str,
    pub params: SubscribeParams,
}

#[derive(Debug, Serialize)]
pub struct SubscribeParams {
    pub channels: Vec<&'static str>,
    pub market_tickers: Vec<String>,
}

// =============================================================================
// WebSocket Response Types
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct WsMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub sid: Option<i32>,
    pub seq: Option<i64>,
    pub msg: Option<WsMessageBody>,
}

#[derive(Debug, Deserialize)]
pub struct WsMessageBody {
    pub market_ticker: Option<String>,
    /// Snapshot: YES bid levels as [[price_cents, quantity], ...]
    pub yes: Option<Vec<Vec<i64>>>,
    /// Snapshot: NO bid levels as [[price_cents, quantity], ...]
    pub no: Option<Vec<Vec<i64>>>,
    /// Delta: price level that changed
    pub price: Option<i64>,
    /// Delta: quantity change (positive = increase, negative = decrease)
    pub delta: Option<i64>,
    /// Delta: which side changed ("yes" or "no")
    pub side: Option<String>,
}

// =============================================================================
// Orderbook State
// =============================================================================

/// Current orderbook state for a single market.
/// Tracks all price levels to provide accurate live top-of-book.
#[derive(Debug, Clone, Default)]
pub struct OrderbookState {
    /// YES bids: price_cents -> quantity
    yes_bids: BTreeMap<u16, i64>,
    /// NO bids: price_cents -> quantity
    no_bids: BTreeMap<u16, i64>,
    /// Cached best YES bid (updated after each change)
    pub yes_bid: u16,
    /// Cached best NO bid (updated after each change)
    pub no_bid: u16,
    /// Cached YES quantity at best price
    pub yes_qty: i64,
    /// Cached NO quantity at best price
    pub no_qty: i64,
}

impl OrderbookState {
    pub fn new() -> Self {
        Self::default()
    }

    /// YES probability as percentage (best YES bid / 100)
    #[inline]
    pub fn yes_probability(&self) -> f64 {
        self.yes_bid as f64 / 100.0
    }

    /// NO probability as percentage (best NO bid / 100)
    #[inline]
    pub fn no_probability(&self) -> f64 {
        self.no_bid as f64 / 100.0
    }

    /// Check if we have valid prices
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.yes_bid > 0 || self.no_bid > 0
    }

    /// Get best YES bid (highest price with qty > 0)
    fn best_yes_bid(&self) -> Option<(u16, i64)> {
        self.yes_bids
            .iter()
            .rev()
            .find(|(_, &qty)| qty > 0)
            .map(|(&p, &q)| (p, q))
    }

    /// Get best NO bid (highest price with qty > 0)
    fn best_no_bid(&self) -> Option<(u16, i64)> {
        self.no_bids
            .iter()
            .rev()
            .find(|(_, &qty)| qty > 0)
            .map(|(&p, &q)| (p, q))
    }

    /// Update cached best bid values from the book
    fn update_cache(&mut self) {
        if let Some((price, qty)) = self.best_yes_bid() {
            self.yes_bid = price;
            self.yes_qty = qty;
        } else {
            self.yes_bid = 0;
            self.yes_qty = 0;
        }

        if let Some((price, qty)) = self.best_no_bid() {
            self.no_bid = price;
            self.no_qty = qty;
        } else {
            self.no_bid = 0;
            self.no_qty = 0;
        }
    }

    /// Update from snapshot message
    pub fn update_from_snapshot(&mut self, body: &WsMessageBody) {
        // Clear and rebuild YES book
        if let Some(levels) = &body.yes {
            self.yes_bids.clear();
            for level in levels {
                if level.len() >= 2 {
                    let price = level[0] as u16;
                    let qty = level[1];
                    // Filter fake prices (0 and 1 cent)
                    if price > 1 && qty > 0 {
                        self.yes_bids.insert(price, qty);
                    }
                }
            }
        }

        // Clear and rebuild NO book
        if let Some(levels) = &body.no {
            self.no_bids.clear();
            for level in levels {
                if level.len() >= 2 {
                    let price = level[0] as u16;
                    let qty = level[1];
                    // Filter fake prices (0 and 1 cent)
                    if price > 1 && qty > 0 {
                        self.no_bids.insert(price, qty);
                    }
                }
            }
        }

        self.update_cache();
    }

    /// Update from delta message
    pub fn update_from_delta(&mut self, body: &WsMessageBody) {
        let Some(side) = &body.side else { return };
        let Some(price) = body.price else { return };
        let delta = body.delta.unwrap_or(0);

        // Ignore fake prices (0 and 1 cent)
        let price = price as u16;
        if price <= 1 {
            return;
        }

        let book = match side.as_str() {
            "yes" => &mut self.yes_bids,
            "no" => &mut self.no_bids,
            _ => return,
        };

        let qty = book.entry(price).or_insert(0);
        *qty += delta;
        if *qty <= 0 {
            book.remove(&price);
        }

        self.update_cache();
    }
}

// =============================================================================
// Probability Update Event
// =============================================================================

/// Probability update event emitted when prices change
#[derive(Debug, Clone, Serialize)]
pub struct ProbabilityUpdate {
    pub market_ticker: String,
    pub yes_prob: f64,
    pub no_prob: f64,
    pub yes_bid: u16,
    pub no_bid: u16,
    pub yes_qty: i64,
    pub no_qty: i64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl ProbabilityUpdate {
    pub fn new(ticker: &str, state: &OrderbookState) -> Self {
        Self {
            market_ticker: ticker.to_string(),
            yes_prob: state.yes_probability(),
            no_prob: state.no_probability(),
            yes_bid: state.yes_bid,
            no_bid: state.no_bid,
            yes_qty: state.yes_qty,
            no_qty: state.no_qty,
            timestamp: chrono::Utc::now(),
        }
    }
}

// =============================================================================
// Order Types
// =============================================================================

/// Order action (buy or sell)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderAction {
    Buy,
    Sell,
}

impl OrderAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

/// Order side (yes or no)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Yes,
    No,
}

impl OrderSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
        }
    }
}

/// Order type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderType {
    Limit,
    Market,
}

/// Time in force for orders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForce {
    Gtc,
    #[serde(rename = "immediate_or_cancel")]
    Ioc,
    Gtd,
}

/// Order status
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderStatus {
    Resting,
    Executed,
    Canceled,
    Pending,
    #[serde(other)]
    Unknown,
}

// =============================================================================
// Order Request/Response
// =============================================================================

/// Request to create a new order
#[derive(Debug, Clone, Serialize)]
pub struct CreateOrderRequest {
    pub ticker: String,
    pub action: OrderAction,
    pub side: OrderSide,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    pub count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yes_price: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_price: Option<i64>,
    pub client_order_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<TimeInForce>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_ts: Option<i64>,
}

impl CreateOrderRequest {
    pub fn generate_client_order_id() -> String {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        format!("km_{}{}", ts, counter)
    }

    pub fn limit_buy(ticker: &str, side: OrderSide, price_cents: i64, count: i64) -> Self {
        let (yes_price, no_price) = match side {
            OrderSide::Yes => (Some(price_cents), None),
            OrderSide::No => (None, Some(price_cents)),
        };
        Self {
            ticker: ticker.to_string(),
            action: OrderAction::Buy,
            side,
            order_type: OrderType::Limit,
            count,
            yes_price,
            no_price,
            client_order_id: Self::generate_client_order_id(),
            time_in_force: None,
            expiration_ts: None,
        }
    }

    pub fn limit_sell(ticker: &str, side: OrderSide, price_cents: i64, count: i64) -> Self {
        let (yes_price, no_price) = match side {
            OrderSide::Yes => (Some(price_cents), None),
            OrderSide::No => (None, Some(price_cents)),
        };
        Self {
            ticker: ticker.to_string(),
            action: OrderAction::Sell,
            side,
            order_type: OrderType::Limit,
            count,
            yes_price,
            no_price,
            client_order_id: Self::generate_client_order_id(),
            time_in_force: None,
            expiration_ts: None,
        }
    }

    pub fn ioc_buy(ticker: &str, side: OrderSide, price_cents: i64, count: i64) -> Self {
        let mut req = Self::limit_buy(ticker, side, price_cents, count);
        req.time_in_force = Some(TimeInForce::Ioc);
        req
    }

    pub fn ioc_sell(ticker: &str, side: OrderSide, price_cents: i64, count: i64) -> Self {
        let mut req = Self::limit_sell(ticker, side, price_cents, count);
        req.time_in_force = Some(TimeInForce::Ioc);
        req
    }
}

/// Request to amend an existing order
#[derive(Debug, Clone, Serialize)]
pub struct AmendOrderRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
}

/// Response from order creation
#[derive(Debug, Clone, Deserialize)]
pub struct OrderResponse {
    pub order: Order,
}

/// Order details
#[derive(Debug, Clone, Deserialize)]
pub struct Order {
    pub order_id: String,
    pub ticker: String,
    pub status: OrderStatus,
    pub action: String,
    pub side: String,
    #[serde(rename = "type")]
    pub order_type: String,
    #[serde(default)]
    pub yes_price: Option<i64>,
    #[serde(default)]
    pub no_price: Option<i64>,
    #[serde(default)]
    pub count: Option<i64>,
    #[serde(default)]
    pub remaining_count: Option<i64>,
    #[serde(default)]
    pub taker_fill_count: Option<i64>,
    #[serde(default)]
    pub maker_fill_count: Option<i64>,
    #[serde(default)]
    pub taker_fill_cost: Option<i64>,
    #[serde(default)]
    pub maker_fill_cost: Option<i64>,
    #[serde(default)]
    pub created_time: Option<String>,
    #[serde(default)]
    pub client_order_id: Option<String>,
}

impl Order {
    pub fn filled_count(&self) -> i64 {
        self.taker_fill_count.unwrap_or(0) + self.maker_fill_count.unwrap_or(0)
    }

    pub fn fill_cost(&self) -> i64 {
        self.taker_fill_cost.unwrap_or(0) + self.maker_fill_cost.unwrap_or(0)
    }

    pub fn is_filled(&self) -> bool {
        matches!(self.status, OrderStatus::Executed) || self.remaining_count == Some(0)
    }

    pub fn is_resting(&self) -> bool {
        matches!(self.status, OrderStatus::Resting)
    }

    pub fn price_cents(&self) -> Option<i64> {
        self.yes_price.or(self.no_price)
    }
}

/// Response for list orders
#[derive(Debug, Clone, Deserialize)]
pub struct OrdersResponse {
    pub orders: Vec<Order>,
    #[serde(default)]
    pub cursor: Option<String>,
}

// =============================================================================
// Position Types
// =============================================================================

/// Market position
#[derive(Debug, Clone, Deserialize)]
pub struct Position {
    pub ticker: String,
    #[serde(default)]
    pub market_exposure: i64,
    #[serde(default)]
    pub total_traded: i64,
    #[serde(default)]
    pub realized_pnl: i64,
    #[serde(default)]
    pub resting_orders_count: i64,
}

/// Response for get positions
#[derive(Debug, Clone, Deserialize)]
pub struct PositionsResponse {
    #[serde(default)]
    pub market_positions: Vec<Position>,
    #[serde(default)]
    pub cursor: Option<String>,
}

// =============================================================================
// Balance Types
// =============================================================================

/// Account balance
#[derive(Debug, Clone, Deserialize)]
pub struct Balance {
    #[serde(default)]
    pub balance: i64,
    #[serde(default)]
    pub portfolio_value: i64,
}

/// Response for get balance
#[derive(Debug, Clone, Deserialize)]
pub struct BalanceResponse {
    #[serde(flatten)]
    pub balance: Balance,
}
