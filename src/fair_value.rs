//! Fair probability estimation for BTC binary option markets.
//!
//! Uses Black-Scholes binary option pricing to estimate fair probability
//! that BTC will be above/below a strike price at expiry.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use std::f64::consts::PI;
use tracing::{debug, warn};

// =============================================================================
// VOLATILITY SOURCE (PLACEHOLDER)
// =============================================================================

/// Placeholder annualized volatility for BTC.
/// TODO: Replace with Deribit implied vol or historical vol calculation.
///
/// Current value: 50% annualized vol (reasonable for BTC)
/// Adjust this based on market conditions.
pub const PLACEHOLDER_VOLATILITY: f64 = 0.50;

/// Volatility source for fair value calculation
#[derive(Debug, Clone)]
pub enum VolatilitySource {
    /// Constant placeholder volatility
    Constant(f64),
    /// Historical realized volatility (future: from price data)
    Historical { window_hours: u32, value: f64 },
    /// Implied volatility from options market (future: from Deribit)
    Implied { source: String, value: f64 },
}

impl Default for VolatilitySource {
    fn default() -> Self {
        Self::Constant(PLACEHOLDER_VOLATILITY)
    }
}

impl VolatilitySource {
    pub fn get_vol(&self) -> f64 {
        match self {
            Self::Constant(v) => *v,
            Self::Historical { value, .. } => *value,
            Self::Implied { value, .. } => *value,
        }
    }
}

// =============================================================================
// MARKET SPECIFICATION
// =============================================================================

/// Market type for BTC binary options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketType {
    /// Pays if BTC >= strike at expiry
    Above,
    /// Pays if BTC < strike at expiry (or "between" for ranges)
    Below,
    /// Range market with floor and ceiling
    Range { floor: u64, ceiling: u64 },
}

/// Parsed BTC market specification
#[derive(Debug, Clone)]
pub struct BtcMarketSpec {
    /// Original ticker string
    pub ticker: String,
    /// Strike price in USD
    pub strike: f64,
    /// Expiry datetime (UTC)
    pub expiry: DateTime<Utc>,
    /// Market type (above/below)
    pub market_type: MarketType,
}

impl BtcMarketSpec {
    /// Time to expiry in years
    pub fn time_to_expiry(&self) -> f64 {
        let now = Utc::now();
        let duration = self.expiry.signed_duration_since(now);
        let seconds = duration.num_seconds() as f64;
        // Convert to years (365.25 days)
        seconds / (365.25 * 24.0 * 60.0 * 60.0)
    }

    /// Time to expiry in hours (more intuitive for short-dated)
    pub fn time_to_expiry_hours(&self) -> f64 {
        let now = Utc::now();
        let duration = self.expiry.signed_duration_since(now);
        duration.num_seconds() as f64 / 3600.0
    }

    /// Check if market has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expiry
    }
}

// =============================================================================
// TICKER PARSER
// =============================================================================

/// Parse a Kalshi BTC market ticker into a market specification.
///
/// Supports formats:
/// - `KXBTC-26FEB04-T1200-B97250` → Above $97,250 at 12:00 ET on Feb 4, 2026
/// - `KXBTC-26FEB04-T1200-B97250-97499` → Between $97,250 and $97,499
/// - Manual specification via `BtcMarketSpec::new()`
///
/// # Arguments
/// * `ticker` - The market ticker string
///
/// # Returns
/// Parsed market spec or None if parsing fails
pub fn parse_btc_ticker(ticker: &str) -> Option<BtcMarketSpec> {
    // Try different parsing strategies
    parse_kxbtc_format(ticker)
        .or_else(|| parse_simple_btc_format(ticker))
        .or_else(|| {
            warn!("Could not parse BTC ticker: {}", ticker);
            None
        })
}

/// Parse KXBTC format: KXBTC-26FEB04-T1200-B97250
fn parse_kxbtc_format(ticker: &str) -> Option<BtcMarketSpec> {
    let parts: Vec<&str> = ticker.split('-').collect();
    if parts.len() < 4 {
        return None;
    }

    // Check prefix
    if !parts[0].starts_with("KXBTC") {
        return None;
    }

    // Parse date: 26FEB04 -> Feb 4, 2026
    let date_str = parts[1];
    let date = parse_kalshi_date(date_str)?;

    // Parse time: T1200 -> 12:00
    let time_str = parts[2];
    if !time_str.starts_with('T') {
        return None;
    }
    let time = parse_kalshi_time(&time_str[1..])?;

    // Combine into datetime (Kalshi uses ET, convert to UTC)
    let expiry = combine_date_time_et_to_utc(date, time)?;

    // Parse strike: B97250 -> $97,250 (Above)
    let strike_part = parts[3];
    let (market_type, strike) = parse_strike_part(strike_part, parts.get(4).copied())?;

    Some(BtcMarketSpec {
        ticker: ticker.to_string(),
        strike,
        expiry,
        market_type,
    })
}

/// Parse simple format: BTC-26FEB04-97250
fn parse_simple_btc_format(ticker: &str) -> Option<BtcMarketSpec> {
    let parts: Vec<&str> = ticker.split('-').collect();
    if parts.len() < 3 {
        return None;
    }

    if parts[0] != "BTC" {
        return None;
    }

    // Parse date
    let date = parse_kalshi_date(parts[1])?;

    // Default to 4 PM ET (common settlement time)
    let time = NaiveTime::from_hms_opt(16, 0, 0)?;
    let expiry = combine_date_time_et_to_utc(date, time)?;

    // Parse strike (assume "above" by default)
    let strike: f64 = parts[2].parse().ok()?;

    Some(BtcMarketSpec {
        ticker: ticker.to_string(),
        strike,
        expiry,
        market_type: MarketType::Above,
    })
}

/// Parse Kalshi date format: 26FEB04 -> NaiveDate
fn parse_kalshi_date(s: &str) -> Option<NaiveDate> {
    if s.len() < 7 {
        return None;
    }

    let year_prefix = &s[0..2];
    let month_str = &s[2..5];
    let day_str = &s[5..];

    let year: i32 = format!("20{}", year_prefix).parse().ok()?;
    let month = match month_str.to_uppercase().as_str() {
        "JAN" => 1,
        "FEB" => 2,
        "MAR" => 3,
        "APR" => 4,
        "MAY" => 5,
        "JUN" => 6,
        "JUL" => 7,
        "AUG" => 8,
        "SEP" => 9,
        "OCT" => 10,
        "NOV" => 11,
        "DEC" => 12,
        _ => return None,
    };
    let day: u32 = day_str.parse().ok()?;

    NaiveDate::from_ymd_opt(year, month, day)
}

/// Parse Kalshi time format: 1200 -> NaiveTime (12:00)
fn parse_kalshi_time(s: &str) -> Option<NaiveTime> {
    if s.len() < 4 {
        return None;
    }

    let hour: u32 = s[0..2].parse().ok()?;
    let minute: u32 = s[2..4].parse().ok()?;

    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// Convert ET date/time to UTC datetime
fn combine_date_time_et_to_utc(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::new(date, time);

    // ET is UTC-5 (EST) or UTC-4 (EDT)
    // For simplicity, assume EST (UTC-5). A production system should use proper timezone handling.
    let et_offset_hours = 5; // EST offset
    let utc_naive = naive + chrono::Duration::hours(et_offset_hours);

    Some(Utc.from_utc_datetime(&utc_naive))
}

/// Parse strike part: B97250 or range 97250-97499
fn parse_strike_part(part: &str, next_part: Option<&str>) -> Option<(MarketType, f64)> {
    if part.starts_with('B') {
        // Above market: B97250
        let strike: f64 = part[1..].parse().ok()?;

        // Check for range
        if let Some(ceiling_str) = next_part {
            if let Ok(ceiling) = ceiling_str.parse::<u64>() {
                return Some((
                    MarketType::Range {
                        floor: strike as u64,
                        ceiling,
                    },
                    strike,
                ));
            }
        }

        Some((MarketType::Above, strike))
    } else if part.starts_with('A') {
        // Below market: A97250 (alternative notation)
        let strike: f64 = part[1..].parse().ok()?;
        Some((MarketType::Below, strike))
    } else {
        // Plain number, assume above
        let strike: f64 = part.parse().ok()?;
        Some((MarketType::Above, strike))
    }
}

// =============================================================================
// FAIR VALUE CALCULATION
// =============================================================================

/// Standard normal cumulative distribution function (CDF)
///
/// Uses the error function approximation for efficiency.
fn normal_cdf(x: f64) -> f64 {
    // Using the complementary error function relation:
    // Phi(x) = 0.5 * erfc(-x / sqrt(2))
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// Complementary error function (erfc) approximation
/// Uses Horner's method with coefficients from Abramowitz & Stegun
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);

    let ans = t
        * (-z * z - 1.26551223
            + t * (1.00002368
                + t * (0.37409196
                    + t * (0.09678418
                        + t * (-0.18628806
                            + t * (0.27886807
                                + t * (-1.13520398
                                    + t * (1.48851587
                                        + t * (-0.82215223 + t * 0.17087277)))))))))
        .exp();

    if x >= 0.0 {
        ans
    } else {
        2.0 - ans
    }
}

/// Calculate fair probability for a binary option.
///
/// For "above" markets (YES pays if S > K at expiry):
/// P(YES) = N(d2)
///
/// where d2 = [ln(S/K) + (r - σ²/2)T] / (σ√T)
///
/// # Arguments
/// * `spot` - Current BTC price
/// * `strike` - Strike price
/// * `time_to_expiry` - Time to expiry in years
/// * `volatility` - Annualized volatility (e.g., 0.50 for 50%)
/// * `risk_free_rate` - Risk-free rate (typically 0 for crypto)
///
/// # Returns
/// Fair probability that the market settles YES (0.0 to 1.0)
pub fn binary_option_fair_value(
    spot: f64,
    strike: f64,
    time_to_expiry: f64,
    volatility: f64,
    risk_free_rate: f64,
) -> f64 {
    // Edge cases
    if time_to_expiry <= 0.0 {
        // Already expired
        return if spot >= strike { 1.0 } else { 0.0 };
    }

    if volatility <= 0.0 {
        // No volatility = deterministic
        return if spot >= strike { 1.0 } else { 0.0 };
    }

    if strike <= 0.0 || spot <= 0.0 {
        return 0.5; // Invalid inputs
    }

    let sqrt_t = time_to_expiry.sqrt();
    let d2 = ((spot / strike).ln() + (risk_free_rate - 0.5 * volatility.powi(2)) * time_to_expiry)
        / (volatility * sqrt_t);

    normal_cdf(d2)
}

/// Fair value calculator that maintains state
#[derive(Debug, Clone)]
pub struct FairValueCalculator {
    /// Market specification (strike, expiry, etc.)
    pub market_spec: BtcMarketSpec,
    /// Volatility source
    pub vol_source: VolatilitySource,
    /// Risk-free rate (default 0 for crypto)
    pub risk_free_rate: f64,
    /// Last calculated fair probability
    pub last_fair_prob: Option<f64>,
    /// Last spot price used
    pub last_spot: Option<f64>,
}

impl FairValueCalculator {
    /// Create a new fair value calculator for a market
    pub fn new(market_spec: BtcMarketSpec) -> Self {
        Self {
            market_spec,
            vol_source: VolatilitySource::default(),
            risk_free_rate: 0.0,
            last_fair_prob: None,
            last_spot: None,
        }
    }

    /// Create from a ticker string
    pub fn from_ticker(ticker: &str) -> Option<Self> {
        parse_btc_ticker(ticker).map(Self::new)
    }

    /// Create with manual specification (for non-standard tickers)
    pub fn manual(ticker: &str, strike: f64, expiry: DateTime<Utc>, market_type: MarketType) -> Self {
        Self::new(BtcMarketSpec {
            ticker: ticker.to_string(),
            strike,
            expiry,
            market_type,
        })
    }

    /// Set volatility source
    pub fn with_volatility(mut self, vol: VolatilitySource) -> Self {
        self.vol_source = vol;
        self
    }

    /// Update volatility value (convenience method)
    pub fn set_volatility(&mut self, vol: f64) {
        self.vol_source = VolatilitySource::Constant(vol);
    }

    /// Calculate fair probability given current spot price
    ///
    /// # Arguments
    /// * `spot` - Current BTC mid price
    ///
    /// # Returns
    /// Fair probability for YES side (0.0 to 1.0)
    pub fn calculate(&mut self, spot: f64) -> f64 {
        let time_to_expiry = self.market_spec.time_to_expiry();
        let vol = self.vol_source.get_vol();

        let fair_prob = match self.market_spec.market_type {
            MarketType::Above => {
                binary_option_fair_value(
                    spot,
                    self.market_spec.strike,
                    time_to_expiry,
                    vol,
                    self.risk_free_rate,
                )
            }
            MarketType::Below => {
                // Below is just 1 - Above
                1.0 - binary_option_fair_value(
                    spot,
                    self.market_spec.strike,
                    time_to_expiry,
                    vol,
                    self.risk_free_rate,
                )
            }
            MarketType::Range { floor, ceiling } => {
                // P(floor <= S < ceiling) = P(S >= floor) - P(S >= ceiling)
                let p_above_floor = binary_option_fair_value(
                    spot,
                    floor as f64,
                    time_to_expiry,
                    vol,
                    self.risk_free_rate,
                );
                let p_above_ceiling = binary_option_fair_value(
                    spot,
                    ceiling as f64,
                    time_to_expiry,
                    vol,
                    self.risk_free_rate,
                );
                p_above_floor - p_above_ceiling
            }
        };

        self.last_fair_prob = Some(fair_prob);
        self.last_spot = Some(spot);

        debug!(
            "[FAIR] {} | spot=${:.2} | strike=${:.0} | T={:.2}h | vol={:.1}% | fair={:.1}%",
            self.market_spec.ticker,
            spot,
            self.market_spec.strike,
            self.market_spec.time_to_expiry_hours(),
            vol * 100.0,
            fair_prob * 100.0
        );

        fair_prob
    }

    /// Get last calculated fair probability
    pub fn fair_prob(&self) -> Option<f64> {
        self.last_fair_prob
    }

    /// Calculate fair probability for NO side
    pub fn fair_prob_no(&self) -> Option<f64> {
        self.last_fair_prob.map(|p| 1.0 - p)
    }
}

// =============================================================================
// SENSITIVITY CALCULATIONS (GREEKS)
// =============================================================================

/// Calculate delta (sensitivity to spot price change)
/// This is the first derivative of fair_prob with respect to spot
pub fn calculate_delta(spot: f64, strike: f64, time_to_expiry: f64, volatility: f64) -> f64 {
    if time_to_expiry <= 0.0 || volatility <= 0.0 {
        return 0.0;
    }

    let sqrt_t = time_to_expiry.sqrt();
    let d2 = ((spot / strike).ln() + (-0.5 * volatility.powi(2)) * time_to_expiry)
        / (volatility * sqrt_t);

    // d(Phi(d2))/dS = phi(d2) * (1 / (S * vol * sqrt(T)))
    let phi_d2 = normal_pdf(d2);
    phi_d2 / (spot * volatility * sqrt_t)
}

/// Standard normal probability density function
fn normal_pdf(x: f64) -> f64 {
    (-0.5 * x.powi(2)).exp() / (2.0 * PI).sqrt()
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_normal_cdf() {
        // Known values
        assert!((normal_cdf(0.0) - 0.5).abs() < 0.0001);
        assert!((normal_cdf(1.96) - 0.975).abs() < 0.001);
        assert!((normal_cdf(-1.96) - 0.025).abs() < 0.001);
    }

    #[test]
    fn test_binary_fair_value_at_the_money() {
        // At the money with moderate vol, should be close to 50%
        let fair = binary_option_fair_value(
            100_000.0, // spot
            100_000.0, // strike
            1.0 / 365.0, // 1 day to expiry
            0.50,      // 50% vol
            0.0,       // no risk-free rate
        );
        // Should be slightly less than 50% due to drift term
        assert!(fair > 0.40 && fair < 0.55);
    }

    #[test]
    fn test_binary_fair_value_deep_itm() {
        // Deep in the money, should be close to 100%
        let fair = binary_option_fair_value(
            100_000.0, // spot
            90_000.0,  // strike (below spot)
            1.0 / 365.0,
            0.50,
            0.0,
        );
        assert!(fair > 0.90);
    }

    #[test]
    fn test_binary_fair_value_deep_otm() {
        // Deep out of the money, should be close to 0%
        let fair = binary_option_fair_value(
            100_000.0, // spot
            110_000.0, // strike (above spot)
            1.0 / 365.0,
            0.50,
            0.0,
        );
        assert!(fair < 0.10);
    }

    #[test]
    fn test_binary_fair_value_expired() {
        // Expired - should return 1 or 0
        let fair_itm = binary_option_fair_value(100_000.0, 90_000.0, 0.0, 0.50, 0.0);
        assert_eq!(fair_itm, 1.0);

        let fair_otm = binary_option_fair_value(100_000.0, 110_000.0, 0.0, 0.50, 0.0);
        assert_eq!(fair_otm, 0.0);
    }

    #[test]
    fn test_parse_kalshi_date() {
        let date = parse_kalshi_date("26FEB04").unwrap();
        assert_eq!(date.year(), 2026);
        assert_eq!(date.month(), 2);
        assert_eq!(date.day(), 4);
    }

    #[test]
    fn test_parse_kalshi_time() {
        let time = parse_kalshi_time("1230").unwrap();
        assert_eq!(time.hour(), 12);
        assert_eq!(time.minute(), 30);
    }
}
