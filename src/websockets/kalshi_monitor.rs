//! Kalshi WebSocket connection and message handling.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{http::Request, Message},
};
use tracing::{debug, error, info, trace, warn};

use crate::auth::KalshiAuth;
use crate::types::{
    MonitorUpdate, OrderbookState, ProbabilityUpdate, SubscribeCmd, SubscribeParams,
    TradingEnvironment, WsMessage,
};

/// Run the Kalshi WebSocket connection and send updates through channel
pub async fn run(
    auth: &KalshiAuth,
    env: TradingEnvironment,
    market_ticker: &str,
    sender: mpsc::Sender<MonitorUpdate>,
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

    info!("[KALSHI] Connecting to WebSocket ({})...", env);
    let (ws_stream, _response) = connect_async(request)
        .await
        .context("Failed to connect to Kalshi WebSocket")?;

    info!("[KALSHI] Connected to WebSocket");

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to orderbook_delta channel for the market
    let subscribe_cmd = SubscribeCmd {
        id: 1,
        cmd: "subscribe",
        params: SubscribeParams {
            channels: vec!["orderbook_delta"],
            market_tickers: vec![market_ticker.to_string()],
        },
    };

    let subscribe_json = serde_json::to_string(&subscribe_cmd)?;
    debug!("[KALSHI] Sending subscribe: {}", subscribe_json);
    write.send(Message::Text(subscribe_json)).await?;

    info!("[KALSHI] Subscribed to market: {}", market_ticker);

    // Track orderbook state
    let mut state = OrderbookState::new();
    let mut last_yes_prob: Option<f64> = None;

    // Message processing loop
    while let Some(msg_result) = read.next().await {
        match msg_result {
            Ok(Message::Text(text)) => {
                trace!("[KALSHI] Received: {}", text);

                match serde_json::from_str::<WsMessage>(&text) {
                    Ok(ws_msg) => {
                        process_message(
                            &ws_msg,
                            market_ticker,
                            &mut state,
                            &mut last_yes_prob,
                            &sender,
                        ).await;
                    }
                    Err(e) => {
                        trace!("[KALSHI] Parse error (likely non-orderbook msg): {}", e);
                    }
                }
            }
            Ok(Message::Ping(data)) => {
                trace!("[KALSHI] Received ping, sending pong");
                if let Err(e) = write.send(Message::Pong(data)).await {
                    warn!("[KALSHI] Failed to send pong: {}", e);
                }
            }
            Ok(Message::Pong(_)) => {
                trace!("[KALSHI] Received pong");
            }
            Ok(Message::Close(frame)) => {
                info!("[KALSHI] WebSocket closed: {:?}", frame);
                break;
            }
            Ok(Message::Binary(_)) => {
                trace!("[KALSHI] Received binary message (ignored)");
            }
            Ok(Message::Frame(_)) => {
                trace!("[KALSHI] Received raw frame (ignored)");
            }
            Err(e) => {
                error!("[KALSHI] WebSocket error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Process a WebSocket message and emit probability updates
async fn process_message(
    ws_msg: &WsMessage,
    market_ticker: &str,
    state: &mut OrderbookState,
    last_yes_prob: &mut Option<f64>,
    sender: &mpsc::Sender<MonitorUpdate>,
) {
    // Check if this message is for our market
    let msg_ticker = ws_msg
        .msg
        .as_ref()
        .and_then(|m| m.market_ticker.as_ref());

    if let Some(ticker) = msg_ticker {
        if ticker != market_ticker {
            return;
        }
    }

    match ws_msg.msg_type.as_str() {
        "orderbook_snapshot" => {
            if let Some(body) = &ws_msg.msg {
                state.update_from_snapshot(body);
                emit_update(market_ticker, state, last_yes_prob, sender).await;
            }
        }
        "orderbook_delta" => {
            if let Some(body) = &ws_msg.msg {
                let old_yes = state.yes_bid;
                let old_no = state.no_bid;

                state.update_from_delta(body);

                // Only emit if prices changed
                if state.yes_bid != old_yes || state.no_bid != old_no {
                    emit_update(market_ticker, state, last_yes_prob, sender).await;
                }
            }
        }
        "subscribed" => {
            info!("[KALSHI] Subscription confirmed: {:?}", ws_msg.msg);
        }
        "error" => {
            error!("[KALSHI] WebSocket error message: {:?}", ws_msg.msg);
        }
        _ => {
            trace!("[KALSHI] Unhandled message type: {}", ws_msg.msg_type);
        }
    }
}

/// Emit a probability update through the channel
async fn emit_update(
    market_ticker: &str,
    state: &OrderbookState,
    last_yes_prob: &mut Option<f64>,
    sender: &mpsc::Sender<MonitorUpdate>,
) {
    if !state.is_valid() {
        return;
    }

    let update = ProbabilityUpdate::new(market_ticker, state);

    // Calculate delta if we have a previous value
    let delta_str = if let Some(last) = *last_yes_prob {
        let delta: f64 = update.yes_prob - last;
        if delta.abs() > 0.001 {
            format!(" | Δ{:+.1}%", delta * 100.0)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    *last_yes_prob = Some(update.yes_prob);

    // Log the update
    info!(
        "[KALSHI] {} | YES: {:.1}% ({:2}¢) | NO: {:.1}% ({:2}¢){}",
        update.market_ticker,
        update.yes_prob * 100.0,
        update.yes_bid,
        update.no_prob * 100.0,
        update.no_bid,
        delta_str
    );

    // Send through channel to calculator
    if let Err(e) = sender.send(MonitorUpdate::Kalshi(update)).await {
        error!("[KALSHI] Failed to send update: {}", e);
    }
}
