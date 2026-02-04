//! Test binary for Kalshi Trading API.
//!
//! Run with: cargo run --bin test_trading_api
//!
//! Tests the Kalshi REST API by:
//! 1. Getting account balance
//! 2. Listing open orders
//! 3. Placing a test limit order (low price, won't fill)
//! 4. Checking the order status
//! 5. Cancelling the order

use anyhow::{Context, Result};
use tracing::info;

// Import from main crate
use ::kalshi_monitor::auth::KalshiAuth;
use ::kalshi_monitor::trading_apis::KalshiClient;
use ::kalshi_monitor::types::kalshi::{CreateOrderRequest, OrderAction, OrderSide, OrderType};
use ::kalshi_monitor::types::TradingEnvironment;

/// Market ticker to test with
const TEST_MARKET_TICKER: &str = "KXATPMATCH-26JAN14SPIMAR-SPI";

/// Use Demo environment (IMPORTANT: keep this true for testing!)
const USE_DEMO: bool = false;

/// Test order price (very low so it won't fill)
const TEST_ORDER_PRICE_CENTS: i64 = 1;

/// Test order quantity
const TEST_ORDER_COUNT: i64 = 1;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("kalshi_monitor=debug".parse().unwrap())
                .add_directive("test_trading_api=info".parse().unwrap()),
        )
        .with_target(false)
        .init();

    let env = if USE_DEMO {
        TradingEnvironment::Demo
    } else {
        TradingEnvironment::Production
    };

    info!("========================================");
    info!("  Kalshi Trading API Test");
    info!("========================================");
    info!("  Environment: {}", env);
    info!("  Market:      {}", TEST_MARKET_TICKER);
    info!("========================================");


    // Load credentials
    let auth = KalshiAuth::from_env(env)
        .context("Failed to load Kalshi credentials")?;
    info!(
        "Loaded credentials: {}...",
        &auth.api_key_id[..8.min(auth.api_key_id.len())]
    );

    // Create client
    let client = KalshiClient::new(auth, env);

    // --- Test 1: Get Balance ---
    info!("\n[TEST 1] Getting account balance...");
    let balance = client.get_balance().await
        .context("Failed to get balance")?;
    info!(
        "  Balance: ${:.2}",
        balance.balance as f64 / 100.0
    );

    // --- Test 2: List Open Orders ---
    info!("\n[TEST 2] Listing open orders...");
    let orders = client.get_orders(Some(TEST_MARKET_TICKER)).await
        .context("Failed to get orders")?;
    info!("  Found {} open orders for {}", orders.len(), TEST_MARKET_TICKER);
    for order in &orders {
        info!(
            "    - {} {} @ {:?}¢ (status: {:?})",
            order.action,
            order.side,
            order.yes_price.or(order.no_price),
            order.status
        );
    }

    // --- Test 3: Place Test Order ---
    info!("\n[TEST 3] Placing test order...");
    info!(
        "  Order: BUY YES @ {}¢ x {} on {}",
        TEST_ORDER_PRICE_CENTS, TEST_ORDER_COUNT, TEST_MARKET_TICKER
    );

    let order_request = CreateOrderRequest {
        ticker: TEST_MARKET_TICKER.to_string(),
        action: OrderAction::Buy,
        side: OrderSide::Yes,
        order_type: OrderType::Limit,
        count: TEST_ORDER_COUNT,
        yes_price: Some(TEST_ORDER_PRICE_CENTS),
        no_price: None,
        client_order_id: CreateOrderRequest::generate_client_order_id(),
        time_in_force: None,
        expiration_ts: None,
    };

    let order = client.create_order(order_request).await
        .context("Failed to create order")?;
    info!("  Order created!");
    info!("    ID: {}", order.order_id);
    info!("    Status: {:?}", order.status);

    // --- Test 4: Get Order Status ---
    info!("\n[TEST 4] Checking order status...");
    let order_status = client.get_order(&order.order_id).await
        .context("Failed to get order")?;
    info!("    Status: {:?}", order_status.status);
    info!("    Remaining: {:?}", order_status.remaining_count);

    // --- Test 5: Cancel Order ---
    info!("\n[TEST 5] Cancelling order...");
    client.cancel_order(&order.order_id).await
        .context("Failed to cancel order")?;
    info!("  Order cancelled!");

    // Verify cancellation
    let orders_after = client.get_orders(Some(TEST_MARKET_TICKER)).await
        .context("Failed to get orders")?;
    info!(
        "  Orders remaining for {}: {}",
        TEST_MARKET_TICKER,
        orders_after.len()
    );

    info!("\n========================================");
    info!("  All tests passed!");
    info!("========================================");

    Ok(())
}
