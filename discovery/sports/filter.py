"""
Filter settled sports markets by sport type.

Supports: NFL, NBA, College Football, Soccer (multiple leagues).
Excludes women's sports by default.
"""
import re
from datetime import datetime
from typing import Optional
import pandas as pd


# =============================================================================
# TEAM CODES
# =============================================================================

NFL_TEAMS = {
    "ARI", "ATL", "BAL", "BUF", "CAR", "CHI", "CIN", "CLE",
    "DAL", "DEN", "DET", "GB", "HOU", "IND", "JAC", "KC",
    "LA", "LAC", "LV", "MIA", "MIN", "NE", "NO", "NYG",
    "NYJ", "PHI", "PIT", "SEA", "SF", "TB", "TEN", "WAS"
}

NBA_TEAMS = {
    "ATL", "BOS", "BKN", "CHA", "CHI", "CLE", "DAL", "DEN",
    "DET", "GS", "HOU", "IND", "LAC", "LAL", "MEM", "MIA",
    "MIL", "MIN", "NO", "NY", "NYK", "OKC", "ORL", "PHI",
    "PHX", "POR", "SAC", "SA", "SAS", "TOR", "UTA", "WAS"
}

# Soccer uses 3-letter codes but varies by league
SOCCER_SERIES = {
    "KXEPLGAME",        # English Premier League
    "KXLALIGAGAME",     # La Liga
    "KXBUNDESLIGAGAME", # Bundesliga
    "KXSERIEAGAME",     # Serie A
    "KXLIGUE1GAME",     # Ligue 1
    "KXMLSGAME",        # MLS
    "KXUCLGAME",        # UEFA Champions League
    "KXUELGAME",        # UEFA Europa League
    "KXUECLGAME",       # UEFA Europa Conference League
    "KXFIFAGAME",       # FIFA/International
    "KXEREDIVISIEGAME", # Eredivisie
    "KXLIGAPORTUGALGAME", # Liga Portugal
    "KXSCOTTISHPREMGAME", # Scottish Premiership
}

# Series to exclude (women's sports)
EXCLUDED_SERIES = {
    "KXWNBAGAME",      # WNBA
    "KXNCAAWBGAME",    # NCAA Women's Basketball
}


# =============================================================================
# PARSING HELPERS
# =============================================================================

def parse_game_ticker(event_ticker: str, team_set: set, prefix: str) -> dict:
    """
    Generic parser for game event tickers.

    Format: {PREFIX}-{YY}{MON}{DD}{AWAY}{HOME}
    """
    pattern = rf'{prefix}-(\d{{2}})([A-Z]{{3}})(\d{{2}})([A-Z]+)'
    match = re.match(pattern, event_ticker)
    if not match:
        return {"game_date": None, "away_team": None, "home_team": None}

    year = int(match.group(1)) + 2000
    month_str = match.group(2)
    day = int(match.group(3))
    teams_str = match.group(4)

    # Parse teams by trying known team codes (3-letter first, then 2-letter)
    away_team = None
    home_team = None
    for length in [3, 2]:
        if len(teams_str) >= length:
            prefix_team = teams_str[:length]
            if prefix_team in team_set:
                away_team = prefix_team
                home_team = teams_str[length:]
                if home_team in team_set:
                    break
                else:
                    away_team = None
                    home_team = None

    months = {
        "JAN": 1, "FEB": 2, "MAR": 3, "APR": 4, "MAY": 5, "JUN": 6,
        "JUL": 7, "AUG": 8, "SEP": 9, "OCT": 10, "NOV": 11, "DEC": 12
    }
    month = months.get(month_str, 1)

    try:
        game_date = datetime(year, month, day)
    except ValueError:
        game_date = None

    return {
        "game_date": game_date,
        "away_team": away_team,
        "home_team": home_team,
    }


def parse_soccer_ticker(event_ticker: str) -> dict:
    """
    Parse soccer event ticker - teams are embedded but codes vary by league.
    Just extract the date and keep team info in title.
    """
    # Try to match date pattern
    match = re.search(r'-(\d{2})([A-Z]{3})(\d{2})', event_ticker)
    if not match:
        return {"game_date": None, "away_team": None, "home_team": None}

    year = int(match.group(1)) + 2000
    month_str = match.group(2)
    day = int(match.group(3))

    months = {
        "JAN": 1, "FEB": 2, "MAR": 3, "APR": 4, "MAY": 5, "JUN": 6,
        "JUL": 7, "AUG": 8, "SEP": 9, "OCT": 10, "NOV": 11, "DEC": 12
    }
    month = months.get(month_str, 1)

    try:
        game_date = datetime(year, month, day)
    except ValueError:
        game_date = None

    # Extract teams from the ticker after date
    teams_part = event_ticker.split(match.group(0))[-1] if match else ""

    return {
        "game_date": game_date,
        "away_team": teams_part[:3] if len(teams_part) >= 3 else None,
        "home_team": teams_part[3:] if len(teams_part) >= 6 else teams_part[3:] if len(teams_part) > 3 else None,
    }


# =============================================================================
# CORE FILTER FUNCTION
# =============================================================================

def filter_markets(
    series_tickers: list[str],
    team_set: Optional[set] = None,
    ticker_prefix: Optional[str] = None,
    input_path: str = "discovery/sports_markets_settled.csv",
    output_path: Optional[str] = None,
    parse_teams: bool = True,
) -> pd.DataFrame:
    """
    Filter markets by series ticker(s).

    Args:
        series_tickers: List of series tickers to include
        team_set: Set of valid team codes for parsing
        ticker_prefix: Prefix for event ticker parsing
        input_path: Path to input CSV
        output_path: Path for output CSV (optional)
        parse_teams: Whether to parse team codes from ticker

    Returns:
        Filtered DataFrame
    """
    df = pd.read_csv(input_path)

    # Filter to specified series, excluding women's sports
    mask = df["series_ticker"].isin(series_tickers)
    mask &= ~df["series_ticker"].isin(EXCLUDED_SERIES)
    filtered = df[mask].copy()

    print(f"Found {len(filtered)} markets")
    print(f"Covering {filtered['event_ticker'].nunique()} events")

    # Parse event tickers if requested
    if parse_teams and team_set and ticker_prefix:
        parsed = filtered["event_ticker"].apply(
            lambda x: parse_game_ticker(x, team_set, ticker_prefix)
        ).apply(pd.Series)
        filtered = pd.concat([filtered, parsed], axis=1)
    elif parse_teams:
        # Use generic soccer parser
        parsed = filtered["event_ticker"].apply(parse_soccer_ticker).apply(pd.Series)
        filtered = pd.concat([filtered, parsed], axis=1)

    # Extract team from market_ticker
    filtered["team"] = filtered["market_ticker"].str.split("-").str[-1]

    # Determine winner
    filtered["is_winner"] = filtered["result"] == "yes"

    # Reorder columns
    base_cols = ["series_ticker", "event_ticker", "market_ticker"]
    parsed_cols = ["game_date", "away_team", "home_team"] if parse_teams else []
    other_cols = ["team", "is_winner", "title", "volume", "volume_24h",
                  "open_interest", "open_time", "close_time", "result"]
    columns = base_cols + parsed_cols + other_cols
    columns = [c for c in columns if c in filtered.columns]
    filtered = filtered[columns]

    # Sort (handle None values in game_date)
    if "game_date" in filtered.columns:
        filtered = filtered.sort_values(["event_ticker", "team"])
        # Sort by game_date where available
        filtered["_sort_date"] = pd.to_datetime(filtered["game_date"], errors="coerce")
        filtered = filtered.sort_values(["_sort_date", "event_ticker", "team"])
        filtered = filtered.drop(columns=["_sort_date"])
    else:
        filtered = filtered.sort_values(["event_ticker", "team"])

    # Save if output path provided
    if output_path:
        filtered.to_csv(output_path, index=False)
        print(f"Saved to {output_path}")

    # Summary
    if "game_date" in filtered.columns:
        valid_dates = filtered["game_date"].dropna()
        if len(valid_dates) > 0:
            dates = pd.to_datetime(valid_dates)
            print(f"Date range: {dates.min()} to {dates.max()}")
    print(f"Total volume: {filtered['volume'].sum():,}")

    return filtered


# =============================================================================
# SPORT-SPECIFIC FUNCTIONS
# =============================================================================

def filter_nfl(
    input_path: str = "discovery/sports_markets_settled.csv",
    output_path: str = "discovery/nfl_games.csv",
) -> pd.DataFrame:
    """Filter for NFL game winner markets."""
    print("=== NFL Games ===")
    return filter_markets(
        series_tickers=["KXNFLGAME"],
        team_set=NFL_TEAMS,
        ticker_prefix="KXNFLGAME",
        input_path=input_path,
        output_path=output_path,
    )


def filter_nba(
    input_path: str = "discovery/sports_markets_settled.csv",
    output_path: str = "discovery/nba_games.csv",
) -> pd.DataFrame:
    """Filter for NBA game winner markets."""
    print("=== NBA Games ===")
    return filter_markets(
        series_tickers=["KXNBAGAME"],
        team_set=NBA_TEAMS,
        ticker_prefix="KXNBAGAME",
        input_path=input_path,
        output_path=output_path,
    )


def filter_college_football(
    input_path: str = "discovery/sports_markets_settled.csv",
    output_path: str = "discovery/ncaaf_games.csv",
) -> pd.DataFrame:
    """Filter for NCAA Football game winner markets."""
    print("=== College Football Games ===")
    return filter_markets(
        series_tickers=["KXNCAAFGAME"],
        team_set=None,  # Too many teams to enumerate
        ticker_prefix=None,
        input_path=input_path,
        output_path=output_path,
        parse_teams=False,
    )


def filter_soccer(
    input_path: str = "discovery/sports_markets_settled.csv",
    output_path: str = "discovery/soccer_games.csv",
    leagues: Optional[list[str]] = None,
) -> pd.DataFrame:
    """
    Filter for soccer game winner markets.

    Args:
        leagues: Specific leagues to include. If None, includes all major leagues.
                 Options: EPL, LALIGA, BUNDESLIGA, SERIEA, LIGUE1, MLS, UCL, UEL, FIFA
    """
    print("=== Soccer Games ===")

    league_map = {
        "EPL": "KXEPLGAME",
        "LALIGA": "KXLALIGAGAME",
        "BUNDESLIGA": "KXBUNDESLIGAGAME",
        "SERIEA": "KXSERIEAGAME",
        "LIGUE1": "KXLIGUE1GAME",
        "MLS": "KXMLSGAME",
        "UCL": "KXUCLGAME",
        "UEL": "KXUELGAME",
        "UECL": "KXUECLGAME",
        "FIFA": "KXFIFAGAME",
    }

    if leagues:
        series = [league_map[lg.upper()] for lg in leagues if lg.upper() in league_map]
    else:
        series = list(SOCCER_SERIES)

    return filter_markets(
        series_tickers=series,
        team_set=None,
        ticker_prefix=None,
        input_path=input_path,
        output_path=output_path,
        parse_teams=True,
    )


def filter_all(
    input_path: str = "discovery/sports_markets_settled.csv",
) -> dict[str, pd.DataFrame]:
    """
    Filter all supported sports and return dict of DataFrames.

    Returns:
        Dict with keys: 'nfl', 'nba', 'ncaaf', 'soccer'
    """
    return {
        "nfl": filter_nfl(input_path),
        "nba": filter_nba(input_path),
        "ncaaf": filter_college_football(input_path),
        "soccer": filter_soccer(input_path),
    }


# =============================================================================
# CLI
# =============================================================================

if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        print("Usage: python filter.py <sport>")
        print("Sports: nfl, nba, ncaaf, soccer, all")
        sys.exit(1)

    sport = sys.argv[1].lower()

    if sport == "nfl":
        df = filter_nfl()
    elif sport == "nba":
        df = filter_nba()
    elif sport == "ncaaf":
        df = filter_college_football()
    elif sport == "soccer":
        df = filter_soccer()
    elif sport == "all":
        filter_all()
    else:
        print(f"Unknown sport: {sport}")
        sys.exit(1)
