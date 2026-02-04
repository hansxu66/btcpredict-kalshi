//! Type definitions for all APIs.

pub mod binance;
pub mod coinbase;
pub mod crypto_aggregator;
pub mod cryptocom;
pub mod kalshi;
pub mod kraken;
pub mod messages;

// Re-export common Kalshi types
pub use kalshi::{
    // Environment
    TradingEnvironment,
    // WebSocket
    OrderbookState, ProbabilityUpdate, SubscribeCmd, SubscribeParams, WsMessage, WsMessageBody,
    // Orders
    AmendOrderRequest, Balance, BalanceResponse, CreateOrderRequest, Order, OrderAction,
    OrderResponse, OrderSide, OrderStatus, OrderType, OrdersResponse, Position,
    PositionsResponse, TimeInForce,
};

// Re-export aggregator types
pub use crypto_aggregator::{AggregatedPriceUpdate, CryptoAggregatorEvent, Exchange};

// Re-export message types
pub use messages::{
    CalculatorStateSnapshot, FillUpdate, MarketConfig, MonitorUpdate,
};
