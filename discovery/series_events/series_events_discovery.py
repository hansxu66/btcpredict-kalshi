"""
Kalshi market discovery utilities.

Functions for exploring and exporting Kalshi market data:
- find_series_list: Export all series to series_list.csv
- find_categories_list: Export all categories to series_categories.csv
- find_events_test: List events for a given series ticker
- find_markets_test: List markets for a given event ticker
"""
import json
import csv
from discovery.helper.client import init_kalshi_client
from discovery.helper.discovery import MarketDiscovery

def find_series_list():
    client = init_kalshi_client()
    discovery = MarketDiscovery(client)
    series_list = discovery.get_series_list()
    print(f"Series count: {len(series_list)}")
    with open("series_list.csv", "w", newline="", encoding="utf-8") as csv_file:
        fieldnames = sorted({key for series in series_list for key in series.keys()})
        writer = csv.DictWriter(csv_file, fieldnames=fieldnames)
        writer.writeheader()
        for series in series_list:
            row = {}
            for key in fieldnames:
                value = series.get(key, "")
                if isinstance(value, (list, dict)):
                    value = json.dumps(value, ensure_ascii=True)
                row[key] = value
            writer.writerow(row)
    print("Wrote series_list.csv to the disk with all series on Kalshi")

def find_categories_list():
    client = init_kalshi_client()
    discovery = MarketDiscovery(client)
    categories = discovery.get_series_categories()
    print(f"Category count: {len(categories)}")
    with open("series_categories.csv", "w", newline="", encoding="utf-8") as csv_file:
        writer = csv.writer(csv_file)
        writer.writerow(["category", "tag"])
        for category, tags in categories.items():
            for tag in (tags or []):
                writer.writerow([category, tag])
    print("Wrote series_categories.csv")

def find_events_test(series_ticker):
    client = init_kalshi_client()
    discovery = MarketDiscovery(client)
    events = discovery.get_events(series_ticker=series_ticker)
    print(f"Events count: {len(events)}")
    for event in events:
        print(f"{event.event_ticker} | {event.title} | {event.sub_title}")

def find_markets_test(event_ticker):
    client = init_kalshi_client()
    discovery = MarketDiscovery(client)
    markets = discovery.get_markets(event_ticker=event_ticker)
    print(f"Markets count: {len(markets)}")
    for market in markets:
        for key, value in getattr(market, "__dict__", {}).items():
            print(f"{key}: {value}")
        print("---")

if __name__ == "__main__":
    find_series_list()
    find_categories_list()

    series_ticker = "KXNFLTOTAL"
    find_events_test(series_ticker)

    events_ticker = "KXNFLTOTAL-25SEP28MINPIT"
    find_markets_test(events_ticker)