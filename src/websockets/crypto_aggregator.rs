//! Aggregated Crypto Price Monitor
//!
//! Maintains WebSocket connections to multiple exchanges (Binance, Coinbase, Kraken, Crypto.com)
//! and emits aggregated mean prices whenever any exchange updates.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::auth::{BinanceConfig, CoinbaseConfig, CryptocomConfig, KrakenConfig};
use crate::types::crypto_aggregator::{
    AggregatedPriceUpdate, AggregatorState, CryptoAggregatorEvent, Exchange, ExchangePrice,
};
use crate::types::MonitorUpdate;

// Binance types
use crate::types::binance::BookTickerMessage;
// Coinbase types
use crate::types::coinbase::CoinbaseMessage;
// Kraken types
use crate::types::kraken::{parse_kraken_message, KrakenMessage};
// Crypto.com types
use crate::types::cryptocom::CryptocomResponse;

/// Minimum price change (in dollars) to emit an update from individual exchanges
const MIN_PRICE_CHANGE: f64 = 0.50;

/// Internal message for exchange price updates
#[derive(Debug)]
enum InternalUpdate {
    Price(ExchangePrice),
    Connected(Exchange),
    Disconnected(Exchange),
}

/// Configuration for the aggregator
#[derive(Debug, Clone)]
pub struct AggregatorConfig {
    pub enable_binance: bool,
    pub enable_coinbase: bool,
    pub enable_kraken: bool,
    pub enable_cryptocom: bool,
    pub reconnect_delay_secs: u64,
}

impl Default for AggregatorConfig {
    fn default() -> Self {
        Self {
            enable_binance: true,
            enable_coinbase: true,
            enable_kraken: true,
            enable_cryptocom: true,
            reconnect_delay_secs: 5,
        }
    }
}

/// Run the aggregated crypto monitor
pub async fn run(
    config: AggregatorConfig,
    sender: mpsc::Sender<MonitorUpdate>,
) -> Result<()> {
    // Internal channel for all exchange updates
    let (internal_tx, mut internal_rx) = mpsc::channel::<InternalUpdate>(100);

    // Spawn exchange monitors
    if config.enable_binance {
        let tx = internal_tx.clone();
        let delay = config.reconnect_delay_secs;
        tokio::spawn(async move {
            run_binance_loop(tx, delay).await;
        });
    }

    if config.enable_coinbase {
        let tx = internal_tx.clone();
        let delay = config.reconnect_delay_secs;
        tokio::spawn(async move {
            run_coinbase_loop(tx, delay).await;
        });
    }

    if config.enable_kraken {
        let tx = internal_tx.clone();
        let delay = config.reconnect_delay_secs;
        tokio::spawn(async move {
            run_kraken_loop(tx, delay).await;
        });
    }

    if config.enable_cryptocom {
        let tx = internal_tx.clone();
        let delay = config.reconnect_delay_secs;
        tokio::spawn(async move {
            run_cryptocom_loop(tx, delay).await;
        });
    }

    // Aggregation loop
    let mut state = AggregatorState::new();

    info!("[AGGREGATOR] Started - waiting for exchange connections");

    while let Some(update) = internal_rx.recv().await {
        match update {
            InternalUpdate::Price(price) => {
                let triggered_by = price.exchange;
                state.update(price);

                // Calculate aggregated prices
                if let (Some(mean_mid), Some(mean_bid), Some(mean_ask)) = (
                    state.mean_mid_price(),
                    state.mean_bid_price(),
                    state.mean_ask_price(),
                ) {
                    let exchange_prices = state
                        .prices
                        .iter()
                        .map(|(e, p)| (*e, p.mid_price))
                        .collect();

                    let agg_update = AggregatedPriceUpdate {
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        mean_mid_price: mean_mid,
                        mean_bid_price: mean_bid,
                        mean_ask_price: mean_ask,
                        exchange_count: state.exchange_count(),
                        triggered_by,
                        exchange_prices,
                    };

                    // Log the aggregated price
                    info!(
                        "[AGGREGATOR] mean=${:.2} | {} exchanges | triggered by {}",
                        mean_mid,
                        state.exchange_count(),
                        triggered_by
                    );

                    // Send to calculator
                    let event = CryptoAggregatorEvent::PriceUpdate(agg_update);
                    if let Err(e) = sender.send(MonitorUpdate::Crypto(event)).await {
                        error!("[AGGREGATOR] Failed to send update: {}", e);
                    }
                }
            }
            InternalUpdate::Connected(exchange) => {
                info!("[AGGREGATOR] {} connected", exchange);
                let event = CryptoAggregatorEvent::ExchangeConnected(exchange);
                let _ = sender.send(MonitorUpdate::Crypto(event)).await;
            }
            InternalUpdate::Disconnected(exchange) => {
                warn!("[AGGREGATOR] {} disconnected", exchange);
                let event = CryptoAggregatorEvent::ExchangeDisconnected(exchange);
                let _ = sender.send(MonitorUpdate::Crypto(event)).await;
            }
        }
    }

    Ok(())
}

// =============================================================================
// BINANCE
// =============================================================================

async fn run_binance_loop(tx: mpsc::Sender<InternalUpdate>, reconnect_delay: u64) {
    let config = BinanceConfig::btc_usdt();
    let mut last_mid: Option<f64> = None;

    loop {
        match run_binance(&config, &tx, &mut last_mid).await {
            Ok(()) => info!("[BINANCE] Connection closed"),
            Err(e) => error!("[BINANCE] Error: {}", e),
        }
        let _ = tx.send(InternalUpdate::Disconnected(Exchange::Binance)).await;
        info!("[BINANCE] Reconnecting in {}s...", reconnect_delay);
        tokio::time::sleep(Duration::from_secs(reconnect_delay)).await;
    }
}

async fn run_binance(
    config: &BinanceConfig,
    tx: &mpsc::Sender<InternalUpdate>,
    last_mid: &mut Option<f64>,
) -> Result<()> {
    let ws_url = config.book_ticker_url();
    let (ws_stream, _) = connect_async(&ws_url).await.context("Binance connect failed")?;
    let _ = tx.send(InternalUpdate::Connected(Exchange::Binance)).await;

    let (mut write, mut read) = ws_stream.split();

    while let Some(msg) = read.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(ticker) = serde_json::from_str::<BookTickerMessage>(&text) {
                    if let (Some(bid), Some(ask)) = (ticker.bid_price(), ticker.ask_price()) {
                        let mid = (bid + ask) / 2.0;
                        let should_emit = last_mid.map(|l| (mid - l).abs() >= MIN_PRICE_CHANGE).unwrap_or(true);
                        if should_emit {
                            *last_mid = Some(mid);
                            let price = ExchangePrice {
                                exchange: Exchange::Binance,
                                bid_price: bid,
                                ask_price: ask,
                                mid_price: mid,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                            };
                            let _ = tx.send(InternalUpdate::Price(price)).await;
                        }
                    }
                }
            }
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

// =============================================================================
// COINBASE
// =============================================================================

async fn run_coinbase_loop(tx: mpsc::Sender<InternalUpdate>, reconnect_delay: u64) {
    let config = CoinbaseConfig::btc_usd();
    let mut last_mid: Option<f64> = None;

    loop {
        match run_coinbase(&config, &tx, &mut last_mid).await {
            Ok(()) => info!("[COINBASE] Connection closed"),
            Err(e) => error!("[COINBASE] Error: {}", e),
        }
        let _ = tx.send(InternalUpdate::Disconnected(Exchange::Coinbase)).await;
        info!("[COINBASE] Reconnecting in {}s...", reconnect_delay);
        tokio::time::sleep(Duration::from_secs(reconnect_delay)).await;
    }
}

async fn run_coinbase(
    config: &CoinbaseConfig,
    tx: &mpsc::Sender<InternalUpdate>,
    last_mid: &mut Option<f64>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(config.ws_url()).await.context("Coinbase connect failed")?;
    let _ = tx.send(InternalUpdate::Connected(Exchange::Coinbase)).await;

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to ticker
    write.send(Message::Text(config.ticker_subscribe_msg())).await?;

    while let Some(msg) = read.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(msg) = serde_json::from_str::<CoinbaseMessage>(&text) {
                    if msg.channel == "ticker" {
                        for event in &msg.events {
                            for ticker in &event.tickers {
                                if let (Some(bid), Some(ask)) = (ticker.bid_price(), ticker.ask_price()) {
                                    if bid > 0.0 && ask > 0.0 {
                                        let mid = (bid + ask) / 2.0;
                                        let should_emit = last_mid.map(|l| (mid - l).abs() >= MIN_PRICE_CHANGE).unwrap_or(true);
                                        if should_emit {
                                            *last_mid = Some(mid);
                                            let price = ExchangePrice {
                                                exchange: Exchange::Coinbase,
                                                bid_price: bid,
                                                ask_price: ask,
                                                mid_price: mid,
                                                timestamp: chrono::Utc::now().to_rfc3339(),
                                            };
                                            let _ = tx.send(InternalUpdate::Price(price)).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

// =============================================================================
// KRAKEN
// =============================================================================

async fn run_kraken_loop(tx: mpsc::Sender<InternalUpdate>, reconnect_delay: u64) {
    let config = KrakenConfig::btc_usd();
    let mut last_mid: Option<f64> = None;

    loop {
        match run_kraken(&config, &tx, &mut last_mid).await {
            Ok(()) => info!("[KRAKEN] Connection closed"),
            Err(e) => error!("[KRAKEN] Error: {}", e),
        }
        let _ = tx.send(InternalUpdate::Disconnected(Exchange::Kraken)).await;
        info!("[KRAKEN] Reconnecting in {}s...", reconnect_delay);
        tokio::time::sleep(Duration::from_secs(reconnect_delay)).await;
    }
}

async fn run_kraken(
    config: &KrakenConfig,
    tx: &mpsc::Sender<InternalUpdate>,
    last_mid: &mut Option<f64>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(config.ws_url()).await.context("Kraken connect failed")?;
    let _ = tx.send(InternalUpdate::Connected(Exchange::Kraken)).await;

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to ticker
    write.send(Message::Text(config.ticker_subscribe_msg())).await?;

    while let Some(msg) = read.next().await {
        match msg? {
            Message::Text(text) => {
                if let KrakenMessage::Ticker(ticker) = parse_kraken_message(&text) {
                    if ticker.bid_price > 0.0 && ticker.ask_price > 0.0 {
                        let mid = ticker.mid_price();
                        let should_emit = last_mid.map(|l| (mid - l).abs() >= MIN_PRICE_CHANGE).unwrap_or(true);
                        if should_emit {
                            *last_mid = Some(mid);
                            let price = ExchangePrice {
                                exchange: Exchange::Kraken,
                                bid_price: ticker.bid_price,
                                ask_price: ticker.ask_price,
                                mid_price: mid,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                            };
                            let _ = tx.send(InternalUpdate::Price(price)).await;
                        }
                    }
                }
            }
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

// =============================================================================
// CRYPTO.COM
// =============================================================================

async fn run_cryptocom_loop(tx: mpsc::Sender<InternalUpdate>, reconnect_delay: u64) {
    let config = CryptocomConfig::btc_usdt();
    let mut last_mid: Option<f64> = None;

    loop {
        match run_cryptocom(&config, &tx, &mut last_mid).await {
            Ok(()) => info!("[CRYPTO.COM] Connection closed"),
            Err(e) => error!("[CRYPTO.COM] Error: {}", e),
        }
        let _ = tx.send(InternalUpdate::Disconnected(Exchange::Cryptocom)).await;
        info!("[CRYPTO.COM] Reconnecting in {}s...", reconnect_delay);
        tokio::time::sleep(Duration::from_secs(reconnect_delay)).await;
    }
}

async fn run_cryptocom(
    config: &CryptocomConfig,
    tx: &mpsc::Sender<InternalUpdate>,
    last_mid: &mut Option<f64>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(config.ws_url()).await.context("Crypto.com connect failed")?;
    let _ = tx.send(InternalUpdate::Connected(Exchange::Cryptocom)).await;

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to ticker
    write.send(Message::Text(config.ticker_subscribe_msg())).await?;

    while let Some(msg) = read.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(response) = serde_json::from_str::<CryptocomResponse>(&text) {
                    if let Some(ref result) = response.result {
                        if result.channel.starts_with("ticker.") {
                            for ticker in &result.data {
                                if let (Some(bid), Some(ask)) = (ticker.bid_price(), ticker.ask_price()) {
                                    if bid > 0.0 && ask > 0.0 {
                                        let mid = (bid + ask) / 2.0;
                                        let should_emit = last_mid.map(|l| (mid - l).abs() >= MIN_PRICE_CHANGE).unwrap_or(true);
                                        if should_emit {
                                            *last_mid = Some(mid);
                                            let price = ExchangePrice {
                                                exchange: Exchange::Cryptocom,
                                                bid_price: bid,
                                                ask_price: ask,
                                                mid_price: mid,
                                                timestamp: chrono::Utc::now().to_rfc3339(),
                                            };
                                            let _ = tx.send(InternalUpdate::Price(price)).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}
