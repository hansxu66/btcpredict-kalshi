//! Redis client for publishing real-time updates.
//!
//! Provides non-blocking publish to Redis pub/sub channels and state storage.
//!
//! Channels:
//! - `kalshi:updates` - Kalshi probability updates
//! - `bolt:updates` - BoltOdds moneyline updates
//!
//! State keys:
//! - `kalshi:state:{ticker}` - Latest orderbook state per market
//! - `bolt:state:{game}:{sportsbook}` - Latest moneyline odds per game/sportsbook

use anyhow::{Context, Result};
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Redis channel names (pub/sub for live updates)
pub mod channels {
    pub const KALSHI_UPDATES: &str = "kalshi:updates";
    pub const BOLT_UPDATES: &str = "bolt:updates";
    pub const BOLT_PROBS: &str = "bolt:probs";
    pub const ODDS_UPDATES: &str = "odds:updates";
    pub const CALCULATOR_UPDATES: &str = "calculator:updates";
}

/// Redis key prefixes (latest state)
pub mod keys {
    pub const KALSHI_STATE: &str = "kalshi:state";
    pub const BOLT_STATE: &str = "bolt:state";
    pub const BOLT_PROBS_STATE: &str = "bolt:probs:state";
    pub const ODDS_STATE: &str = "odds:state";
    pub const CALCULATOR_STATE: &str = "calculator:state";
}

/// Redis stream names (persistent history)
pub mod streams {
    pub const KALSHI_STREAM: &str = "kalshi:stream";
    pub const BOLT_PROBS_STREAM: &str = "bolt:probs:stream";
    pub const CALCULATOR_STREAM: &str = "calculator:stream";
}

/// Redis client wrapper with automatic reconnection
#[derive(Clone)]
pub struct RedisClient {
    connection: Arc<RwLock<Option<MultiplexedConnection>>>,
    url: String,
}

impl RedisClient {
    /// Create a new Redis client
    ///
    /// Does not connect immediately - call `connect()` or use `new_connected()`.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            connection: Arc::new(RwLock::new(None)),
            url: url.into(),
        }
    }

    /// Create a new Redis client and connect immediately
    pub async fn new_connected(url: impl Into<String>) -> Result<Self> {
        let client = Self::new(url);
        client.connect().await?;
        Ok(client)
    }

    /// Create from environment variable REDIS_URL (defaults to localhost)
    pub async fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        Self::new_connected(url).await
    }

    /// Connect to Redis
    pub async fn connect(&self) -> Result<()> {
        let client = redis::Client::open(self.url.as_str())
            .context("Failed to create Redis client")?;

        let connection = client
            .get_multiplexed_async_connection()
            .await
            .context("Failed to connect to Redis")?;

        info!("[REDIS] Connected to {}", self.url);

        let mut conn = self.connection.write().await;
        *conn = Some(connection);

        Ok(())
    }

    /// Check if connected
    pub async fn is_connected(&self) -> bool {
        self.connection.read().await.is_some()
    }

    /// Publish a message to a channel (non-blocking via spawn)
    ///
    /// This spawns the publish operation so it doesn't block the caller.
    /// Errors are logged but not returned.
    pub fn publish_nonblocking(&self, channel: &str, message: impl Into<String> + Send + 'static) {
        let client = self.clone();
        let channel = channel.to_string();
        let message = message.into();

        tokio::spawn(async move {
            if let Err(e) = client.publish(&channel, &message).await {
                warn!("[REDIS] Publish failed: {}", e);
            }
        });
    }

    /// Publish a message to a channel
    pub async fn publish(&self, channel: &str, message: &str) -> Result<()> {
        let conn_guard = self.connection.read().await;
        let Some(conn) = conn_guard.as_ref() else {
            return Err(anyhow::anyhow!("Not connected to Redis"));
        };

        let mut conn = conn.clone();
        drop(conn_guard);

        let _: () = conn
            .publish(channel, message)
            .await
            .context("Failed to publish message")?;

        debug!("[REDIS] Published to {}: {} bytes", channel, message.len());
        Ok(())
    }

    /// Set a key with optional expiration (for state storage)
    pub async fn set_state(&self, key: &str, value: &str, expire_secs: Option<u64>) -> Result<()> {
        let conn_guard = self.connection.read().await;
        let Some(conn) = conn_guard.as_ref() else {
            return Err(anyhow::anyhow!("Not connected to Redis"));
        };

        let mut conn = conn.clone();
        drop(conn_guard);

        if let Some(secs) = expire_secs {
            let _: () = conn
                .set_ex(key, value, secs)
                .await
                .context("Failed to set key with expiration")?;
        } else {
            let _: () = conn
                .set(key, value)
                .await
                .context("Failed to set key")?;
        }

        debug!("[REDIS] Set {}: {} bytes", key, value.len());
        Ok(())
    }

    /// Set state non-blocking (spawns the operation)
    pub fn set_state_nonblocking(
        &self,
        key: &str,
        value: impl Into<String> + Send + 'static,
        expire_secs: Option<u64>,
    ) {
        let client = self.clone();
        let key = key.to_string();
        let value = value.into();

        tokio::spawn(async move {
            if let Err(e) = client.set_state(&key, &value, expire_secs).await {
                warn!("[REDIS] Set state failed: {}", e);
            }
        });
    }

    /// Get a key value
    pub async fn get_state(&self, key: &str) -> Result<Option<String>> {
        let conn_guard = self.connection.read().await;
        let Some(conn) = conn_guard.as_ref() else {
            return Err(anyhow::anyhow!("Not connected to Redis"));
        };

        let mut conn = conn.clone();
        drop(conn_guard);

        let value: Option<String> = conn
            .get(key)
            .await
            .context("Failed to get key")?;

        Ok(value)
    }

    /// Append to a Redis Stream with max length cap (non-blocking)
    ///
    /// Uses XADD with MAXLEN ~ for approximate trimming (faster).
    pub fn stream_nonblocking(
        &self,
        stream: &str,
        data: impl Into<String> + Send + 'static,
        max_len: usize,
    ) {
        let client = self.clone();
        let stream = stream.to_string();
        let data = data.into();

        tokio::spawn(async move {
            if let Err(e) = client.stream_add(&stream, &data, max_len).await {
                warn!("[REDIS] Stream add failed: {}", e);
            }
        });
    }

    /// Append to a Redis Stream (blocking)
    pub async fn stream_add(&self, stream: &str, data: &str, max_len: usize) -> Result<()> {
        let conn_guard = self.connection.read().await;
        let Some(conn) = conn_guard.as_ref() else {
            return Err(anyhow::anyhow!("Not connected to Redis"));
        };

        let mut conn = conn.clone();
        drop(conn_guard);

        // XADD stream MAXLEN ~ max_len * data
        let _: String = redis::cmd("XADD")
            .arg(&stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(max_len)
            .arg("*")
            .arg("data")
            .arg(data)
            .query_async(&mut conn)
            .await
            .context("Failed to add to stream")?;

        debug!("[REDIS] Stream {}: {} bytes", stream, data.len());
        Ok(())
    }
}

/// Max entries in Redis Streams (~27 hours at 1 update/sec)
const STREAM_MAX_LEN: usize = 100_000;

/// Helper to publish Kalshi updates
pub fn publish_kalshi_update(client: &RedisClient, json: String, ticker: &str) {
    // Publish to channel (live dashboard)
    client.publish_nonblocking(channels::KALSHI_UPDATES, json.clone());

    // Update state key (latest value)
    let state_key = format!("{}:{}", keys::KALSHI_STATE, ticker);
    client.set_state_nonblocking(&state_key, json.clone(), Some(3600));

    // Append to stream (persistent history)
    client.stream_nonblocking(streams::KALSHI_STREAM, json, STREAM_MAX_LEN);
}

/// Helper to publish Bolt updates
pub fn publish_bolt_update(client: &RedisClient, json: String, game: &str, sportsbook: &str) {
    // Publish to channel (live dashboard)
    client.publish_nonblocking(channels::BOLT_UPDATES, json.clone());

    // Update state key (latest value)
    let game_key = game.replace(' ', "_").replace(',', "");
    let state_key = format!("{}:{}:{}", keys::BOLT_STATE, game_key, sportsbook);
    client.set_state_nonblocking(&state_key, json, Some(3600));
}

/// Helper to publish Bolt fair probabilities (no-vig)
pub fn publish_bolt_probs(client: &RedisClient, json: String, game: &str, sportsbook: &str) {
    // Publish to channel (live dashboard)
    client.publish_nonblocking(channels::BOLT_PROBS, json.clone());

    // Update state key (latest value)
    let game_key = game.replace(' ', "_").replace(',', "");
    let state_key = format!("{}:{}:{}", keys::BOLT_PROBS_STATE, game_key, sportsbook);
    client.set_state_nonblocking(&state_key, json.clone(), Some(3600));

    // Append to stream (persistent history)
    client.stream_nonblocking(streams::BOLT_PROBS_STREAM, json, STREAM_MAX_LEN);
}

/// Helper to publish Odds (Kalstrop) updates
pub fn publish_odds_update(client: &RedisClient, json: String, fixture_id: &str) {
    // Publish to channel (live dashboard)
    client.publish_nonblocking(channels::ODDS_UPDATES, json.clone());

    // Update state key (latest value)
    let state_key = format!("{}:{}", keys::ODDS_STATE, fixture_id);
    client.set_state_nonblocking(&state_key, json, Some(3600));
}

/// Helper to publish Calculator fair price updates
pub fn publish_calculator_update(client: &RedisClient, json: String, ticker_id: &str) {
    // Publish to channel (live dashboard)
    client.publish_nonblocking(channels::CALCULATOR_UPDATES, json.clone());

    // Update state key (latest value)
    let state_key = format!("{}:{}", keys::CALCULATOR_STATE, ticker_id);
    client.set_state_nonblocking(&state_key, json.clone(), Some(3600));

    // Append to stream (persistent history)
    client.stream_nonblocking(streams::CALCULATOR_STREAM, json, STREAM_MAX_LEN);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires running Redis server
    async fn test_connect() {
        let client = RedisClient::new_connected("redis://127.0.0.1:6379")
            .await
            .expect("Failed to connect");

        assert!(client.is_connected().await);
    }

    #[tokio::test]
    #[ignore] // Requires running Redis server
    async fn test_publish() {
        let client = RedisClient::new_connected("redis://127.0.0.1:6379")
            .await
            .expect("Failed to connect");

        client
            .publish("test:channel", "hello world")
            .await
            .expect("Failed to publish");
    }
}
