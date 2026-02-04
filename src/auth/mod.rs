//! Authentication and configuration modules for different APIs.

pub mod binance;
pub mod coinbase;
pub mod cryptocom;
pub mod kalshi;
pub mod kraken;

pub use binance::BinanceConfig;
pub use coinbase::CoinbaseConfig;
pub use cryptocom::CryptocomConfig;
pub use kalshi::KalshiAuth;
pub use kraken::KrakenConfig;
