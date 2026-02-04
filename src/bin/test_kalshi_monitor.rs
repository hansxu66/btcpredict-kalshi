//! Kalshi WebSocket monitor test with Redis publishing.
//!
//! Connects to Kalshi WebSocket, streams orderbook updates, and publishes to Redis.
//!
//! Usage:
//!   cargo run --bin test_kalshi_monitor
//!   cargo run --bin test_kalshi_monitor -- --market KXBTC-25JAN15-B100000
//!   cargo run --bin test_kalshi_monitor -- --demo
//!
//! Environment:
//!   KALSHI_PROD_API_KEY_ID / KALSHI_DEMO_API_KEY_ID
//!   KALSHI_PROD_PRIVATE_KEY_PATH / KALSHI_DEMO_PRIVATE_KEY_PATH
//!   REDIS_URL - Redis connection URL (default: redis://127.0.0.1:6379)

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tracing::info;

use ::kalshi_monitor::auth::KalshiAuth;
use ::kalshi_monitor::redis_client::RedisClient;
use ::kalshi_monitor::types::{MonitorUpdate, TradingEnvironment};
use ::kalshi_monitor::websockets::kalshi_monitor;

#[derive(Parser, Debug)]
#[command(name = "test_kalshi_monitor")]
#[command(about = "Monitor Kalshi orderbook and publish to Redis")]
struct Args {
    /// Market ticker to monitor
    #[arg(long, default_value = "KXNBAGAME-26JAN15CHALAL-LAL")]
    market: String,

    /// Use demo environment (default: production)
    #[arg(long)]
    demo: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("kalshi_monitor=info".parse().unwrap())
                .add_directive("tokio_tungstenite=warn".parse().unwrap()),
        )
        .with_target(false)
        .init();

    let args = Args::parse();
    let env = if args.demo {
        TradingEnvironment::Demo
    } else {
        TradingEnvironment::Production
    };

    info!("========================================");
    info!("  Kalshi Monitor (Redis)");
    info!("========================================");
    info!("  Environment: {}", env);
    info!("  Market:      {}", args.market);
    info!("  Output:      Redis pub/sub");
    info!("========================================");

    // Connect to Redis (verify connection works)
    let _redis = RedisClient::from_env()
        .await
        .context("Failed to connect to Redis")?;
    info!("Connected to Redis");

    // Load credentials
    let auth = KalshiAuth::from_env(env).context("Failed to load Kalshi credentials")?;
    info!("Loaded credentials: {}...", &auth.api_key_id[..8.min(auth.api_key_id.len())]);

    // Create channel
    let (ws_sender, mut receiver) = mpsc::channel::<MonitorUpdate>(100);

    // Event counter task
    let counter_task = tokio::spawn(async move {
        let mut count = 0u64;
        while let Some(update) = receiver.recv().await {
            if let MonitorUpdate::Kalshi(p) = update {
                count += 1;
                if count % 10 == 0 {
                    info!(
                        "[COUNTER] {} updates received (YES: {:.1}%, NO: {:.1}%)",
                        count,
                        p.yes_prob * 100.0,
                        p.no_prob * 100.0
                    );
                }
            }
        }
        count
    });

    // Run monitor (Redis publishing now handled by calculator)
    info!("Connecting to Kalshi...");
    if let Err(e) = kalshi_monitor::run(&auth, env, &args.market, ws_sender).await {
        info!("Monitor error: {}", e);
    }

    let count = counter_task.await.unwrap_or(0);
    info!("Done. {} updates published to Redis", count);
    Ok(())
}
