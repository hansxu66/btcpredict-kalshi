"""
Market discovery module.
Handles finding and filtering markets by time range, status, and sport.
"""
import json
import sys
from typing import Optional, Callable, Iterable

from kalshi_python_sync import KalshiClient


class MarketDiscovery:
    """Discover markets within a time range, optionally filtered by sport."""

    def __init__(self, client: KalshiClient):
        self.client = client

    def _paginate(
        self,
        fetch_fn: Callable[[Optional[str]], object],
        extract_fn: Callable[[object], list],
    ) -> list:
        """
        Generic pagination helper - fetches ALL pages.

        Args:
            fetch_fn: Function that takes cursor and returns response
            extract_fn: Function that extracts items list from response

        Returns:
            All items from all pages
        """
        all_items = []
        cursor = None

        while True:
            response = fetch_fn(cursor)
            items = extract_fn(response)
            all_items.extend(items)

            cursor = response.cursor
            if not cursor:
                break

        return all_items


    def get_markets(
        self,
        event_ticker: str,
        status: Optional[list[str]] = None,
    ) -> list:
        """
        Get all markets for a specific event.

        Args:
            event_ticker: Event ticker (e.g., "NFL-KC-BUF-2026-01-04")
            status: Optional market status filter

        Returns:
            List of market objects

        Here are the fields of market:
            'ticker',
            'event_ticker',
            'market_type',
            'title',
            'subtitle',
            'yes_sub_title',
            'no_sub_title',
            'created_time',
            'open_time',
            'close_time',
            'expected_expiration_time',
            'expiration_time',
            'latest_expiration_time',
            'settlement_timer_seconds',
            'status',
            'response_price_units',
            'yes_bid',
            'yes_bid_dollars',
            'yes_ask',
            'yes_ask_dollars',
            'no_bid',
            'no_bid_dollars',
            'no_ask',
            'no_ask_dollars',
            'last_price',
            'last_price_dollars',
            'voindex.csvlume',
            'volume_24h',
            'result',
            'can_close_early',
            'open_interest',
            'notional_value',
            'notional_value_dollars',
            'previous_yes_bid',
            'previous_yes_bid_dollars',
            'previous_yes_ask',
            'previous_yes_ask_dollars',
            'previous_price',
            'previous_price_dollars',
            'liquidity',
            'liquidity_dollars',
            'settlement_value',
            'settlement_value_dollars',
            'settlement_ts',
            'expiration_value',
            'category',
            'risk_limit_cents',
            'fee_waiver_expiration_time',
            'early_close_condition',
            'tick_size',
            'strike_type',
            'floor_strike',
            'cap_strike',
            'functional_strike',
            'custom_strike',
            'rules_primary',
            'rules_secondary',
            'mve_collection_ticker',
            'mve_selected_legs',
            'primary_participant_key',
            'price_level_structure',
            'price_ranges'

        """
        if status is not None and not isinstance(status, list):
            raise TypeError("status must be a list of strings or None")
        invalid_statuses = [s for s in (status or []) if s not in {"unopened", "open", "closed", "settled"}]
        if invalid_statuses:
            raise ValueError(f"Invalid status values: {invalid_statuses}")

        status_label = ", ".join(status) if status else "any status"
        # (f"[API] Fetching {status_label} markets for event {event_ticker}...")

        def fetch(cursor, status_value: Optional[str]):
            kwargs = {"event_ticker": event_ticker, "cursor": cursor}
            if status_value:
                kwargs["status"] = status_value
            return self.client._market_api.get_markets(**kwargs)

        if status:
            markets = []
            for status_value in status:
                markets.extend(self._paginate(lambda c: fetch(c, status_value), lambda r: r.markets))
        else:
            markets = self._paginate(lambda c: fetch(c, None), lambda r: r.markets)
        # print(f"[API] Fetched {len(markets)} markets for event {event_ticker}")
        return markets

    def get_events(
        self,
        series_ticker: str,
        status: Optional[list[str]] = None,
    ) -> list:
        """
        Get all events for a series.

        Args:
            series_ticker: Series ticker (e.g., "NFL")
            status: Event status filter

        Returns:
            List of event objects

        Here are all the event fields:
            'event_ticker',
            'series_ticker',
            'sub_title',
            'title',
            'collateral_return_type',
            'mutually_exclusive',
            'category',
            'strike_date',
            'strike_period',
            'markets',
            'available_on_brokers',
            'product_metadata'
        """
        if status is not None and not isinstance(status, list):
            raise TypeError("status must be a list of strings or None")
        invalid_statuses = [s for s in (status or []) if s not in {"open", "closed", "settled"}]
        if invalid_statuses:
            raise ValueError(f"Invalid status values: {invalid_statuses}")

        status_label = ", ".join(status) if status else "any status"
        # print(f"[API] Fetching {status_label} events for {series_ticker}...")

        def fetch(cursor, status_value: Optional[str]):
            kwargs = {
                "series_ticker": series_ticker,
                "cursor": cursor,
            }
            if status_value:
                kwargs["status"] = status_value
            return self.client._events_api.get_events(**kwargs)

        if status:
            events = []
            for status_value in status:
                events.extend(self._paginate(lambda c: fetch(c, status_value), lambda r: r.events))
        else:
            events = self._paginate(lambda c: fetch(c, None), lambda r: r.events)

        # Filter by max close time if provided (API only supports min_close_ts)
        # print(f"[API] Fetched {len(events)} events total")
        return events

    def get_series_list(
        self,
        category: Optional[str] = None,
        tags: Optional[str] = None,
        include_product_metadata: Optional[bool] = None,
    ) -> list:
        """
        Get available series, optionally filtered by category or tags.

        Args:
            category: Optional series category filter
            tags: Optional tags filter
            include_product_metadata: Include product metadata if True

        Returns:
            List of series objects

        Here are the series fields:
            'additional_prohibitions',
            'category',
            'contract_terms_url',
            'contract_url',
            'fee_multiplier',
            'fee_type',
            'frequency',
            'settlement_sources',
            'tags',
            'ticker',
            'title']
        """
        # print("[API] Fetching series list...")
        response = self.client._market_api.get_series_list_without_preload_content(
            category=category,
            tags=tags,
            include_product_metadata=include_product_metadata,
        )
        raw = response.data if hasattr(response, "data") else response.read()
        if isinstance(raw, (bytes, bytearray)):
            raw = raw.decode("utf-8")
        payload = json.loads(raw)
        series = payload.get("series", [])
        # print(f"[API] Fetched {len(series)} series total")
        return series

    def get_series_categories(self) -> dict:
        """
        Get series categories and their tags.

        Returns:
            Dict mapping category name to list of tags
        """
        # print("[API] Fetching series categories...")
        response = self.client._search_api.get_tags_for_series_categories_without_preload_content()
        raw = response.data if hasattr(response, "data") else response.read()
        if isinstance(raw, (bytes, bytearray)):
            raw = raw.decode("utf-8")
        payload = json.loads(raw)
        categories = payload.get("tags_by_categories", {})
        # print(f"[API] Fetched {len(categories)} categories total")
        return categories
