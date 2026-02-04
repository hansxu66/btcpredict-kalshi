//! Market making strategy engine with directional signals.
//!
//! Implements inventory-aware quoting with alpha from fair value estimation.
//! Supports both passive market making and aggressive taking when edge is large.

use crate::calculator::{calculate_fee, ProductType};
use crate::fair_value::FairValueCalculator;
use crate::trading_apis::KalshiClient;
use crate::types::{Order, OrderSide};
use anyhow::Result;
use std::collections::HashMap;
use tracing::{debug, info, warn};

// =============================================================================
// CONFIGURATION
// =============================================================================

/// Market maker configuration
#[derive(Debug, Clone)]
pub struct MarketMakerConfig {
    /// Maximum loss allowed per market (in dollars)
    /// Position will be limited so worst-case loss doesn't exceed this
    pub max_loss_per_market: f64,

    /// Base spread width (in probability points, e.g., 0.02 = 2 cents)
    pub base_spread: f64,

    /// Minimum edge required to post a quote (after fees)
    pub min_edge_to_quote: f64,

    /// Edge threshold for aggressive taking (in probability points)
    /// If edge > this, take liquidity instead of posting
    pub aggressive_take_threshold: f64,

    /// Inventory skew factor - how much to adjust quotes per contract of inventory
    /// Higher = more aggressive inventory reduction
    pub inventory_skew_factor: f64,

    /// Maximum inventory (contracts) before refusing to add more
    pub max_inventory: i64,

    /// Minimum time to expiry (hours) to trade - avoid last-minute risk
    pub min_hours_to_expiry: f64,

    /// Whether market has maker fees (most Kalshi markets don't)
    pub maker_fee_market: bool,

    /// Product type for fee calculation
    pub product_type: ProductType,

    /// Confidence in fair value model (0.0 to 1.0)
    /// - 1.0 = fully trust model, use 100% model fair value
    /// - 0.5 = blend 50% model + 50% market mid price
    /// - 0.0 = pure market making, use 100% market mid (no alpha)
    ///
    /// When uncertain about your model, lower this value to:
    /// - Blend towards market consensus
    /// - Effectively widen your edge requirements
    /// - Reduce directional risk from model error
    pub fair_value_confidence: f64,
}

impl Default for MarketMakerConfig {
    fn default() -> Self {
        Self {
            max_loss_per_market: 100.0, // $100 max loss
            base_spread: 0.03,           // 3 cent base spread
            min_edge_to_quote: 0.005,    // 0.5 cent min edge after fees
            aggressive_take_threshold: 0.03, // 3 cent edge to take
            inventory_skew_factor: 0.001,    // 0.1 cent per contract
            max_inventory: 500,
            min_hours_to_expiry: 0.5, // 30 minutes minimum
            maker_fee_market: false,
            product_type: ProductType::Standard,
            fair_value_confidence: 0.5, // Default: 50% model + 50% market
        }
    }
}

// =============================================================================
// POSITION & INVENTORY
// =============================================================================

/// Current position state
#[derive(Debug, Clone, Default)]
pub struct PositionState {
    /// Net YES contracts held (positive = long YES, negative = short YES/long NO)
    pub yes_position: i64,
    /// Average entry price for YES position (in probability 0-1)
    pub avg_entry_price: f64,
    /// Total cost basis (dollars spent)
    pub cost_basis: f64,
    /// Realized P&L (from closed trades)
    pub realized_pnl: f64,
}

impl PositionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate max loss if market expires at worst-case outcome
    ///
    /// If long YES (yes_position > 0):
    ///   - Worst case: expires NO (worth 0)
    ///   - Max loss = cost_basis (what we paid)
    ///
    /// If short YES / long NO (yes_position < 0):
    ///   - Worst case: expires YES (we owe $1 per contract)
    ///   - Max loss = contracts * $1 - cost_basis (we received premium)
    pub fn max_loss(&self) -> f64 {
        if self.yes_position > 0 {
            // Long YES: lose what we paid if expires at 0
            self.cost_basis
        } else if self.yes_position < 0 {
            // Short YES (long NO): lose if expires at 1
            // We received premium, but have to pay out $1 per contract
            let contracts = self.yes_position.abs() as f64;
            (contracts * 1.0) - self.cost_basis.abs()
        } else {
            0.0
        }
    }

    /// Calculate unrealized P&L given current fair probability
    pub fn unrealized_pnl(&self, current_fair_prob: f64) -> f64 {
        if self.yes_position == 0 {
            return 0.0;
        }

        let contracts = self.yes_position.abs() as f64;
        let current_value = contracts * current_fair_prob;

        if self.yes_position > 0 {
            // Long YES
            current_value - self.cost_basis
        } else {
            // Short YES (long NO)
            self.cost_basis - current_value
        }
    }

    /// Update position after a fill
    pub fn update_from_fill(&mut self, side: OrderSide, is_buy: bool, contracts: i64, price: f64) {
        let signed_contracts = match (side, is_buy) {
            (OrderSide::Yes, true) => contracts,      // Buy YES = long YES
            (OrderSide::Yes, false) => -contracts,    // Sell YES = reduce long
            (OrderSide::No, true) => -contracts,      // Buy NO = short YES
            (OrderSide::No, false) => contracts,      // Sell NO = reduce short
        };

        let cost = contracts as f64 * price;

        if (self.yes_position >= 0 && signed_contracts > 0)
            || (self.yes_position <= 0 && signed_contracts < 0)
        {
            // Adding to position
            let total_contracts = self.yes_position.abs() + contracts;
            if total_contracts > 0 {
                self.avg_entry_price = (self.avg_entry_price * self.yes_position.abs() as f64
                    + price * contracts as f64)
                    / total_contracts as f64;
            }
            self.cost_basis += cost;
        } else {
            // Reducing position - realize P&L
            let contracts_closed = contracts.min(self.yes_position.abs());
            let pnl = contracts_closed as f64 * (price - self.avg_entry_price);
            self.realized_pnl += if self.yes_position > 0 { pnl } else { -pnl };
            self.cost_basis -= contracts_closed as f64 * self.avg_entry_price;
        }

        self.yes_position += signed_contracts;
    }

    /// Calculate maximum contracts we can add given max loss constraint
    pub fn max_contracts_to_add(&self, price: f64, max_loss: f64, side: OrderSide, is_buy: bool) -> i64 {
        let current_max_loss = self.max_loss();
        let remaining_loss_budget = (max_loss - current_max_loss).max(0.0);

        match (side, is_buy) {
            (OrderSide::Yes, true) => {
                // Buying YES: max loss = price * contracts
                if price > 0.0 {
                    (remaining_loss_budget / price) as i64
                } else {
                    0
                }
            }
            (OrderSide::No, true) => {
                // Buying NO: max loss = price * contracts
                if price > 0.0 {
                    (remaining_loss_budget / price) as i64
                } else {
                    0
                }
            }
            (OrderSide::Yes, false) | (OrderSide::No, false) => {
                // Selling (closing): no max loss constraint, limited by position
                self.yes_position.abs()
            }
        }
    }
}

// =============================================================================
// EDGE CALCULATION
// =============================================================================

/// Edge calculation result
#[derive(Debug, Clone)]
pub struct EdgeCalculation {
    /// Fair probability (YES side)
    pub fair_prob: f64,
    /// Market YES bid price (best bid to sell into)
    pub market_yes_bid: f64,
    /// Market YES ask price (best ask to buy)
    pub market_yes_ask: f64,
    /// Market NO bid price
    pub market_no_bid: f64,
    /// Market NO ask price
    pub market_no_ask: f64,
    /// Raw edge for buying YES = fair_prob - ask (positive = profitable to buy)
    /// NOTE: This is BEFORE fees. Use edge_after_fees() for trading decisions.
    pub yes_buy_edge_raw: f64,
    /// Raw edge for selling YES = bid - fair_prob (positive = profitable to sell)
    pub yes_sell_edge_raw: f64,
    /// Raw edge for buying NO = (1 - fair_prob) - no_ask
    pub no_buy_edge_raw: f64,
    /// Raw edge for selling NO = no_bid - (1 - fair_prob)
    pub no_sell_edge_raw: f64,
    /// Edge after taker fees (for aggressive taking)
    pub yes_buy_edge_net: f64,
    pub yes_sell_edge_net: f64,
    pub no_buy_edge_net: f64,
    pub no_sell_edge_net: f64,
    /// Edge after maker fees (for passive quoting)
    pub yes_buy_edge_maker: f64,
    pub yes_sell_edge_maker: f64,
    pub no_buy_edge_maker: f64,
    pub no_sell_edge_maker: f64,
}

impl EdgeCalculation {
    /// Calculate edges given fair probability and market prices
    ///
    /// # Arguments
    /// * `fair_prob` - Fair probability for YES (0-1)
    /// * `yes_bid` - Best YES bid in cents (e.g., 55 = $0.55)
    /// * `no_bid` - Best NO bid in cents
    /// * `maker_fee_market` - Whether this market charges maker fees
    /// * `product_type` - Product type for fee calculation
    pub fn calculate(
        fair_prob: f64,
        yes_bid: u16,
        no_bid: u16,
        maker_fee_market: bool,
        product_type: ProductType,
    ) -> Self {
        // Convert cents to probability (0-1)
        let market_yes_bid = yes_bid as f64 / 100.0;
        let market_no_bid = no_bid as f64 / 100.0;

        // In Kalshi, buying YES at X is equivalent to selling NO at (1-X)
        // The "ask" for YES is effectively (1 - NO bid) because someone buying YES
        // is taking the other side of someone selling NO
        let market_yes_ask = 1.0 - market_no_bid;
        let market_no_ask = 1.0 - market_yes_bid;

        let fair_prob_no = 1.0 - fair_prob;

        // Raw edges (before fees)
        let yes_buy_edge_raw = fair_prob - market_yes_ask;
        let yes_sell_edge_raw = market_yes_bid - fair_prob;
        let no_buy_edge_raw = fair_prob_no - market_no_ask;
        let no_sell_edge_raw = market_no_bid - fair_prob_no;

        // Calculate fees for each trade direction
        // Fee = price * (1-price) * rate (per contract)
        let yes_buy_taker_fee = calculate_fee(market_yes_ask, 1, true, maker_fee_market, product_type);
        let yes_sell_taker_fee = calculate_fee(market_yes_bid, 1, true, maker_fee_market, product_type);
        let no_buy_taker_fee = calculate_fee(market_no_ask, 1, true, maker_fee_market, product_type);
        let no_sell_taker_fee = calculate_fee(market_no_bid, 1, true, maker_fee_market, product_type);

        let yes_buy_maker_fee = calculate_fee(market_yes_ask, 1, false, maker_fee_market, product_type);
        let yes_sell_maker_fee = calculate_fee(market_yes_bid, 1, false, maker_fee_market, product_type);
        let no_buy_maker_fee = calculate_fee(market_no_ask, 1, false, maker_fee_market, product_type);
        let no_sell_maker_fee = calculate_fee(market_no_bid, 1, false, maker_fee_market, product_type);

        Self {
            fair_prob,
            market_yes_bid,
            market_yes_ask,
            market_no_bid,
            market_no_ask,
            yes_buy_edge_raw,
            yes_sell_edge_raw,
            no_buy_edge_raw,
            no_sell_edge_raw,
            // Net edge after taker fees (for aggressive taking)
            yes_buy_edge_net: yes_buy_edge_raw - yes_buy_taker_fee,
            yes_sell_edge_net: yes_sell_edge_raw - yes_sell_taker_fee,
            no_buy_edge_net: no_buy_edge_raw - no_buy_taker_fee,
            no_sell_edge_net: no_sell_edge_raw - no_sell_taker_fee,
            // Net edge after maker fees (for passive quoting)
            yes_buy_edge_maker: yes_buy_edge_raw - yes_buy_maker_fee,
            yes_sell_edge_maker: yes_sell_edge_raw - yes_sell_maker_fee,
            no_buy_edge_maker: no_buy_edge_raw - no_buy_maker_fee,
            no_sell_edge_maker: no_sell_edge_raw - no_sell_maker_fee,
        }
    }

    /// Best raw edge available (before fees)
    pub fn best_edge_raw(&self) -> f64 {
        self.yes_buy_edge_raw
            .max(self.yes_sell_edge_raw)
            .max(self.no_buy_edge_raw)
            .max(self.no_sell_edge_raw)
            .max(0.0)
    }

    /// Best net edge after taker fees (for aggressive taking decisions)
    pub fn best_edge_net(&self) -> f64 {
        self.yes_buy_edge_net
            .max(self.yes_sell_edge_net)
            .max(self.no_buy_edge_net)
            .max(self.no_sell_edge_net)
            .max(0.0)
    }

    /// Best action to take (based on NET edge after taker fees)
    pub fn best_action(&self) -> Option<TradeAction> {
        let edges = [
            (self.yes_buy_edge_net, TradeAction::BuyYes),
            (self.yes_sell_edge_net, TradeAction::SellYes),
            (self.no_buy_edge_net, TradeAction::BuyNo),
            (self.no_sell_edge_net, TradeAction::SellNo),
        ];

        edges
            .iter()
            .filter(|(edge, _)| *edge > 0.0)
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
            .map(|(_, action)| *action)
    }

    /// Get net edge for a specific action (after taker fees)
    pub fn edge_for_action(&self, action: TradeAction) -> f64 {
        match action {
            TradeAction::BuyYes => self.yes_buy_edge_net,
            TradeAction::SellYes => self.yes_sell_edge_net,
            TradeAction::BuyNo => self.no_buy_edge_net,
            TradeAction::SellNo => self.no_sell_edge_net,
        }
    }
}

/// Possible trade actions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeAction {
    BuyYes,
    SellYes,
    BuyNo,
    SellNo,
}

impl TradeAction {
    pub fn side(&self) -> OrderSide {
        match self {
            Self::BuyYes | Self::SellYes => OrderSide::Yes,
            Self::BuyNo | Self::SellNo => OrderSide::No,
        }
    }

    pub fn is_buy(&self) -> bool {
        matches!(self, Self::BuyYes | Self::BuyNo)
    }
}

// =============================================================================
// QUOTE GENERATION
// =============================================================================

/// A quote to post or action to take
#[derive(Debug, Clone)]
pub enum MarketMakerSignal {
    /// Post a passive quote (maker order)
    Quote(QuoteOrder),
    /// Take liquidity aggressively (taker order)
    AggressiveTake(AggressiveOrder),
    /// Amend an existing order
    AmendOrder { order_id: String, new_price: i64, new_count: i64 },
    /// Cancel an order
    CancelOrder { order_id: String },
    /// Cancel all orders for safety
    CancelAll { reason: String },
    /// Do nothing (no edge or risk limits)
    Hold { reason: String },
}

/// Passive quote order
#[derive(Debug, Clone)]
pub struct QuoteOrder {
    pub side: OrderSide,
    pub is_buy: bool,
    pub price_cents: i64,
    pub contracts: i64,
    pub edge: f64,
}

/// Aggressive take order
#[derive(Debug, Clone)]
pub struct AggressiveOrder {
    pub side: OrderSide,
    pub is_buy: bool,
    pub price_cents: i64,
    pub contracts: i64,
    pub edge: f64,
}

// =============================================================================
// MARKET MAKER ENGINE
// =============================================================================

/// Market maker strategy engine
pub struct MarketMaker {
    /// Market ticker
    pub ticker: String,
    /// Configuration
    pub config: MarketMakerConfig,
    /// Fair value calculator
    pub fair_value_calc: FairValueCalculator,
    /// Current position
    pub position: PositionState,
    /// Active orders (order_id -> order)
    pub active_orders: HashMap<String, Order>,
    /// Last calculated edge
    pub last_edge: Option<EdgeCalculation>,
    /// Last model fair value (before blending)
    pub last_model_fair: Option<f64>,
    /// Last market mid price
    pub last_market_mid: Option<f64>,
    /// Last blended fair value (what we actually use for trading)
    pub last_blended_fair: Option<f64>,
}

impl MarketMaker {
    /// Create a new market maker for a ticker
    pub fn new(ticker: String, fair_value_calc: FairValueCalculator, config: MarketMakerConfig) -> Self {
        Self {
            ticker,
            config,
            fair_value_calc,
            position: PositionState::new(),
            active_orders: HashMap::new(),
            last_edge: None,
            last_model_fair: None,
            last_market_mid: None,
            last_blended_fair: None,
        }
    }

    /// Update fair value with new spot price and return MODEL fair probability
    /// (before blending with market)
    pub fn update_model_fair_value(&mut self, spot_price: f64) -> f64 {
        let model_fair = self.fair_value_calc.calculate(spot_price);
        self.last_model_fair = Some(model_fair);
        model_fair
    }

    /// Blend model fair value with market mid price based on confidence
    ///
    /// Formula: blended = confidence * model + (1 - confidence) * market_mid
    ///
    /// - confidence = 1.0 → 100% model (full alpha)
    /// - confidence = 0.5 → 50/50 blend
    /// - confidence = 0.0 → 100% market (pure MM, no directional view)
    ///
    /// # Arguments
    /// * `model_fair` - Fair probability from the model (e.g., Black-Scholes)
    /// * `market_mid` - Market mid price: (best_bid + best_ask) / 2
    ///
    /// # Returns
    /// Blended fair probability to use for quoting
    pub fn blend_fair_value(&mut self, model_fair: f64, market_mid: f64) -> f64 {
        let confidence = self.config.fair_value_confidence.clamp(0.0, 1.0);
        let blended = confidence * model_fair + (1.0 - confidence) * market_mid;

        self.last_market_mid = Some(market_mid);
        self.last_blended_fair = Some(blended);

        blended
    }

    /// Calculate market mid price from YES bid and NO bid
    ///
    /// Market mid = (yes_bid + yes_ask) / 2
    /// where yes_ask = 1 - no_bid
    pub fn calculate_market_mid(yes_bid: u16, no_bid: u16) -> f64 {
        let yes_bid_prob = yes_bid as f64 / 100.0;
        let yes_ask_prob = 1.0 - (no_bid as f64 / 100.0);
        (yes_bid_prob + yes_ask_prob) / 2.0
    }

    /// Get effective fair value for trading decisions
    /// This is the blended value (or model value if confidence = 1.0)
    pub fn effective_fair_value(&self) -> Option<f64> {
        self.last_blended_fair.or(self.last_model_fair)
    }

    /// Calculate dynamic spread based on market conditions
    ///
    /// Spread widens when:
    /// - Volatility is high
    /// - Time to expiry is short (gamma risk)
    /// - Inventory is large (adverse selection)
    fn calculate_spread(&self) -> f64 {
        let mut spread = self.config.base_spread;

        // Widen spread based on inventory
        let inventory_adjustment =
            self.position.yes_position.abs() as f64 * self.config.inventory_skew_factor;
        spread += inventory_adjustment;

        // Widen spread near expiry (gamma risk)
        let hours_to_expiry = self.fair_value_calc.market_spec.time_to_expiry_hours();
        if hours_to_expiry < 2.0 && hours_to_expiry > 0.0 {
            spread *= 2.0 - (hours_to_expiry / 2.0); // Up to 2x spread in last 2 hours
        }

        // Cap spread at reasonable level
        spread.min(0.10) // Max 10 cent spread
    }

    /// Calculate inventory skew (how much to shift quotes to reduce inventory)
    fn calculate_inventory_skew(&self) -> f64 {
        self.position.yes_position as f64 * self.config.inventory_skew_factor
    }

    /// Calculate fee for a trade
    fn calculate_trade_fee(&self, price: f64, contracts: i64, is_taker: bool) -> f64 {
        calculate_fee(
            price,
            contracts as i32,
            is_taker,
            self.config.maker_fee_market,
            self.config.product_type,
        )
    }

    /// Generate trading signals based on current state
    ///
    /// # Arguments
    /// * `spot_price` - Current BTC spot price
    /// * `yes_bid` - Best YES bid in cents
    /// * `no_bid` - Best NO bid in cents
    ///
    /// # Returns
    /// Vector of signals (quotes to post, orders to cancel, etc.)
    pub fn generate_signals(
        &mut self,
        spot_price: f64,
        yes_bid: u16,
        no_bid: u16,
    ) -> Vec<MarketMakerSignal> {
        let mut signals = Vec::new();

        // Update MODEL fair value from spot price
        let model_fair = self.update_model_fair_value(spot_price);

        // Calculate market mid price
        let market_mid = Self::calculate_market_mid(yes_bid, no_bid);

        // Blend model fair value with market mid based on confidence
        let fair_prob = self.blend_fair_value(model_fair, market_mid);

        // Check time to expiry
        let hours_to_expiry = self.fair_value_calc.market_spec.time_to_expiry_hours();
        if hours_to_expiry < self.config.min_hours_to_expiry {
            signals.push(MarketMakerSignal::CancelAll {
                reason: format!("Too close to expiry ({:.1}h)", hours_to_expiry),
            });
            return signals;
        }

        // Check if expired
        if self.fair_value_calc.market_spec.is_expired() {
            signals.push(MarketMakerSignal::CancelAll {
                reason: "Market expired".to_string(),
            });
            return signals;
        }

        // Calculate edge (with fees baked in) using BLENDED fair value
        let edge = EdgeCalculation::calculate(
            fair_prob,
            yes_bid,
            no_bid,
            self.config.maker_fee_market,
            self.config.product_type,
        );
        self.last_edge = Some(edge.clone());

        // Log with both model and blended fair values
        info!(
            "[MM] {} | model={:.1}% | mkt_mid={:.1}% | blended={:.1}% (conf={:.0}%) | raw_edge={:.1}¢ | net_edge={:.1}¢ | pos={}",
            self.ticker,
            model_fair * 100.0,
            market_mid * 100.0,
            fair_prob * 100.0,
            self.config.fair_value_confidence * 100.0,
            edge.best_edge_raw() * 100.0,
            edge.best_edge_net() * 100.0,
            self.position.yes_position
        );

        // Check max inventory
        if self.position.yes_position.abs() >= self.config.max_inventory {
            signals.push(MarketMakerSignal::Hold {
                reason: "Max inventory reached".to_string(),
            });
            // Could still post reducing quotes
        }

        // Check max loss
        let current_max_loss = self.position.max_loss();
        if current_max_loss >= self.config.max_loss_per_market {
            signals.push(MarketMakerSignal::Hold {
                reason: format!("Max loss limit reached (${:.2})", current_max_loss),
            });
            // Could still post reducing quotes
        }

        // Calculate spread and skew
        let spread = self.calculate_spread();
        let skew = self.calculate_inventory_skew();

        // --- Aggressive Taking Logic ---
        // If edge is large enough (after fees), take liquidity
        if let Some(action) = edge.best_action() {
            // Get the net edge (already has taker fees subtracted)
            let edge_after_fees = edge.edge_for_action(action);

            let price = match action {
                TradeAction::BuyYes => edge.market_yes_ask,
                TradeAction::SellYes => edge.market_yes_bid,
                TradeAction::BuyNo => edge.market_no_ask,
                TradeAction::SellNo => edge.market_no_bid,
            };

            if edge_after_fees > self.config.aggressive_take_threshold {
                // Calculate size based on max loss
                let max_contracts = self.position.max_contracts_to_add(
                    price,
                    self.config.max_loss_per_market,
                    action.side(),
                    action.is_buy(),
                );
                let contracts = max_contracts.min(self.config.max_inventory - self.position.yes_position.abs());

                if contracts > 0 {
                    signals.push(MarketMakerSignal::AggressiveTake(AggressiveOrder {
                        side: action.side(),
                        is_buy: action.is_buy(),
                        price_cents: (price * 100.0).round() as i64,
                        contracts,
                        edge: edge_after_fees,
                    }));

                    info!(
                        "[MM] AGGRESSIVE TAKE: {:?} {} @ {:.0}¢ | edge={:.1}¢ (after fees)",
                        action,
                        contracts,
                        price * 100.0,
                        edge_after_fees * 100.0
                    );
                }
            }
        }

        // --- Passive Quoting Logic ---
        // Post bids and asks around fair value with spread and inventory skew

        let half_spread = spread / 2.0;

        // YES bid (to buy YES) - lower if we're already long
        let yes_bid_price = fair_prob - half_spread - skew;
        let yes_bid_edge_raw = fair_prob - yes_bid_price;
        let yes_bid_maker_fee = self.calculate_trade_fee(yes_bid_price, 1, false);
        let yes_bid_edge_net = yes_bid_edge_raw - yes_bid_maker_fee;

        if yes_bid_price > 0.02
            && yes_bid_price < 0.98
            && yes_bid_edge_net > self.config.min_edge_to_quote
            && self.position.yes_position < self.config.max_inventory
        {
            let max_contracts = self.position.max_contracts_to_add(
                yes_bid_price,
                self.config.max_loss_per_market,
                OrderSide::Yes,
                true,
            );
            if max_contracts > 0 {
                signals.push(MarketMakerSignal::Quote(QuoteOrder {
                    side: OrderSide::Yes,
                    is_buy: true,
                    price_cents: (yes_bid_price * 100.0).round() as i64,
                    contracts: max_contracts.min(100), // Cap single order size
                    edge: yes_bid_edge_net,
                }));
            }
        }

        // YES ask (to sell YES) - higher if we're already short
        let yes_ask_price = fair_prob + half_spread - skew;
        let yes_ask_edge_raw = yes_ask_price - fair_prob;
        let yes_ask_maker_fee = self.calculate_trade_fee(yes_ask_price, 1, false);
        let yes_ask_edge_net = yes_ask_edge_raw - yes_ask_maker_fee;

        if yes_ask_price > 0.02
            && yes_ask_price < 0.98
            && yes_ask_edge_net > self.config.min_edge_to_quote
            && self.position.yes_position > -self.config.max_inventory
        {
            let max_contracts = self.position.max_contracts_to_add(
                yes_ask_price,
                self.config.max_loss_per_market,
                OrderSide::Yes,
                false,
            );
            if max_contracts > 0 || self.position.yes_position > 0 {
                // Can sell if we have position
                let contracts = if self.position.yes_position > 0 {
                    self.position.yes_position.min(100)
                } else {
                    max_contracts.min(100)
                };
                if contracts > 0 {
                    signals.push(MarketMakerSignal::Quote(QuoteOrder {
                        side: OrderSide::Yes,
                        is_buy: false,
                        price_cents: (yes_ask_price * 100.0).round() as i64,
                        contracts,
                        edge: yes_ask_edge_net,
                    }));
                }
            }
        }

        // NO side quotes (similar logic, inverted)
        let fair_prob_no = 1.0 - fair_prob;

        // NO bid (to buy NO = short YES)
        let no_bid_price = fair_prob_no - half_spread + skew;
        let no_bid_edge_raw = fair_prob_no - no_bid_price;
        let no_bid_maker_fee = self.calculate_trade_fee(no_bid_price, 1, false);
        let no_bid_edge_net = no_bid_edge_raw - no_bid_maker_fee;

        if no_bid_price > 0.02
            && no_bid_price < 0.98
            && no_bid_edge_net > self.config.min_edge_to_quote
            && self.position.yes_position > -self.config.max_inventory
        {
            let max_contracts = self.position.max_contracts_to_add(
                no_bid_price,
                self.config.max_loss_per_market,
                OrderSide::No,
                true,
            );
            if max_contracts > 0 {
                signals.push(MarketMakerSignal::Quote(QuoteOrder {
                    side: OrderSide::No,
                    is_buy: true,
                    price_cents: (no_bid_price * 100.0).round() as i64,
                    contracts: max_contracts.min(100),
                    edge: no_bid_edge_net,
                }));
            }
        }

        // NO ask (to sell NO = reduce short YES)
        let no_ask_price = fair_prob_no + half_spread + skew;
        let no_ask_edge_raw = no_ask_price - fair_prob_no;
        let no_ask_maker_fee = self.calculate_trade_fee(no_ask_price, 1, false);
        let no_ask_edge_net = no_ask_edge_raw - no_ask_maker_fee;

        if no_ask_price > 0.02
            && no_ask_price < 0.98
            && no_ask_edge_net > self.config.min_edge_to_quote
            && self.position.yes_position < self.config.max_inventory
        {
            if self.position.yes_position < 0 {
                // Have NO position to sell
                let contracts = self.position.yes_position.abs().min(100);
                signals.push(MarketMakerSignal::Quote(QuoteOrder {
                    side: OrderSide::No,
                    is_buy: false,
                    price_cents: (no_ask_price * 100.0).round() as i64,
                    contracts,
                    edge: no_ask_edge_net,
                }));
            }
        }

        if signals.is_empty() {
            signals.push(MarketMakerSignal::Hold {
                reason: "No profitable opportunities".to_string(),
            });
        }

        signals
    }

    /// Process a fill notification
    pub fn on_fill(&mut self, side: OrderSide, is_buy: bool, contracts: i64, price_cents: i64) {
        let price = price_cents as f64 / 100.0;
        self.position.update_from_fill(side, is_buy, contracts, price);

        info!(
            "[MM] FILL: {:?} {:?} {} @ {}¢ | new_pos={} | max_loss=${:.2}",
            if is_buy { "BUY" } else { "SELL" },
            side,
            contracts,
            price_cents,
            self.position.yes_position,
            self.position.max_loss()
        );
    }

    /// Get current P&L summary
    pub fn pnl_summary(&self) -> PnLSummary {
        let unrealized = self
            .fair_value_calc
            .fair_prob()
            .map(|fp| self.position.unrealized_pnl(fp))
            .unwrap_or(0.0);

        PnLSummary {
            realized_pnl: self.position.realized_pnl,
            unrealized_pnl: unrealized,
            total_pnl: self.position.realized_pnl + unrealized,
            max_loss: self.position.max_loss(),
            position: self.position.yes_position,
        }
    }
}

/// P&L summary
#[derive(Debug, Clone)]
pub struct PnLSummary {
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    pub total_pnl: f64,
    pub max_loss: f64,
    pub position: i64,
}

// =============================================================================
// SIGNAL EXECUTOR
// =============================================================================

/// Execute market maker signals via the Kalshi API
pub struct SignalExecutor {
    client: KalshiClient,
    ticker: String,
}

impl SignalExecutor {
    pub fn new(client: KalshiClient, ticker: String) -> Self {
        Self { client, ticker }
    }

    /// Execute a single signal
    pub async fn execute(&self, signal: &MarketMakerSignal) -> Result<Option<Order>> {
        match signal {
            MarketMakerSignal::Quote(quote) => {
                info!(
                    "[EXEC] Posting quote: {:?} {:?} {} @ {}¢",
                    if quote.is_buy { "BUY" } else { "SELL" },
                    quote.side,
                    quote.contracts,
                    quote.price_cents
                );

                let order = if quote.is_buy {
                    self.client
                        .buy_limit(&self.ticker, quote.side, quote.price_cents, quote.contracts)
                        .await?
                } else {
                    self.client
                        .sell_limit(&self.ticker, quote.side, quote.price_cents, quote.contracts)
                        .await?
                };

                Ok(Some(order))
            }

            MarketMakerSignal::AggressiveTake(take) => {
                info!(
                    "[EXEC] Aggressive take: {:?} {:?} {} @ {}¢",
                    if take.is_buy { "BUY" } else { "SELL" },
                    take.side,
                    take.contracts,
                    take.price_cents
                );

                let order = if take.is_buy {
                    self.client
                        .buy_ioc(&self.ticker, take.side, take.price_cents, take.contracts)
                        .await?
                } else {
                    self.client
                        .sell_ioc(&self.ticker, take.side, take.price_cents, take.contracts)
                        .await?
                };

                Ok(Some(order))
            }

            MarketMakerSignal::AmendOrder {
                order_id,
                new_price,
                new_count,
            } => {
                info!(
                    "[EXEC] Amending order {}: price={}¢, count={}",
                    order_id, new_price, new_count
                );

                let request = crate::types::AmendOrderRequest {
                    price: Some(*new_price),
                    count: Some(*new_count),
                };

                let order = self.client.amend_order(order_id, request).await?;
                Ok(Some(order))
            }

            MarketMakerSignal::CancelOrder { order_id } => {
                info!("[EXEC] Cancelling order {}", order_id);
                self.client.cancel_order(order_id).await?;
                Ok(None)
            }

            MarketMakerSignal::CancelAll { reason } => {
                warn!("[EXEC] Cancelling all orders: {}", reason);
                self.client.cancel_all_orders().await?;
                Ok(None)
            }

            MarketMakerSignal::Hold { reason } => {
                debug!("[EXEC] Hold: {}", reason);
                Ok(None)
            }
        }
    }

    /// Execute multiple signals
    pub async fn execute_all(&self, signals: &[MarketMakerSignal]) -> Vec<Result<Option<Order>>> {
        let mut results = Vec::new();
        for signal in signals {
            results.push(self.execute(signal).await);
        }
        results
    }
}

// =============================================================================
// MARKET MAKER RUNNER
// =============================================================================

use tokio::sync::mpsc;
use crate::types::{CalculatorStateSnapshot, FillUpdate, OrderAction};

/// Run the market maker task
///
/// Receives state snapshots from Calculator and fill updates from fill monitor.
/// Generates trading signals and executes them via the Kalshi API.
///
/// # Arguments
/// * `client` - Kalshi API client for order execution
/// * `fair_value_calc` - Fair value calculator with market spec
/// * `config` - Market maker configuration
/// * `state_rx` - Channel receiving state snapshots from Calculator
/// * `fill_rx` - Channel receiving fill updates from fill monitor
pub async fn run(
    client: KalshiClient,
    fair_value_calc: FairValueCalculator,
    config: MarketMakerConfig,
    mut state_rx: mpsc::Receiver<CalculatorStateSnapshot>,
    mut fill_rx: mpsc::Receiver<FillUpdate>,
) {
    let ticker = fair_value_calc.market_spec.ticker.clone();
    let mut mm = MarketMaker::new(ticker.clone(), fair_value_calc, config);
    let executor = SignalExecutor::new(client, ticker.clone());

    info!("[MM] Started for {} | max_loss=${:.0} | conf={:.0}%",
        ticker,
        mm.config.max_loss_per_market,
        mm.config.fair_value_confidence * 100.0
    );

    loop {
        tokio::select! {
            // Process state updates from Calculator
            Some(snapshot) = state_rx.recv() => {
                // Generate signals based on current state
                let signals = mm.generate_signals(
                    snapshot.btc_mid_price,
                    snapshot.yes_bid,
                    snapshot.no_bid,
                );

                // Execute signals
                for signal in &signals {
                    match executor.execute(signal).await {
                        Ok(Some(order)) => {
                            info!("[MM] Order placed: {} | status={:?}",
                                order.order_id, order.status);
                            mm.active_orders.insert(order.order_id.clone(), order);
                        }
                        Ok(None) => {
                            // No order placed (cancel, hold, etc.)
                        }
                        Err(e) => {
                            warn!("[MM] Order execution failed: {}", e);
                        }
                    }
                }
            }

            // Process fill updates from fill monitor
            Some(fill) = fill_rx.recv() => {
                // Update position from fill
                let is_buy = matches!(fill.action, OrderAction::Buy);
                mm.on_fill(fill.side, is_buy, fill.count, fill.price_cents);

                // Remove from active orders if fully filled
                mm.active_orders.remove(&fill.order_id);

                // Log P&L summary
                let pnl = mm.pnl_summary();
                info!("[MM] P&L: realized=${:.2} | unrealized=${:.2} | total=${:.2} | pos={}",
                    pnl.realized_pnl,
                    pnl.unrealized_pnl,
                    pnl.total_pnl,
                    pnl.position
                );
            }

            // Both channels closed - exit
            else => {
                info!("[MM] Channels closed, shutting down for {}", ticker);
                break;
            }
        }
    }

    // Final P&L summary
    let pnl = mm.pnl_summary();
    info!("[MM] Final P&L for {}: realized=${:.2} | total=${:.2} | final_pos={}",
        ticker, pnl.realized_pnl, pnl.total_pnl, pnl.position);
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_max_loss_long() {
        let mut pos = PositionState::new();
        pos.yes_position = 100;
        pos.cost_basis = 50.0; // Paid $50 for 100 contracts at 50c

        // Max loss = what we paid
        assert_eq!(pos.max_loss(), 50.0);
    }

    #[test]
    fn test_position_max_loss_short() {
        let mut pos = PositionState::new();
        pos.yes_position = -100; // Short 100 YES
        pos.cost_basis = -30.0; // Received $30 premium (selling at 30c)

        // Max loss = contracts * $1 - premium received = 100 - 30 = 70
        assert!((pos.max_loss() - 70.0).abs() < 0.01);
    }

    #[test]
    fn test_edge_calculation() {
        // No maker fees for this test
        let edge = EdgeCalculation::calculate(0.60, 55, 42, false, ProductType::Standard);

        assert!((edge.fair_prob - 0.60).abs() < 0.001);
        assert!((edge.market_yes_bid - 0.55).abs() < 0.001);
        assert!((edge.market_no_bid - 0.42).abs() < 0.001);

        // YES ask = 1 - NO bid = 1 - 0.42 = 0.58
        assert!((edge.market_yes_ask - 0.58).abs() < 0.001);

        // YES buy raw edge = fair - ask = 0.60 - 0.58 = 0.02
        assert!((edge.yes_buy_edge_raw - 0.02).abs() < 0.001);

        // YES sell raw edge = bid - fair = 0.55 - 0.60 = -0.05
        assert!((edge.yes_sell_edge_raw - (-0.05)).abs() < 0.001);

        // Net edge should be raw edge minus taker fee
        // Taker fee at 0.58: 0.58 * 0.42 * 0.07 = 0.017052
        let expected_taker_fee = 0.58 * 0.42 * 0.07;
        let expected_net = 0.02 - expected_taker_fee;
        // Note: fee is rounded up to cents, so actual fee is 0.02
        assert!(edge.yes_buy_edge_net < edge.yes_buy_edge_raw);
        assert!(edge.yes_buy_edge_net < 0.01); // Should be small or negative after 7% taker fee
    }

    #[test]
    fn test_max_contracts_to_add() {
        let pos = PositionState::new();

        // With $100 max loss budget, buying at 50c = max 200 contracts
        let max = pos.max_contracts_to_add(0.50, 100.0, OrderSide::Yes, true);
        assert_eq!(max, 200);

        // Buying at 25c = max 400 contracts
        let max = pos.max_contracts_to_add(0.25, 100.0, OrderSide::Yes, true);
        assert_eq!(max, 400);
    }

    #[test]
    fn test_market_mid_calculation() {
        // yes_bid = 55, no_bid = 42
        // yes_ask = 1 - 0.42 = 0.58
        // mid = (0.55 + 0.58) / 2 = 0.565
        let mid = MarketMaker::calculate_market_mid(55, 42);
        assert!((mid - 0.565).abs() < 0.001);

        // Symmetric case: yes_bid = 50, no_bid = 50
        // yes_ask = 1 - 0.50 = 0.50
        // mid = (0.50 + 0.50) / 2 = 0.50
        let mid = MarketMaker::calculate_market_mid(50, 50);
        assert!((mid - 0.50).abs() < 0.001);
    }

    #[test]
    fn test_fair_value_blending() {
        use crate::fair_value::{FairValueCalculator, MarketType, BtcMarketSpec};
        use chrono::{Utc, Duration};

        // Create a simple market spec
        let expiry = Utc::now() + Duration::hours(24);
        let market_spec = BtcMarketSpec {
            ticker: "TEST".to_string(),
            strike: 100000.0,
            expiry,
            market_type: MarketType::Above,
        };
        let fair_calc = FairValueCalculator::new(market_spec);

        // Test full confidence (100% model)
        let mut config = MarketMakerConfig::default();
        config.fair_value_confidence = 1.0;
        let mut mm = MarketMaker::new("TEST".to_string(), fair_calc.clone(), config);

        let model_fair = 0.60;
        let market_mid = 0.50;
        let blended = mm.blend_fair_value(model_fair, market_mid);
        assert!((blended - 0.60).abs() < 0.001, "100% confidence should use model");

        // Test zero confidence (100% market)
        let mut config = MarketMakerConfig::default();
        config.fair_value_confidence = 0.0;
        let mut mm = MarketMaker::new("TEST".to_string(), fair_calc.clone(), config);

        let blended = mm.blend_fair_value(model_fair, market_mid);
        assert!((blended - 0.50).abs() < 0.001, "0% confidence should use market");

        // Test 50% confidence (50/50 blend)
        let mut config = MarketMakerConfig::default();
        config.fair_value_confidence = 0.5;
        let mut mm = MarketMaker::new("TEST".to_string(), fair_calc.clone(), config);

        let blended = mm.blend_fair_value(model_fair, market_mid);
        let expected = 0.5 * 0.60 + 0.5 * 0.50; // 0.55
        assert!((blended - expected).abs() < 0.001, "50% confidence should be 50/50 blend");

        // Test 70% confidence
        let mut config = MarketMakerConfig::default();
        config.fair_value_confidence = 0.7;
        let mut mm = MarketMaker::new("TEST".to_string(), fair_calc, config);

        let blended = mm.blend_fair_value(model_fair, market_mid);
        let expected = 0.7 * 0.60 + 0.3 * 0.50; // 0.57
        assert!((blended - expected).abs() < 0.001, "70% confidence should blend correctly");
    }
}
