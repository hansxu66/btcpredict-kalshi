//! Kalshi WebSocket fill monitor for tracking order executions.
//!
//! Subscribes to the "fill" channel to receive real-time fill notifications.

use anyhow::{Context, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{http::Request, Message},
};
use tracing::{debug, error, info, trace, warn};

use crate::auth::KalshiAuth;
use crate::types::{FillUpdate, OrderAction, OrderSide, TradingEnvironment};

// =============================================================================
// FILL MESSAGE TYPES
// =============================================================================

/// Subscribe command for fills channel
#[derive(Debug, serde::Serialize)]
struct FillSubscribeCmd {
    id: i32,
    cmd: &'static str,
    params: FillSubscribeParams,
}

#[derive(Debug, serde::Serialize)]
struct FillSubscribeParams {
    channels: Vec<&'static str>,
}

/// WebSocket message wrapper for fills
#[derive(Debug, Deserialize)]
struct FillWsMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    sid: Option<i32>,
    #[serde(default)]
    msg: Option<FillMessageBody>,
}

/// Fill message body from Kalshi
#[derive(Debug, Deserialize)]
struct FillMessageBody {
    /// Order ID
    #[serde(default)]
    order_id: Option<String>,
    /// Market ticker
    #[serde(default)]
    market_ticker: Option<String>,
    /// Action: "buy" or "sell"
    #[serde(default)]
    action: Option<String>,
    /// Side: "yes" or "no"
    #[serde(default)]
    side: Option<String>,
    /// Number of contracts filled
    #[serde(default)]
    count: Option<i64>,
    /// YES price in cents
    #[serde(default)]
    yes_price: Option<i64>,
    /// NO price in cents
    #[serde(default)]
    no_price: Option<i64>,
    /// Trade ID
    #[serde(default)]
    trade_id: Option<String>,
}

// =============================================================================
// FILL MONITOR
// =============================================================================

/// Run the Kalshi fill monitor WebSocket
///
/// # Arguments
/// * `auth` - Kalshi authentication
/// * `env` - Trading environment (Demo or Production)
/// * `market_ticker` - Market to filter fills for (or None for all fills)
/// * `sender` - Channel to send fill updates to Market Maker
pub async fn run(
    auth: &KalshiAuth,
    env: TradingEnvironment,
    market_ticker: Option<&str>,
    sender: mpsc::Sender<FillUpdate>,
) -> Result<()> {
    // Generate auth headers
    let (api_key, signature, timestamp) = auth.ws_auth_headers()?;

    // Build WebSocket request with authentication headers
    let request = Request::builder()
        .uri(env.ws_url())
        .header("KALSHI-ACCESS-KEY", &api_key)
        .header("KALSHI-ACCESS-SIGNATURE", &signature)
        .header("KALSHI-ACCESS-TIMESTAMP", &timestamp)
        .header("Host", env.ws_host())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())?;

    info!("[FILLS] Connecting to WebSocket ({})...", env);
    let (ws_stream, _response) = connect_async(request)
        .await
        .context("Failed to connect to Kalshi WebSocket for fills")?;

    info!("[FILLS] Connected to WebSocket");

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to fill channel (global - receives all fills for the account)
    let subscribe_cmd = FillSubscribeCmd {
        id: 1,
        cmd: "subscribe",
        params: FillSubscribeParams {
            channels: vec!["fill"],
        },
    };

    let subscribe_json = serde_json::to_string(&subscribe_cmd)?;
    debug!("[FILLS] Sending subscribe: {}", subscribe_json);
    write.send(Message::Text(subscribe_json)).await?;

    info!("[FILLS] Subscribed to fill channel");

    // Message processing loop
    while let Some(msg_result) = read.next().await {
        match msg_result {
            Ok(Message::Text(text)) => {
                trace!("[FILLS] Received: {}", text);

                match serde_json::from_str::<FillWsMessage>(&text) {
                    Ok(ws_msg) => {
                        process_fill_message(&ws_msg, market_ticker, &sender).await;
                    }
                    Err(e) => {
                        trace!("[FILLS] Parse error: {}", e);
                    }
                }
            }
            Ok(Message::Ping(data)) => {
                trace!("[FILLS] Received ping, sending pong");
                if let Err(e) = write.send(Message::Pong(data)).await {
                    warn!("[FILLS] Failed to send pong: {}", e);
                }
            }
            Ok(Message::Pong(_)) => {
                trace!("[FILLS] Received pong");
            }
            Ok(Message::Close(frame)) => {
                info!("[FILLS] WebSocket closed: {:?}", frame);
                break;
            }
            Ok(Message::Binary(_)) => {
                trace!("[FILLS] Received binary message (ignored)");
            }
            Ok(Message::Frame(_)) => {
                trace!("[FILLS] Received raw frame (ignored)");
            }
            Err(e) => {
                error!("[FILLS] WebSocket error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Process a fill WebSocket message
async fn process_fill_message(
    ws_msg: &FillWsMessage,
    filter_ticker: Option<&str>,
    sender: &mpsc::Sender<FillUpdate>,
) {
    match ws_msg.msg_type.as_str() {
        "fill" => {
            if let Some(body) = &ws_msg.msg {
                // Extract required fields
                let order_id = match &body.order_id {
                    Some(id) => id.clone(),
                    None => {
                        warn!("[FILLS] Fill message missing order_id");
                        return;
                    }
                };

                let ticker = match &body.market_ticker {
                    Some(t) => t.clone(),
                    None => {
                        warn!("[FILLS] Fill message missing market_ticker");
                        return;
                    }
                };

                // Filter by ticker if specified
                if let Some(filter) = filter_ticker {
                    if ticker != filter {
                        trace!("[FILLS] Ignoring fill for different ticker: {}", ticker);
                        return;
                    }
                }

                let action = match body.action.as_deref() {
                    Some("buy") => OrderAction::Buy,
                    Some("sell") => OrderAction::Sell,
                    _ => {
                        warn!("[FILLS] Fill message has invalid action: {:?}", body.action);
                        return;
                    }
                };

                let side = match body.side.as_deref() {
                    Some("yes") => OrderSide::Yes,
                    Some("no") => OrderSide::No,
                    _ => {
                        warn!("[FILLS] Fill message has invalid side: {:?}", body.side);
                        return;
                    }
                };

                let count = body.count.unwrap_or(0);
                if count <= 0 {
                    warn!("[FILLS] Fill message has invalid count: {}", count);
                    return;
                }

                // Get price based on side
                let price_cents = match side {
                    OrderSide::Yes => body.yes_price.unwrap_or(0),
                    OrderSide::No => body.no_price.unwrap_or(0),
                };

                if price_cents <= 0 || price_cents >= 100 {
                    warn!("[FILLS] Fill message has invalid price: {}", price_cents);
                    return;
                }

                // Create fill update
                let fill = FillUpdate {
                    order_id,
                    ticker: ticker.clone(),
                    side,
                    action,
                    price_cents,
                    count,
                    timestamp: Utc::now(),
                };

                // Log the fill
                info!(
                    "[FILLS] {} | {:?} {:?} {} @ {}¢ | order={}",
                    ticker,
                    action,
                    side,
                    count,
                    price_cents,
                    fill.order_id
                );

                // Send to market maker
                if let Err(e) = sender.send(fill).await {
                    error!("[FILLS] Failed to send fill update: {}", e);
                }
            }
        }
        "subscribed" => {
            info!("[FILLS] Subscription confirmed: {:?}", ws_msg.msg);
        }
        "error" => {
            error!("[FILLS] WebSocket error message: {:?}", ws_msg.msg);
        }
        _ => {
            trace!("[FILLS] Unhandled message type: {}", ws_msg.msg_type);
        }
    }
}
