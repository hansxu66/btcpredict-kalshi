# Discovery Module

Utilities for discovering and exploring Kalshi prediction markets.

## File Structure

```
discovery/
├── helper/                     # Core utilities
│   ├── client.py               # Kalshi API client initialization
│   ├── discovery.py            # MarketDiscovery class for API interactions
│   ├── .env                    # API credentials (not committed)
│   └── private_key.pem         # API private key (not committed)
│
├── series_events/              # Series and event exploration
│   ├── series_events_discovery.py  # Export series/categories to CSV
│   ├── series_list.csv         # All Kalshi series
│   ├── series_list.xlsx        # Excel version
│   └── series_categories.csv   # Category-to-tag mappings
│
├── sports/                     # Sports market utilities
│   ├── sports.py               # Fetch all sports markets
│   ├── filter.py               # Filter by sport (NFL, NBA, soccer, etc.)
│   └── sports_markets_*.csv    # Output files
│
├── top_traded_markets/         # Volume-based market discovery
│   ├── top_traded_markets.py   # Fetch top markets by volume
│   ├── top_traded_markets_volume.csv
│   └── top_traded_markets_volume_24h.csv
│
└── leaderboard/                # Leaderboard scraping
    └── leaderboard_crawler.py  # Selenium-based leaderboard scraper
```

## Module Details

### helper/

**client.py**
- `init_kalshi_client()` - Initialize authenticated Kalshi API client
- Reads credentials from `.env` file (`KALSHI_API_KEY_ID`, `KALSHI_PRIVATE_KEY_PATH`)

**discovery.py**
- `MarketDiscovery` class with methods:
  - `get_markets(event_ticker, status)` - Get all markets for an event
  - `get_events(series_ticker, status)` - Get all events for a series
  - `get_series_list(category, tags)` - Get available series
  - `get_series_categories()` - Get category-to-tag mappings
- Handles pagination automatically

### series_events/

**series_events_discovery.py**
- `find_series_list()` - Export all series to `series_list.csv`
- `find_categories_list()` - Export categories to `series_categories.csv`
- `find_events_test(series_ticker)` - List events for a series
- `find_markets_test(event_ticker)` - List markets for an event

### sports/

**sports.py**
- `get_all_sports_markets(discovery, output_path, status, sports_tags)` - Fetch all sports markets
- Combines category-based and keyword-based series discovery
- Outputs: `sports_markets_open_MM_DD_YYYY.csv`, `sports_markets_settled_MM_DD_YYYY.csv`

**filter.py**
- `filter_nfl()` - Filter for NFL game markets
- `filter_nba()` - Filter for NBA game markets
- `filter_college_football()` - Filter for NCAA football
- `filter_soccer(leagues)` - Filter for soccer (EPL, La Liga, etc.)
- `filter_all()` - Filter all supported sports
- Parses game dates and team codes from event tickers

### top_traded_markets/

**top_traded_markets.py**
- `get_top_traded_markets(status, top_n)` - Get top N markets by volume
- Uses heap-based streaming to handle large datasets
- Outputs separate CSVs for total volume and 24h volume

### leaderboard/

**leaderboard_crawler.py**
- `get_leaderboard(timeframe, category)` - Scrape Kalshi leaderboard
- Requires Selenium and Chrome
- Supports timeframes: day, week, month, ytd, all
- Extracts profit, volume, and predictions rankings

## Usage Examples

```python
from discovery.helper.client import init_kalshi_client
from discovery.helper.discovery import MarketDiscovery

# Initialize
client = init_kalshi_client()
discovery = MarketDiscovery(client)

# Get all NFL series
nfl_series = discovery.get_series_list(category="Sports")

# Get events for a series
events = discovery.get_events(series_ticker="KXNFLGAME", status=["open"])

# Get markets for an event
markets = discovery.get_markets(event_ticker="KXNFLGAME-25JAN12KCBUF")
```

## Output Fields

### Markets
`ticker`, `event_ticker`, `market_type`, `title`, `open_time`, `close_time`, `volume`, `volume_24h`, `result`, `open_interest`

### Events
`event_ticker`, `series_ticker`, `title`, `sub_title`, `category`, `strike_date`

### Series
`ticker`, `title`, `category`, `tags`, `frequency`, `settlement_sources`
