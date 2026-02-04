//! Kalshi REST API client for orders, positions, and account management.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;
use tracing::{debug, warn};

use crate::auth::KalshiAuth;
use crate::types::{
    AmendOrderRequest, Balance, BalanceResponse, CreateOrderRequest, Order, OrderResponse,
    OrderSide, OrdersResponse, Position, PositionsResponse, TradingEnvironment,
};

/// Rate limit delay between requests (ms)
const API_DELAY_MS: u64 = 60;

/// Request timeout
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Order request timeout (shorter)
const ORDER_TIMEOUT: Duration = Duration::from_secs(5);

/// Max retries on rate limit
const MAX_RETRIES: u32 = 5;

/// Kalshi REST API client
pub struct KalshiClient {
    http: reqwest::Client,
    auth: KalshiAuth,
    env: TradingEnvironment,
}

impl KalshiClient {
    /// Create a new API client
    pub fn new(auth: KalshiAuth, env: TradingEnvironment) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("Failed to build HTTP client"),
            auth,
            env,
        }
    }

    /// Get the trading environment
    pub fn environment(&self) -> TradingEnvironment {
        self.env
    }

    /// Get the base URL for this environment
    fn base_url(&self) -> &'static str {
        self.env.api_base_url()
    }

    // =========================================================================
    // Internal HTTP Methods
    // =========================================================================

    /// Authenticated GET request with retry on rate limit
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request::<T, ()>("GET", path, None).await
    }

    /// Authenticated POST request
    async fn post<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        self.request("POST", path, Some(body)).await
    }

    /// Authenticated PUT request
    async fn put<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        self.request("PUT", path, Some(body)).await
    }

    /// Authenticated DELETE request
    async fn delete(&self, path: &str) -> Result<()> {
        self.request_no_response("DELETE", path).await
    }

    /// Generic authenticated request with retry on rate limit
    async fn request<T: DeserializeOwned, B: Serialize>(
        &self,
        method: &str,
        path: &str,
        body: Option<&B>,
    ) -> Result<T> {
        let mut retries = 0;

        loop {
            let url = format!("{}{}", self.base_url(), path);
            let (signature, timestamp) = self.auth.sign_request(method, path)?;

            let mut request = match method {
                "GET" => self.http.get(&url),
                "POST" => self.http.post(&url),
                "PUT" => self.http.put(&url),
                "DELETE" => self.http.delete(&url),
                _ => anyhow::bail!("Unsupported HTTP method: {}", method),
            };

            request = request
                .header("KALSHI-ACCESS-KEY", &self.auth.api_key_id)
                .header("KALSHI-ACCESS-SIGNATURE", &signature)
                .header("KALSHI-ACCESS-TIMESTAMP", &timestamp);

            if let Some(b) = body {
                request = request
                    .header("Content-Type", "application/json")
                    .timeout(ORDER_TIMEOUT)
                    .json(b);
            }

            let resp = request.send().await?;
            let status = resp.status();

            // Handle rate limit with exponential backoff
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                retries += 1;
                if retries > MAX_RETRIES {
                    anyhow::bail!("Rate limited after {} retries", MAX_RETRIES);
                }
                let backoff_ms = 2000 * (1 << retries);
                warn!(
                    "Rate limited, backing off {}ms (retry {}/{})",
                    backoff_ms, retries, MAX_RETRIES
                );
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                continue;
            }

            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("API error {}: {}", status, body);
            }

            let data: T = resp.json().await.context("Failed to parse response")?;

            // Rate limit delay
            tokio::time::sleep(Duration::from_millis(API_DELAY_MS)).await;

            return Ok(data);
        }
    }

    /// Request without response body (for DELETE)
    async fn request_no_response(&self, method: &str, path: &str) -> Result<()> {
        let url = format!("{}{}", self.base_url(), path);
        let (signature, timestamp) = self.auth.sign_request(method, path)?;

        let request = self
            .http
            .delete(&url)
            .header("KALSHI-ACCESS-KEY", &self.auth.api_key_id)
            .header("KALSHI-ACCESS-SIGNATURE", &signature)
            .header("KALSHI-ACCESS-TIMESTAMP", &timestamp);

        let resp = request.send().await?;
        let status = resp.status();

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("API error {}: {}", status, body);
        }

        tokio::time::sleep(Duration::from_millis(API_DELAY_MS)).await;
        Ok(())
    }

    // =========================================================================
    // Account / Portfolio
    // =========================================================================

    /// Get account balance
    pub async fn get_balance(&self) -> Result<Balance> {
        let resp: BalanceResponse = self.get("/portfolio/balance").await?;
        Ok(resp.balance)
    }

    /// Get all positions
    pub async fn get_positions(&self) -> Result<Vec<Position>> {
        let resp: PositionsResponse = self.get("/portfolio/positions").await?;
        Ok(resp.market_positions)
    }

    /// Get position for a specific market
    pub async fn get_position(&self, ticker: &str) -> Result<Option<Position>> {
        let path = format!("/portfolio/positions?ticker={}", ticker);
        let resp: PositionsResponse = self.get(&path).await?;
        Ok(resp.market_positions.into_iter().next())
    }

    // =========================================================================
    // Orders - Core
    // =========================================================================

    /// Create a new order
    pub async fn create_order(&self, request: CreateOrderRequest) -> Result<Order> {
        debug!(
            "Creating order: {:?} {:?} {} @{:?}¢ x{}",
            request.action,
            request.side,
            request.ticker,
            request.yes_price.or(request.no_price),
            request.count
        );
        let resp: OrderResponse = self.post("/portfolio/orders", &request).await?;
        Ok(resp.order)
    }

    /// Get order by ID
    pub async fn get_order(&self, order_id: &str) -> Result<Order> {
        let path = format!("/portfolio/orders/{}", order_id);
        let resp: OrderResponse = self.get(&path).await?;
        Ok(resp.order)
    }

    /// List open (resting) orders, optionally filtered by ticker
    pub async fn get_orders(&self, ticker: Option<&str>) -> Result<Vec<Order>> {
        let path = match ticker {
            Some(t) => format!("/portfolio/orders?ticker={}&status=resting", t),
            None => "/portfolio/orders?status=resting".to_string(),
        };
        let resp: OrdersResponse = self.get(&path).await?;
        Ok(resp.orders)
    }

    /// Cancel an order by ID
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        let path = format!("/portfolio/orders/{}", order_id);
        self.delete(&path).await
    }

    /// Cancel all open orders
    pub async fn cancel_all_orders(&self) -> Result<()> {
        self.delete("/portfolio/orders").await
    }

    /// Amend an order (change price or count)
    pub async fn amend_order(&self, order_id: &str, request: AmendOrderRequest) -> Result<Order> {
        let path = format!("/portfolio/orders/{}", order_id);
        let resp: OrderResponse = self.put(&path, &request).await?;
        Ok(resp.order)
    }

    // =========================================================================
    // Orders - Convenience Methods
    // =========================================================================

    /// Place a limit buy order (rests on book)
    pub async fn buy_limit(
        &self,
        ticker: &str,
        side: OrderSide,
        price_cents: i64,
        count: i64,
    ) -> Result<Order> {
        let request = CreateOrderRequest::limit_buy(ticker, side, price_cents, count);
        self.create_order(request).await
    }

    /// Place a limit sell order (rests on book)
    pub async fn sell_limit(
        &self,
        ticker: &str,
        side: OrderSide,
        price_cents: i64,
        count: i64,
    ) -> Result<Order> {
        let request = CreateOrderRequest::limit_sell(ticker, side, price_cents, count);
        self.create_order(request).await
    }

    /// Place an IOC buy order (immediate-or-cancel)
    pub async fn buy_ioc(
        &self,
        ticker: &str,
        side: OrderSide,
        price_cents: i64,
        count: i64,
    ) -> Result<Order> {
        let request = CreateOrderRequest::ioc_buy(ticker, side, price_cents, count);
        self.create_order(request).await
    }

    /// Place an IOC sell order (immediate-or-cancel)
    pub async fn sell_ioc(
        &self,
        ticker: &str,
        side: OrderSide,
        price_cents: i64,
        count: i64,
    ) -> Result<Order> {
        let request = CreateOrderRequest::ioc_sell(ticker, side, price_cents, count);
        self.create_order(request).await
    }
}
