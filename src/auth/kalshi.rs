//! Kalshi API authentication using RSA-PSS signatures.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use pkcs1::DecodeRsaPrivateKey;
use rsa::{
    pss::SigningKey,
    sha2::Sha256,
    signature::{RandomizedSigner, SignatureEncoding},
    RsaPrivateKey,
};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::TradingEnvironment;

/// Kalshi API credentials
#[derive(Clone)]
pub struct KalshiAuth {
    pub api_key_id: String,
    private_key: RsaPrivateKey,
}

impl KalshiAuth {
    /// Load credentials for specified trading environment
    ///
    /// - Demo: KALSHI_DEMO_API_KEY_ID, KALSHI_DEMO_PRIVATE_KEY_PATH
    /// - Prod: KALSHI_PROD_API_KEY_ID, KALSHI_PROD_PRIVATE_KEY_PATH
    pub fn from_env(env: TradingEnvironment) -> Result<Self> {
        dotenvy::dotenv().ok();

        let prefix = env.env_key_prefix();

        let api_key_id = std::env::var(format!("{}_API_KEY_ID", prefix))?;
        let key_path = std::env::var(format!("{}_PRIVATE_KEY_PATH", prefix))?;

        let private_key_pem = std::fs::read_to_string(&key_path)
            .with_context(|| format!("Failed to read private key from {}", key_path))?;

        let private_key = RsaPrivateKey::from_pkcs1_pem(private_key_pem.trim())
            .context("Failed to parse RSA private key PEM")?;

        Ok(Self {
            api_key_id,
            private_key,
        })
    }

    /// Sign a message using RSA-PSS with SHA256
    ///
    /// Message format: "{timestamp_ms}{METHOD}{path}"
    /// Example: "1234567890123GET/trade-api/v2/portfolio/balance"
    pub fn sign(&self, message: &str) -> Result<String> {
        let signing_key = SigningKey::<Sha256>::new(self.private_key.clone());
        let signature = signing_key.sign_with_rng(&mut rand::thread_rng(), message.as_bytes());
        Ok(BASE64.encode(signature.to_bytes()))
    }

    /// Get current timestamp in milliseconds
    #[inline]
    pub fn timestamp_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    /// Sign a REST API request
    ///
    /// Returns: (signature, timestamp_string)
    pub fn sign_request(&self, method: &str, path: &str) -> Result<(String, String)> {
        let timestamp = Self::timestamp_ms();
        let timestamp_str = timestamp.to_string();

        // Strip query parameters before signing (API requirement)
        let path_without_query = path.split('?').next().unwrap_or(path);

        // Signature message: {timestamp}{METHOD}{full_path}
        // Full path must include /trade-api/v2 prefix
        let full_path = if path_without_query.starts_with("/trade-api/v2") {
            path_without_query.to_string()
        } else {
            format!("/trade-api/v2{}", path_without_query)
        };

        let message = format!("{}{}{}", timestamp, method, full_path);
        let signature = self.sign(&message)?;

        Ok((signature, timestamp_str))
    }

    /// Generate WebSocket authentication headers
    ///
    /// Returns: (api_key, signature, timestamp_string)
    pub fn ws_auth_headers(&self) -> Result<(String, String, String)> {
        let timestamp = Self::timestamp_ms();
        let timestamp_str = timestamp.to_string();

        // WebSocket signature message format
        let message = format!("{}GET/trade-api/ws/v2", timestamp);
        let signature = self.sign(&message)?;

        Ok((self.api_key_id.clone(), signature, timestamp_str))
    }
}
