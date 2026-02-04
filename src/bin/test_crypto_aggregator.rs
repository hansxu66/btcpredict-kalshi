//! Aggregated Crypto Price Monitor test.
//!
//! Connects to multiple crypto exchanges (Binance, Coinbase, Kraken, Crypto.com)
//! and prints aggregated BTC midprice updates.
//!
//! Usage:
//!   cargo run --bin test_crypto_aggregator
//!   cargo run --bin test_crypto_aggregator -- --disable-binance
//!   cargo run --bin test_crypto_aggregator -- --only-binance --only-coinbase

use anyhow::Result;
use clap::Parser;
use tokio::sync::mpsc;
use tracing::info;

use kalshi_monitor::types::{CryptoAggregatorEvent, MonitorUpdate};
use kalshi_monitor::websockets::crypto_aggregator::{self, AggregatorConfig};

#[derive(Parser, Debug)]
#[command(name = "test_crypto_aggregator")]
#[command(about = "Test aggregated crypto price monitor")]
struct Args {
    /// Disable Binance
    #[arg(long)]
    disable_binance: bool,

    /// Disable Coinbase
    #[arg(long)]
    disable_coinbase: bool,

    /// Disable Kraken
    #[arg(long)]
    disable_kraken: bool,

    /// Disable Crypto.com
    #[arg(long)]
    disable_cryptocom: bool,
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

    let config = AggregatorConfig {
        enable_binance: !args.disable_binance,
        enable_coinbase: !args.disable_coinbase,
        enable_kraken: !args.disable_kraken,
        enable_cryptocom: !args.disable_cryptocom,
        reconnect_delay_secs: 5,
    };

    let enabled: Vec<&str> = [
        (config.enable_binance, "Binance"),
        (config.enable_coinbase, "Coinbase"),
        (config.enable_kraken, "Kraken"),
        (config.enable_cryptocom, "Crypto.com"),
    ]
    .iter()
    .filter(|(enabled, _)| *enabled)
    .map(|(_, name)| *name)
    .collect();

    info!("========================================");
    info!("  Crypto Aggregator Test");
    info!("========================================");
    info!("  Exchanges: {}", enabled.join(", "));
    info!("  Pairs: BTCUSDT / BTC-USD / XBT/USD");
    info!("========================================");

    if enabled.is_empty() {
        info!("No exchanges enabled! Use --help for options.");
        return Ok(());
    }

    // Create channel
    let (sender, mut receiver) = mpsc::channel::<MonitorUpdate>(100);

    // Receiver task - prints updates
    let printer_task = tokio::spawn(async move {
        let mut update_count = 0u64;
        let mut connected_exchanges = 0u32;

        while let Some(update) = receiver.recv().await {
            if let MonitorUpdate::Crypto(event) = update {
                match event {
                    CryptoAggregatorEvent::ExchangeConnected(ex) => {
                        connected_exchanges += 1;
                        info!("[CONNECTED] {} ({} total)", ex, connected_exchanges);
                    }
                    CryptoAggregatorEvent::ExchangeDisconnected(ex) => {
                        connected_exchanges = connected_exchanges.saturating_sub(1);
                        info!("[DISCONNECTED] {} ({} remaining)", ex, connected_exchanges);
                    }
                    CryptoAggregatorEvent::PriceUpdate(price) => {
                        update_count += 1;

                        // Build individual prices string
                        let mut prices_str = String::new();
                        for (ex, mid) in &price.exchange_prices {
                            if !prices_str.is_empty() {
                                prices_str.push_str(" | ");
                            }
                            prices_str.push_str(&format!("{}=${:.2}", ex, mid));
                        }

                        info!(
                            "[{:>6}] MEAN=${:.2} | {} exchanges | {}",
                            update_count,
                            price.mean_mid_price,
                            price.exchange_count,
                            prices_str
                        );
                    }
                }
            }
        }
        update_count
    });

    // Run aggregator
    info!("Starting aggregator...");
    if let Err(e) = crypto_aggregator::run(config, sender).await {
        info!("Aggregator error: {}", e);
    }

    let count = printer_task.await.unwrap_or(0);
    info!("Done. {} price updates received", count);
    Ok(())
}
