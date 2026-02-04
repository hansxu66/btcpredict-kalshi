//! Kalshi & Multi-Exchange BTC Real-Time Monitor + Market Maker
//!
//! Streams probability changes from Kalshi and BTC price from multiple exchanges.
//! Publishes updates to Redis for dashboard consumption.
//! Optionally runs a market maker to trade based on fair value signals.
//! Tickers are loaded from `src/tickers.csv`.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use ::kalshi_monitor::auth::KalshiAuth;
use ::kalshi_monitor::calculator::{self, ProductType};
use ::kalshi_monitor::fair_value::FairValueCalculator;
use ::kalshi_monitor::market_maker::{self, MarketMakerConfig};
use ::kalshi_monitor::redis_client::RedisClient;
use ::kalshi_monitor::trading_apis::KalshiClient;
use ::kalshi_monitor::types::{CalculatorStateSnapshot, FillUpdate, MonitorUpdate, TradingEnvironment};
use ::kalshi_monitor::websockets::{
    crypto_aggregator::{self, AggregatorConfig},
    kalshi_fills,
    kalshi_monitor,
};

// =============================================================================
// CONFIGURATION
// =============================================================================

/// Set to `true` for Demo environment, `false` for Production (Kalshi only)
const USE_DEMO: bool = false;

/// Path to the tickers CSV file
const TICKERS_CSV_PATH: &str = "src/tickers.csv";

/// Reconnect delay after disconnection (seconds)
const RECONNECT_DELAY_SECS: u64 = 5;

/// Enable Kalshi monitor
const ENABLE_KALSHI: bool = true;

/// Enable Binance BTC price monitor
const ENABLE_BINANCE: bool = true;

/// Enable Coinbase BTC price monitor
const ENABLE_COINBASE: bool = true;

/// Enable Kraken BTC price monitor
const ENABLE_KRAKEN: bool = true;

/// Enable Crypto.com BTC price monitor
const ENABLE_CRYPTOCOM: bool = true;

/// Enable Redis publishing (set to false to run without Redis)
const ENABLE_REDIS: bool = true;

/// Enable market maker trading (DANGER: real money if USE_DEMO=false)
/// When false, system runs in monitor-only mode
const ENABLE_TRADING: bool = false;

// =============================================================================
// MARKET MAKER CONFIGURATION
// =============================================================================

/// Maximum loss allowed per market (in dollars)
const MM_MAX_LOSS_PER_MARKET: f64 = 100.0;

/// Base spread width (in probability points, e.g., 0.03 = 3 cents)
const MM_BASE_SPREAD: f64 = 0.03;

/// Minimum edge required to post a quote (after fees)
const MM_MIN_EDGE_TO_QUOTE: f64 = 0.005;

/// Edge threshold for aggressive taking (after fees)
const MM_AGGRESSIVE_TAKE_THRESHOLD: f64 = 0.03;

/// Maximum inventory (contracts) per market
const MM_MAX_INVENTORY: i64 = 500;

/// Confidence in fair value model (0.0 = pure market making, 1.0 = full model trust)
const MM_FAIR_VALUE_CONFIDENCE: f64 = 0.5;

// =============================================================================
// TICKER CONFIG FROM CSV
// =============================================================================

/// A row from the tickers CSV file - each row represents a Kalshi market to monitor
#[derive(Debug, Clone, Deserialize)]
pub struct TickerConfig {
    /// Kalshi market ticker
    pub market_ticker_kalshi: String,
    /// Kalshi event ticker (optional)
    #[serde(default)]
    pub event_ticker_kalshi: String,
    /// Kalshi series ticker (optional)
    #[serde(default)]
    pub series_ticker_kalshi: String,
}

/// Load ticker configurations from CSV file
fn load_tickers_from_csv<P: AsRef<Path>>(path: P) -> Result<Vec<TickerConfig>> {
    let path = path.as_ref();
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("Failed to open tickers CSV: {}", path.display()))?;

    let mut tickers = Vec::new();
    for result in reader.deserialize() {
        let record: TickerConfig = result.context("Failed to parse ticker row")?;
        tickers.push(record);
    }

    Ok(tickers)
}

// =============================================================================
// MAIN
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("kalshi_monitor=info".parse().unwrap())
                .add_directive("tokio_tungstenite=warn".parse().unwrap()),
        )
        .with_target(false)
        .init();

    // Load tickers from CSV
    let tickers = load_tickers_from_csv(TICKERS_CSV_PATH)?;

    // Select Kalshi environment based on flag
    let kalshi_env = if USE_DEMO {
        TradingEnvironment::Demo
    } else {
        TradingEnvironment::Production
    };

    info!("========================================");
    if ENABLE_TRADING {
        info!("  BTC Market Maker (TRADING ENABLED)");
    } else {
        info!("  Multi-Exchange BTC Price Monitor");
    }
    info!("========================================");
    info!("  Loaded {} ticker pairs from CSV", tickers.len());

    for ticker in &tickers {
        info!("  Kalshi: {}", ticker.market_ticker_kalshi);
    }

    if ENABLE_KALSHI {
        info!("[KALSHI] Environment: {}", kalshi_env);
    }
    if ENABLE_BINANCE {
        info!("[BINANCE] Enabled (BTCUSDT)");
    }
    if ENABLE_COINBASE {
        info!("[COINBASE] Enabled (BTC-USD)");
    }
    if ENABLE_KRAKEN {
        info!("[KRAKEN] Enabled (XBT/USD)");
    }
    if ENABLE_CRYPTOCOM {
        info!("[CRYPTO.COM] Enabled (BTC_USDT)");
    }
    if ENABLE_REDIS {
        info!("[REDIS] Publishing enabled");
    }
    if ENABLE_TRADING {
        info!("[TRADING] Market Maker ENABLED");
        info!("[TRADING] Max loss: ${:.0} | Confidence: {:.0}%",
            MM_MAX_LOSS_PER_MARKET, MM_FAIR_VALUE_CONFIDENCE * 100.0);
        if !USE_DEMO {
            warn!("[TRADING] *** PRODUCTION MODE - REAL MONEY ***");
        }
    } else {
        info!("[TRADING] Monitor-only mode (no trading)");
    }

    info!("========================================");

    // Validate we have tickers
    if tickers.is_empty() {
        warn!("No tickers found in CSV!");
        warn!("Ensure {} exists and contains valid tickers.", TICKERS_CSV_PATH);
        return Ok(());
    }

    // Connect to Redis (if enabled)
    let redis: Option<Arc<RedisClient>> = if ENABLE_REDIS {
        match RedisClient::from_env().await {
            Ok(client) => {
                info!("[REDIS] Connected");
                Some(Arc::new(client))
            }
            Err(e) => {
                warn!("[REDIS] Failed to connect: {}. Continuing without Redis.", e);
                None
            }
        }
    } else {
        None
    };

    // Load Kalshi credentials
    let kalshi_auth = if ENABLE_KALSHI {
        let auth = KalshiAuth::from_env(kalshi_env)
            .context("Failed to load Kalshi credentials")?;
        info!(
            "[KALSHI] Loaded credentials: {}...",
            &auth.api_key_id[..8.min(auth.api_key_id.len())]
        );

        let client = KalshiClient::new(auth.clone(), kalshi_env);
        let balance = client.get_balance().await
            .context("Failed to connect to Kalshi API")?;
        info!(
            "[KALSHI] Connected! Balance: ${:.2}",
            balance.balance as f64 / 100.0
        );
        Some(auth)
    } else {
        None
    };

    // Configure centralized crypto aggregator
    let aggregator_config = AggregatorConfig {
        enable_binance: ENABLE_BINANCE,
        enable_coinbase: ENABLE_COINBASE,
        enable_kraken: ENABLE_KRAKEN,
        enable_cryptocom: ENABLE_CRYPTOCOM,
        reconnect_delay_secs: RECONNECT_DELAY_SECS,
    };

    // Spawn tasks per ticker pair
    let mut handles = vec![];

    for ticker_config in &tickers {
        let ticker_id = ticker_config.market_ticker_kalshi.clone();

        // Create channel for this ticker pair (monitor -> calculator)
        let (sender, receiver) = mpsc::channel::<MonitorUpdate>(100);

        // --- Create Market Maker channels (if trading enabled) ---
        let (state_sender, mm_state_receiver) = if ENABLE_TRADING {
            let (tx, rx) = mpsc::channel::<CalculatorStateSnapshot>(100);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let (fill_sender, mm_fill_receiver) = if ENABLE_TRADING {
            let (tx, rx) = mpsc::channel::<FillUpdate>(100);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        // --- Spawn Calculator ---
        let calc_ticker_id = ticker_id.clone();
        let calc_redis = redis.clone();
        let calc_handle = tokio::spawn(async move {
            calculator::run(
                calc_ticker_id,
                receiver,
                None,
                calc_redis,
                state_sender,
                MM_FAIR_VALUE_CONFIDENCE,
            ).await;
        });
        handles.push(calc_handle);

        // --- Spawn Kalshi Monitor ---
        if let Some(ref auth) = kalshi_auth {
            let market_ticker = ticker_config.market_ticker_kalshi.clone();
            let auth = auth.clone();
            let sender = sender.clone();

            let kalshi_handle = tokio::spawn(async move {
                loop {
                    match kalshi_monitor::run(&auth, kalshi_env, &market_ticker, sender.clone()).await {
                        Ok(()) => info!("[KALSHI] WebSocket closed for {}", market_ticker),
                        Err(e) => error!("[KALSHI] WebSocket error for {}: {}", market_ticker, e),
                    }
                    info!("[KALSHI] Reconnecting {} in {}s...", market_ticker, RECONNECT_DELAY_SECS);
                    tokio::time::sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
                }
            });
            handles.push(kalshi_handle);
        }

        // --- Spawn Fill Monitor (if trading enabled) ---
        if ENABLE_TRADING {
            if let (Some(ref auth), Some(fill_tx)) = (&kalshi_auth, fill_sender) {
                let market_ticker = ticker_config.market_ticker_kalshi.clone();
                let auth = auth.clone();

                let fill_handle = tokio::spawn(async move {
                    loop {
                        match kalshi_fills::run(&auth, kalshi_env, Some(&market_ticker), fill_tx.clone()).await {
                            Ok(()) => info!("[FILLS] WebSocket closed for {}", market_ticker),
                            Err(e) => error!("[FILLS] WebSocket error for {}: {}", market_ticker, e),
                        }
                        info!("[FILLS] Reconnecting {} in {}s...", market_ticker, RECONNECT_DELAY_SECS);
                        tokio::time::sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
                    }
                });
                handles.push(fill_handle);
            }
        }

        // --- Spawn Market Maker (if trading enabled) ---
        if ENABLE_TRADING {
            if let (Some(ref auth), Some(state_rx), Some(fill_rx)) =
                (&kalshi_auth, mm_state_receiver, mm_fill_receiver)
            {
                let market_ticker = ticker_config.market_ticker_kalshi.clone();
                let client = KalshiClient::new(auth.clone(), kalshi_env);

                // Try to parse fair value calculator from ticker
                let fair_value_calc = match FairValueCalculator::from_ticker(&market_ticker) {
                    Some(fv) => fv,
                    None => {
                        warn!("[MM] Could not parse market spec from ticker: {}", market_ticker);
                        warn!("[MM] Skipping market maker for this ticker");
                        continue;
                    }
                };

                // Configure market maker
                let mm_config = MarketMakerConfig {
                    max_loss_per_market: MM_MAX_LOSS_PER_MARKET,
                    base_spread: MM_BASE_SPREAD,
                    min_edge_to_quote: MM_MIN_EDGE_TO_QUOTE,
                    aggressive_take_threshold: MM_AGGRESSIVE_TAKE_THRESHOLD,
                    max_inventory: MM_MAX_INVENTORY,
                    fair_value_confidence: MM_FAIR_VALUE_CONFIDENCE,
                    maker_fee_market: false,
                    product_type: ProductType::Standard,
                    ..Default::default()
                };

                let mm_handle = tokio::spawn(async move {
                    market_maker::run(
                        client,
                        fair_value_calc,
                        mm_config,
                        state_rx,
                        fill_rx,
                    ).await;
                });
                handles.push(mm_handle);

                info!("[MAIN] Market Maker spawned for: {}", market_ticker);
            }
        }

        // --- Spawn Centralized Crypto Aggregator ---
        // This single aggregator handles all exchanges (Binance, Coinbase, Kraken, Crypto.com)
        // and emits mean midprice updates to the calculator
        let agg_config = aggregator_config.clone();
        let agg_sender = sender.clone();
        let agg_ticker_id = ticker_id.clone();

        let aggregator_handle = tokio::spawn(async move {
            info!("[AGGREGATOR] Starting centralized crypto price monitor for {}", agg_ticker_id);
            if let Err(e) = crypto_aggregator::run(agg_config, agg_sender).await {
                error!("[AGGREGATOR] Fatal error: {}", e);
            }
        });
        handles.push(aggregator_handle);

        info!("[MAIN] Spawned tasks for: {}", ticker_id);
    }

    // Wait for all tasks
    if handles.is_empty() {
        warn!("No tasks spawned! Check ENABLE_* settings.");
        return Ok(());
    }

    let enabled_count = [ENABLE_BINANCE, ENABLE_COINBASE, ENABLE_KRAKEN, ENABLE_CRYPTOCOM]
        .iter()
        .filter(|&&x| x)
        .count();
    info!(
        "[MAIN] All tasks running (aggregator with {} exchanges{})",
        enabled_count,
        if ENABLE_KALSHI { " + Kalshi" } else { "" }
    );

    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}
