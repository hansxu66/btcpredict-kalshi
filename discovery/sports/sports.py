"""Sports market discovery utilities."""
import csv
import time
from datetime import datetime
from typing import Optional
from discovery.helper.discovery import MarketDiscovery
from discovery.helper.client import init_kalshi_client


SPORTS_KEYWORDS = [
    "NFL", "NBA", "MLB", "NHL", "MLS", "UFC", "PGA", "ATP", "NCAA",
    "EPL", "FIFA", "WNBA", "MMA", "GOLF", "TENNIS", "SOCCER", "HOCKEY",
    "BASEBALL", "BASKETBALL", "FOOTBALL",
]


def get_all_sports_markets(
    discovery: MarketDiscovery,
    output_path: str = "sports_markets.csv",
    status: Optional[list[str]] = None,
    sports_tags: Optional[list[str]] = None,
    sleep_seconds: float = 0.1,
) -> None:
    """
    Get all sports markets and save to CSV.

    Args:
        discovery: MarketDiscovery instance
        output_path: Path to save CSV
        status: Filter by market status (e.g., ["open", "closed", "settled"])
        sports_tags: Filter to specific sports (e.g., ["NFL", "NBA"])
        sleep_seconds: Delay between API calls to avoid rate limiting
    """
    # Step 1: Get sports series (robust - category + keyword matching)
    print("==============================================================")
    print("[Step 1] Getting sports series...")

    series_by_category = discovery.get_series_list(category="Sports")
    print(f"  Found {len(series_by_category)} series by category")

    all_series = discovery.get_series_list()
    series_by_keyword = [
        s for s in all_series
        if any(kw in s["ticker"].upper() for kw in SPORTS_KEYWORDS)
    ]
    print(f"  Found {len(series_by_keyword)} series by keyword")

    # Combine and dedupe by ticker
    seen_tickers = set()
    sports_series = []
    for s in series_by_category + series_by_keyword:
        if s["ticker"] not in seen_tickers:
            seen_tickers.add(s["ticker"])
            sports_series.append(s)
    print(f"  Total unique sports series: {len(sports_series)}")

    # Optional: filter by specific sports tags (if you only want NBA)
    if sports_tags:
        sports_series = [
            s for s in sports_series
            if any(tag.upper() in s["ticker"].upper() for tag in sports_tags)
        ]
        print(f"  After filtering by tags {sports_tags}: {len(sports_series)}")

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
            ],
        )
        writer.writeheader()

        for i, series in enumerate(sports_series):
            series_ticker = series["ticker"]
            #print(f"  [{i+1}/{len(sports_series)}] Getting events for {series_ticker}...")

            try:
                events = discovery.get_events(series_ticker=series_ticker, status=status)
                total_events += len(events)
                print(f"Found {len(events)} events")
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
                    })

            time.sleep(sleep_seconds)

    print(f"  Total events: {total_events}")
    print(f"  Total markets: {total_markets}")
    print(f"Saved {total_markets} sports markets to {output_path}")
    if error_log:
        error_log_path = f"{output_path}_error_log.csv"
        with open(error_log_path, "w", encoding="utf-8") as log_file:
            for entry in error_log:
                log_file.write(f"{entry}\n")
        print("Errors:")
        for entry in error_log:
            print(entry)
        print(f"Wrote error log to {error_log_path}")


if __name__ == "__main__":

    client = init_kalshi_client()
    discovery = MarketDiscovery(client)
    date_str = datetime.now().strftime("%m_%d_%Y")
    get_all_sports_markets(discovery, output_path=f"sports_markets_open_{date_str}.csv", status=["open"])
    # get_all_sports_markets(discovery, output_path=f"sports_markets_settled_{date_str}.csv", status=["settled"])
