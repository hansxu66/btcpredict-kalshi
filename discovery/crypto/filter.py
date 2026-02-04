"""
Filter 15-minute crypto markets by cryptocurrency type.

Supports: BTC (Bitcoin), ETH (Ethereum), XRP.
"""
import re
from datetime import datetime
from typing import Optional
import pandas as pd


# =============================================================================
# CRYPTO SERIES DEFINITIONS
# =============================================================================

CRYPTO_15M_SERIES = {
    "KXBTC15M": "Bitcoin",
    "KXETH15M": "Ethereum",
    "KXXRP15M": "XRP",
}

# Mapping from ticker prefix to crypto name
CRYPTO_NAMES = {
    "BTC": "Bitcoin",
    "ETH": "Ethereum",
    "XRP": "XRP",
}


# =============================================================================
# PARSING HELPERS
# =============================================================================

def parse_crypto_event_ticker(event_ticker: str) -> dict:
    """
    Parse crypto event ticker to extract timestamp and details.

    Format examples:
        KXBTC15M-26FEB04T1200 (date and time)
        KXETH15M-26FEB04T1215

    Returns:
        Dict with crypto, event_datetime, year, month, day, time
    """
    # Match pattern: KXCRYPTO15M-YYMONDDTHHMM
    pattern = r'KX([A-Z]+)15M-(\d{2})([A-Z]{3})(\d{2})T(\d{4})'
    match = re.match(pattern, event_ticker)

    if not match:
        return {
            "crypto": None,
            "event_datetime": None,
            "year": None,
            "month": None,
            "day": None,
            "time": None,
        }

    crypto = match.group(1)
    year = int(match.group(2)) + 2000
    month_str = match.group(3)
    day = int(match.group(4))
    time_str = match.group(5)

    months = {
        "JAN": 1, "FEB": 2, "MAR": 3, "APR": 4, "MAY": 5, "JUN": 6,
        "JUL": 7, "AUG": 8, "SEP": 9, "OCT": 10, "NOV": 11, "DEC": 12
    }
    month = months.get(month_str, 1)

    hour = int(time_str[:2])
    minute = int(time_str[2:])

    try:
        event_datetime = datetime(year, month, day, hour, minute)
    except ValueError:
        event_datetime = None

    return {
        "crypto": crypto,
        "event_datetime": event_datetime,
        "year": year,
        "month": month,
        "day": day,
        "time": time_str,
    }


def parse_strike_from_title(title: str) -> Optional[float]:
    """
    Extract strike price from market title.

    Examples:
        "Bitcoin above $100,000?" -> 100000.0
        "ETH above $3,500.50?" -> 3500.50
    """
    if not title:
        return None

    price_match = re.search(r'\$([0-9,]+(?:\.\d+)?)', title)
    if price_match:
        price_str = price_match.group(1).replace(",", "")
        try:
            return float(price_str)
        except ValueError:
            return None
    return None


def parse_direction_from_title(title: str) -> Optional[str]:
    """
    Extract direction (above/below) from market title.

    Examples:
        "Bitcoin above $100,000?" -> "above"
        "ETH below $3,500?" -> "below"
    """
    if not title:
        return None

    title_lower = title.lower()
    if "above" in title_lower:
        return "above"
    elif "below" in title_lower:
        return "below"
    return None


# =============================================================================
# CORE FILTER FUNCTION
# =============================================================================

def filter_crypto_markets(
    series_tickers: list[str],
    input_path: str = "discovery/crypto/crypto_15m_markets.csv",
    output_path: Optional[str] = None,
    parse_events: bool = True,
) -> pd.DataFrame:
    """
    Filter crypto markets by series ticker(s).

    Args:
        series_tickers: List of series tickers to include
        input_path: Path to input CSV
        output_path: Path for output CSV (optional)
        parse_events: Whether to parse event tickers

    Returns:
        Filtered DataFrame
    """
    df = pd.read_csv(input_path)

    # Filter to specified series
    mask = df["series_ticker"].isin(series_tickers)
    filtered = df[mask].copy()

    print(f"Found {len(filtered)} markets")
    print(f"Covering {filtered['event_ticker'].nunique()} events")

    # Parse event tickers
    if parse_events:
        parsed = filtered["event_ticker"].apply(parse_crypto_event_ticker).apply(pd.Series)
        filtered = pd.concat([filtered, parsed], axis=1)

    # Parse strike price and direction from title
    filtered["strike_price"] = filtered["title"].apply(parse_strike_from_title)
    filtered["direction"] = filtered["title"].apply(parse_direction_from_title)

    # Determine outcome
    filtered["is_yes"] = filtered["result"] == "yes"

    # Reorder columns
    base_cols = ["series_ticker", "crypto", "event_ticker", "market_ticker"]
    parsed_cols = ["event_datetime", "time"] if parse_events else []
    market_cols = ["strike_price", "direction", "is_yes", "title",
                   "volume", "volume_24h", "open_interest",
                   "yes_bid", "yes_ask", "last_price",
                   "open_time", "close_time", "result"]
    columns = base_cols + parsed_cols + market_cols
    columns = [c for c in columns if c in filtered.columns]
    filtered = filtered[columns]

    # Sort by datetime
    if "event_datetime" in filtered.columns:
        filtered["_sort_dt"] = pd.to_datetime(filtered["event_datetime"], errors="coerce")
        filtered = filtered.sort_values(["_sort_dt", "strike_price"])
        filtered = filtered.drop(columns=["_sort_dt"])
    else:
        filtered = filtered.sort_values(["event_ticker", "strike_price"])

    # Save if output path provided
    if output_path:
        filtered.to_csv(output_path, index=False)
        print(f"Saved to {output_path}")

    # Summary
    if "event_datetime" in filtered.columns:
        valid_dates = filtered["event_datetime"].dropna()
        if len(valid_dates) > 0:
            dates = pd.to_datetime(valid_dates)
            print(f"Date range: {dates.min()} to {dates.max()}")
    print(f"Total volume: {filtered['volume'].sum():,}")

    return filtered


# =============================================================================
# CRYPTO-SPECIFIC FUNCTIONS
# =============================================================================

def filter_btc(
    input_path: str = "discovery/crypto/crypto_15m_markets.csv",
    output_path: str = "discovery/crypto/btc_15m.csv",
) -> pd.DataFrame:
    """Filter for Bitcoin 15-minute markets."""
    print("=== BTC 15-Minute Markets ===")
    return filter_crypto_markets(
        series_tickers=["KXBTC15M"],
        input_path=input_path,
        output_path=output_path,
    )


def filter_eth(
    input_path: str = "discovery/crypto/crypto_15m_markets.csv",
    output_path: str = "discovery/crypto/eth_15m.csv",
) -> pd.DataFrame:
    """Filter for Ethereum 15-minute markets."""
    print("=== ETH 15-Minute Markets ===")
    return filter_crypto_markets(
        series_tickers=["KXETH15M"],
        input_path=input_path,
        output_path=output_path,
    )


def filter_xrp(
    input_path: str = "discovery/crypto/crypto_15m_markets.csv",
    output_path: str = "discovery/crypto/xrp_15m.csv",
) -> pd.DataFrame:
    """Filter for XRP 15-minute markets."""
    print("=== XRP 15-Minute Markets ===")
    return filter_crypto_markets(
        series_tickers=["KXXRP15M"],
        input_path=input_path,
        output_path=output_path,
    )


def filter_all(
    input_path: str = "discovery/crypto/crypto_15m_markets.csv",
) -> dict[str, pd.DataFrame]:
    """
    Filter all supported cryptocurrencies and return dict of DataFrames.

    Returns:
        Dict with keys: 'btc', 'eth', 'xrp'
    """
    return {
        "btc": filter_btc(input_path),
        "eth": filter_eth(input_path),
        "xrp": filter_xrp(input_path),
    }


# =============================================================================
# ADDITIONAL UTILITIES
# =============================================================================

def get_market_summary(df: pd.DataFrame) -> pd.DataFrame:
    """
    Get summary statistics for crypto markets.

    Args:
        df: DataFrame from filter functions

    Returns:
        Summary DataFrame with stats per crypto and time slot
    """
    if "crypto" not in df.columns:
        print("Warning: 'crypto' column not found")
        return df

    summary = df.groupby(["crypto", "time"]).agg({
        "market_ticker": "count",
        "volume": "sum",
        "open_interest": "sum",
    }).rename(columns={
        "market_ticker": "num_markets",
        "volume": "total_volume",
        "open_interest": "total_oi",
    })

    return summary.reset_index()


def get_strike_distribution(df: pd.DataFrame) -> pd.DataFrame:
    """
    Get distribution of strike prices for analysis.

    Args:
        df: DataFrame from filter functions

    Returns:
        DataFrame with strike price statistics
    """
    if "strike_price" not in df.columns or "crypto" not in df.columns:
        print("Warning: required columns not found")
        return df

    stats = df.groupby("crypto")["strike_price"].describe()
    return stats


# =============================================================================
# CLI
# =============================================================================

if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        print("Usage: python filter.py <crypto>")
        print("Cryptos: btc, eth, xrp, all")
        sys.exit(1)

    crypto = sys.argv[1].lower()

    if crypto == "btc":
        df = filter_btc()
    elif crypto == "eth":
        df = filter_eth()
    elif crypto == "xrp":
        df = filter_xrp()
    elif crypto == "all":
        filter_all()
    else:
        print(f"Unknown crypto: {crypto}")
        sys.exit(1)
