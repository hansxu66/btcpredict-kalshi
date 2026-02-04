"""Crypto market discovery utilities for 15-minute markets."""
import csv
import time
from datetime import datetime
from typing import Optional
from discovery.helper.discovery import MarketDiscovery
from discovery.helper.client import init_kalshi_client


# 15-minute crypto market series
CRYPTO_SERIES = {
    "KXBTC15M": "Bitcoin",
    "KXETH15M": "Ethereum",
    "KXXRP15M": "XRP",
}

# Keywords for fallback discovery
CRYPTO_KEYWORDS = ["BTC", "ETH", "XRP", "CRYPTO", "BITCOIN", "ETHEREUM"]


def get_all_crypto_markets(
    discovery: MarketDiscovery,
    output_path: str = "crypto_markets.csv",
    status: Optional[list[str]] = None,
    crypto_filter: Optional[list[str]] = None,
    sleep_seconds: float = 0.1,
) -> None:
    """
    Get all 15-minute crypto markets and save to CSV.

    Args:
        discovery: MarketDiscovery instance
        output_path: Path to save CSV
        status: Filter by market status (e.g., ["open", "closed", "settled"])
        crypto_filter: Filter to specific cryptos (e.g., ["BTC", "ETH"])
        sleep_seconds: Delay between API calls to avoid rate limiting
    """
    # Step 1: Get crypto series (direct + keyword matching)
    print("==============================================================")
    print("[Step 1] Getting crypto series...")

    # Direct series lookup
    all_series = discovery.get_series_list()

    # Filter for 15-minute crypto series
    crypto_series = [
        s for s in all_series
        if s["ticker"] in CRYPTO_SERIES or
        any(kw in s["ticker"].upper() for kw in CRYPTO_KEYWORDS)
    ]
    print(f"  Found {len(crypto_series)} crypto series")

    # Optional: filter by specific cryptos
    if crypto_filter:
        crypto_series = [
            s for s in crypto_series
            if any(cf.upper() in s["ticker"].upper() for cf in crypto_filter)
        ]
        print(f"  After filtering by {crypto_filter}: {len(crypto_series)}")

    print("==============================================================")

    # Step 2/3/4: Fetch events/markets and stream to CSV
    print("[Step 2] Getting events and markets for each series...")
    total_events = 0
    total_markets = 0
    error_log: list[str] = []

    with open(output_path, "w", newline="", encoding="utf-8") as csv_file:
        writer = csv.DictWriter(
            csv_file,
            fieldnames=[
                "series_ticker",
                "event_ticker",
                "market_ticker",
                "market_type",
                "title",
                "open_time",
                "close_time",
                "volume",
                "volume_24h",
                "result",
                "open_interest",
                "yes_bid",
                "yes_ask",
                "last_price",
            ],
        )
        writer.writeheader()

        for i, series in enumerate(crypto_series):
            series_ticker = series["ticker"]
            print(f"  [{i+1}/{len(crypto_series)}] Getting events for {series_ticker}...")

            try:
                events = discovery.get_events(series_ticker=series_ticker, status=status)
                total_events += len(events)
                print(f"    Found {len(events)} events")
            except Exception as e:
                error_log.append(f"events:{series_ticker}:{e}")
                time.sleep(sleep_seconds)
                continue

            for event in events:
                event_ticker = event.event_ticker if hasattr(event, "event_ticker") else event.get("event_ticker")
                retries = 3
                markets = None
                last_error = None
                for attempt in range(1, retries + 1):
                    try:
                        markets = discovery.get_markets(event_ticker=event_ticker, status=status)
                        total_markets += len(markets)
                        break
                    except Exception as e:
                        last_error = e
                        time.sleep(1)
                if markets is None:
                    if last_error is not None:
                        error_log.append(f"markets:event={event_ticker}:failed_after_{retries}:{last_error}")
                    continue

                for m in markets:
                    writer.writerow({
                        "series_ticker": series_ticker,
                        "event_ticker": event_ticker,
                        "market_ticker": m.ticker,
                        "market_type": m.market_type,
                        "title": m.title,
                        "open_time": m.open_time,
                        "close_time": m.close_time,
                        "volume": m.volume,
                        "volume_24h": m.volume_24h,
                        "result": m.result,
                        "open_interest": m.open_interest,
                        "yes_bid": getattr(m, "yes_bid", None),
                        "yes_ask": getattr(m, "yes_ask", None),
                        "last_price": getattr(m, "last_price", None),
                    })

            time.sleep(sleep_seconds)

    print(f"  Total events: {total_events}")
    print(f"  Total markets: {total_markets}")
    print(f"Saved {total_markets} crypto markets to {output_path}")
    if error_log:
        error_log_path = f"{output_path}_error_log.csv"
        with open(error_log_path, "w", encoding="utf-8") as log_file:
            for entry in error_log:
                log_file.write(f"{entry}\n")
        print("Errors:")
        for entry in error_log:
            print(entry)
        print(f"Wrote error log to {error_log_path}")


def get_15m_crypto_markets(
    discovery: MarketDiscovery,
    output_path: str = "crypto_15m_markets.csv",
    status: Optional[list[str]] = None,
    cryptos: Optional[list[str]] = None,
    sleep_seconds: float = 0.1,
) -> None:
    """
    Get specifically 15-minute crypto markets (KXBTC15M, KXETH15M, KXXRP15M).

    Args:
        discovery: MarketDiscovery instance
        output_path: Path to save CSV
        status: Filter by market status
        cryptos: Specific cryptos to fetch (e.g., ["BTC", "ETH", "XRP"])
        sleep_seconds: Delay between API calls
    """
    print("==============================================================")
    print("[Step 1] Getting 15-minute crypto series...")

    # Map crypto names to series tickers
    crypto_to_series = {
        "BTC": "KXBTC15M",
        "ETH": "KXETH15M",
        "XRP": "KXXRP15M",
    }

    if cryptos:
        target_series = [crypto_to_series[c.upper()] for c in cryptos if c.upper() in crypto_to_series]
    else:
        target_series = list(crypto_to_series.values())

    print(f"  Target series: {target_series}")
    print("==============================================================")

    # Step 2: Fetch events/markets
    print("[Step 2] Getting events and markets...")
    total_events = 0
    total_markets = 0
    error_log: list[str] = []

    with open(output_path, "w", newline="", encoding="utf-8") as csv_file:
        writer = csv.DictWriter(
            csv_file,
            fieldnames=[
                "series_ticker",
                "crypto",
                "event_ticker",
                "market_ticker",
                "market_type",
                "title",
                "strike_price",
                "open_time",
                "close_time",
                "volume",
                "volume_24h",
                "result",
                "open_interest",
                "yes_bid",
                "yes_ask",
                "last_price",
            ],
        )
        writer.writeheader()

        for series_ticker in target_series:
            crypto = series_ticker.replace("KX", "").replace("15M", "")
            print(f"  Getting events for {series_ticker} ({crypto})...")

            try:
                events = discovery.get_events(series_ticker=series_ticker, status=status)
                total_events += len(events)
                print(f"    Found {len(events)} events")
            except Exception as e:
                error_log.append(f"events:{series_ticker}:{e}")
                time.sleep(sleep_seconds)
                continue

            for event in events:
                event_ticker = event.event_ticker if hasattr(event, "event_ticker") else event.get("event_ticker")
                retries = 3
                markets = None
                last_error = None
                for attempt in range(1, retries + 1):
                    try:
                        markets = discovery.get_markets(event_ticker=event_ticker, status=status)
                        total_markets += len(markets)
                        break
                    except Exception as e:
                        last_error = e
                        time.sleep(1)
                if markets is None:
                    if last_error is not None:
                        error_log.append(f"markets:event={event_ticker}:failed_after_{retries}:{last_error}")
                    continue

                for m in markets:
                    # Extract strike price from title if available
                    strike_price = None
                    title = m.title or ""
                    if "$" in title:
                        import re
                        price_match = re.search(r'\$([0-9,]+(?:\.\d+)?)', title)
                        if price_match:
                            strike_price = price_match.group(1).replace(",", "")

                    writer.writerow({
                        "series_ticker": series_ticker,
                        "crypto": crypto,
                        "event_ticker": event_ticker,
                        "market_ticker": m.ticker,
                        "market_type": m.market_type,
                        "title": title,
                        "strike_price": strike_price,
                        "open_time": m.open_time,
                        "close_time": m.close_time,
                        "volume": m.volume,
                        "volume_24h": m.volume_24h,
                        "result": m.result,
                        "open_interest": m.open_interest,
                        "yes_bid": getattr(m, "yes_bid", None),
                        "yes_ask": getattr(m, "yes_ask", None),
                        "last_price": getattr(m, "last_price", None),
                    })

            time.sleep(sleep_seconds)

    print(f"  Total events: {total_events}")
    print(f"  Total markets: {total_markets}")
    print(f"Saved {total_markets} 15-minute crypto markets to {output_path}")
    if error_log:
        error_log_path = f"{output_path}_error_log.csv"
        with open(error_log_path, "w", encoding="utf-8") as log_file:
            for entry in error_log:
                log_file.write(f"{entry}\n")
        print(f"Wrote error log to {error_log_path}")


if __name__ == "__main__":
    client = init_kalshi_client()
    discovery = MarketDiscovery(client)
    date_str = datetime.now().strftime("%m_%d_%Y")

    # Get all 15-minute crypto markets (BTC, ETH, XRP)
    get_15m_crypto_markets(
        discovery,
        output_path=f"discovery/crypto/crypto_15m_open_{date_str}.csv",
        status=["open"]
    )
    # Uncomment to get settled markets
    # get_15m_crypto_markets(
    #     discovery,
    #     output_path=f"discovery/crypto/crypto_15m_settled_{date_str}.csv",
    #     status=["settled"]
    # )
