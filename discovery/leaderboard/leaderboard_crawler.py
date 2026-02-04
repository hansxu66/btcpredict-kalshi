# -*- coding: utf-8 -*-
"""
Kalshi Leaderboard Crawler

Requires: pip install selenium
Also requires Chrome and chromedriver installed.
"""
import json
import random
import time
from datetime import datetime
from itertools import cycle
from selenium import webdriver
from selenium.webdriver.chrome.options import Options
from bs4 import BeautifulSoup


# User agent pool for rotation (Chrome, Safari, Firefox)
USER_AGENTS = cycle([
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_1) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Safari/605.1.15',
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0',
])


def get_leaderboard(timeframe='week', category=None, wait_time=5, retries=3):
    """
    Fetch Kalshi leaderboard data.

    Args:
        timeframe: 'day', 'week', 'month', 'ytd', or 'all'
        category: Optional category filter (e.g., 'Politics', 'Sports', 'Crypto')
        wait_time: Seconds to wait for JS to render
        retries: Number of retry attempts on failure

    Returns:
        dict with url, timeframe, category, scraped_at, and traders
    """
    url = f'https://kalshi.com/social/leaderboard?timeframe={timeframe}'
    if category:
        url += f'&category={category}'

    for attempt in range(retries):
        try:
            # Setup Chrome with rotating user agent
            options = Options()
            options.add_argument('--headless=new')
            options.add_argument('--no-sandbox')
            options.add_argument('--disable-dev-shm-usage')
            options.add_argument('--disable-gpu')
            options.add_argument('--window-size=1920,1080')
            options.add_argument(f'--user-agent={next(USER_AGENTS)}')

            print(f'[Crawler] Loading: {url} (attempt {attempt + 1}/{retries})')
            driver = webdriver.Chrome(options=options)

            try:
                driver.get(url)
                time.sleep(wait_time)
                html = driver.page_source
                print(f'[Crawler] Got {len(html)} bytes')
            finally:
                driver.quit()

            # Parse the HTML
            traders = parse_leaderboard(html)

            # Verify we got data
            total = sum(len(traders[k]) for k in traders)
            if total == 0:
                raise ValueError('No traders parsed')

            return {
                'url': url,
                'timeframe': timeframe,
                'scraped_at': datetime.now().isoformat(),
                'traders': traders,
            }

        except Exception as e:
            print(f'[Crawler] Attempt {attempt + 1} failed: {e}')
            if attempt < retries - 1:
                delay = random.uniform(2, 5)
                print(f'[Crawler] Retrying in {delay:.1f}s...')
                time.sleep(delay)
                return None
            else:
                print('[Crawler] All retries failed')
                raise
    return None


def parse_leaderboard(html):
    """
    Parse leaderboard HTML to extract trader data.
    Returns dict with 'profit', 'volume', 'predictions' lists.
    """
    import re
    soup = BeautifulSoup(html, 'html.parser')

    # Remove scripts/styles and get clean text
    for tag in soup(['script', 'style', 'noscript']):
        tag.decompose()
    text = soup.get_text(' ', strip=True)

    # Parse the three leaderboard sections
    result = {
        'profit': [],
        'volume': [],
        'predictions': [],
    }

    # Pattern: rank username value (with optional $)
    # Profit section: "1 GoldenPants13 $92,886"
    # Volume section: "1 valence.trade 4,761,528"
    # Predictions: "10 brunoBowser 385"

    # Split by section headers
    sections = re.split(r'(Profit|Volume|Predictions)', text)

    current_section = None
    for part in sections:
        if part in ('Profit', 'Volume', 'Predictions'):
            current_section = part.lower()
            continue

        if current_section and current_section in result:
            # Find entries: number + username + value
            entries = re.findall(
                r'(\d+)\s+([A-Za-z0-9_.]+)\s+\$?([\d,]+)',
                part
            )
            for rank, username, value in entries:
                result[current_section].append({
                    'rank': int(rank),
                    'username': username,
                    'value': value.replace(',', ''),
                })

    return result


def save_results(data, filename=None):
    """Save results to JSON with timestamped filename."""
    if filename is None:
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        filename = f"leader_{timestamp}.json"

    with open(filename, 'w', encoding='utf-8') as f:
        json.dump(data, f, indent=2, ensure_ascii=False)

    print(f'[Crawler] Saved to {filename}')



if __name__ == '__main__':
    # Configuration
    TIMEFRAME = 'week'      # 'day', 'week', 'month', 'ytd', 'all'
    CATEGORY = 'Sports'         # e.g., 'Politics', 'Sports', 'Crypto', or None for all
    WAIT_TIME = 5           # Seconds to wait for page load

    # Run
    data = get_leaderboard(TIMEFRAME, CATEGORY, WAIT_TIME)

    # Save results
    save_results(data)