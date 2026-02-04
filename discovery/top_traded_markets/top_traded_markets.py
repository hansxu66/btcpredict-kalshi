"""
Top traded markets utility.
Fetches open markets and ranks by 24h volume.
"""
from __future__ import annotations

import csv
import json
import heapq
import time
from typing import Any

from discovery.helper.client import init_kalshi_client


def _extract_payload(response: Any) -> dict:
    raw = response.data if hasattr(response, "data") else response.read()
    if isinstance(raw, (bytes, bytearray)):
        raw = raw.decode("utf-8")
    return json.loads(raw)


def parse_markets(markets: list[dict]) -> list[dict]:
    """
    Parse market dicts into a compact row format.

    Args:
        markets: Raw market dictionaries

    Returns:
        List of parsed market rows
    """
    rows = []
    for m in markets:
        rows.append({
            "market_ticker": m.get("ticker"),
            "market_type": m.get("market_type"),
            "title": m.get("title"),
            "open_time": m.get("open_time"),
            "close_time": m.get("close_time"),
            "volume": m.get("volume"),
            "volume_24h": m.get("volume_24h"),
            "result": m.get("result"),
            "open_interest": m.get("open_interest"),
        })
    return rows


def get_top_traded_markets(
    status: str,
    top_n: int = 1000,
    output_path_volume: str | None = None,
    output_path_volume_24h: str | None = None,
) -> tuple[list[dict], list[dict], int]:
    """
    Track top traded markets by volume and 24h volume and write to CSV.

    Args:
        top_n: Number of top rows to keep per metric
        output_path_volume: CSV path for top markets by total volume
        output_path_volume_24h: CSV path for top markets by 24h volume

    Returns:
        Tuple of (top_volume_rows, top_volume_24h_rows, total market count)
    """
    if output_path_volume is None:
        output_path_volume = f"top_traded_markets_{status}_volume.csv"
    if output_path_volume_24h is None:
        output_path_volume_24h = f"top_traded_markets_{status}_volume_24h.csv"

    client = init_kalshi_client()
    cursor = None
    total_count = 0
    error_log: list[dict] = []
    row_fields = [
        "market_ticker",
        "market_type",
        "title",
        "open_time",
        "close_time",
        "volume",
        "volume_24h",
        "result",
        "open_interest",
    ]

    top_volume: list[tuple[float, int, dict]] = []
    top_volume_24h: list[tuple[float, int, dict]] = []
    counter = 0
    i = 1
    while True:
        payload = None
        for attempt in range(1, 4):
            try:
                response = client._market_api.get_markets_without_preload_content(
                    status=status,
                    limit=1000,
                    cursor=cursor,
                )
                payload = _extract_payload(response)
                break
            except Exception as exc:
                error_log.append({
                    "timestamp": time.time(),
                    "status": status,
                    "cursor": cursor,
                    "page": i,
                    "attempt": attempt,
                    "error": str(exc),
                })
                time.sleep(1)
        if payload is None:
            break

        for market in payload.get("markets", []):
            row = parse_markets([market])[0]
            total_count += 1
            counter += 1

            volume = row.get("volume") or 0
            volume_24h = row.get("volume_24h") or 0

            if len(top_volume) < top_n:
                heapq.heappush(top_volume, (volume, counter, row))
            elif volume > top_volume[0][0]:
                heapq.heapreplace(top_volume, (volume, counter, row))

            if len(top_volume_24h) < top_n:
                heapq.heappush(top_volume_24h, (volume_24h, counter, row))
            elif volume_24h > top_volume_24h[0][0]:
                heapq.heapreplace(top_volume_24h, (volume_24h, counter, row))

        cursor = payload.get("cursor")
        if not cursor:
            break
        print(f"[API] Page {i} is finished...")
        time.sleep(0.2)
        i += 1

    top_volume_rows = [item[2] for item in sorted(top_volume, key=lambda x: x[0], reverse=True)]
    top_volume_24h_rows = [item[2] for item in sorted(top_volume_24h, key=lambda x: x[0], reverse=True)]

    with open(output_path_volume, "w", newline="", encoding="utf-8") as csv_file:
        writer = csv.DictWriter(csv_file, fieldnames=row_fields)
        writer.writeheader()
        writer.writerows(top_volume_rows)

    with open(output_path_volume_24h, "w", newline="", encoding="utf-8") as csv_file:
        writer = csv.DictWriter(csv_file, fieldnames=row_fields)
        writer.writeheader()
        writer.writerows(top_volume_24h_rows)

    if error_log:
        error_log_path = f"{output_path_volume}_error_log.csv"
        with open(error_log_path, "w", newline="", encoding="utf-8") as csv_file:
            writer = csv.DictWriter(
                csv_file,
                fieldnames=["timestamp", "status", "cursor", "page", "attempt", "error"],
            )
            writer.writeheader()
            writer.writerows(error_log)

    return top_volume_rows, top_volume_24h_rows, total_count


def get_top_main(status) -> None:
    _, _, total_count = get_top_traded_markets(status)
    print(f"Total {status} markets: {total_count}")


if __name__ == "__main__":
    # get_top_main("open")
    get_top_main("settled")
