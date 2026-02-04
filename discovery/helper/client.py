import os
import warnings
from datetime import datetime, timedelta
from typing import Optional

warnings.filterwarnings("ignore", message="urllib3.*doesn't match a supported version")

from dotenv import load_dotenv
from kalshi_python_sync import Configuration, KalshiClient


def init_kalshi_client(use_demo: bool = False) -> KalshiClient:
    """
    Initialize the Kalshi client by reading credentials from .env file.

    Args:
        use_demo: If True, uses demo API URL and demo credentials. Defaults to False.

    Required .env variables:
        KALSHI_API_KEY_ID: Your Kalshi API key ID (production)
        KALSHI_PRIVATE_KEY_PATH: Path to your private key PEM file
        OR
        KALSHI_PRIVATE_KEY_PEM: The private key content directly (with \\n for newlines)

        For demo mode:
        KALSHI_DEMO_API_KEY_ID: Your Kalshi demo API key ID
        KALSHI_DEMO_PRIVATE_KEY_PATH: Path to your demo private key PEM file
        OR
        KALSHI_DEMO_PRIVATE_KEY_PEM: The demo private key content directly

    Returns:
        KalshiClient: Authenticated Kalshi client instance
    """
    load_dotenv()

    if use_demo:
        api_key_id = os.getenv("KALSHI_DEMO_API_KEY_ID")
        private_key_path = os.getenv("KALSHI_DEMO_PRIVATE_KEY_PATH")
        private_key_pem = os.getenv("KALSHI_DEMO_PRIVATE_KEY_PEM")
        host_url = "https://demo-api.kalshi.co/trade-api/v2"
    else:
        api_key_id = os.getenv("KALSHI_PROD_API_KEY_ID")
        private_key_path = os.getenv("KALSHI_PROD_PRIVATE_KEY_PATH")
        private_key_pem = os.getenv("KALSHI_PROD_PRIVATE_KEY_PEM")
        host_url = "https://api.elections.kalshi.com/trade-api/v2"

    if not api_key_id:
        key_name = "KALSHI_DEMO_API_KEY_ID" if use_demo else "KALSHI_API_KEY_ID"
        raise ValueError(f"{key_name} not found in .env file")

    # Read private key from file path or use direct PEM content
    # Handle relative paths by checking relative to .env file location
    if private_key_path:
        # Try absolute path first, then relative to script directory
        if os.path.isabs(private_key_path):
            key_path = private_key_path
        else:
            key_path = os.path.join(os.path.dirname(__file__), private_key_path)

        if os.path.exists(key_path):
            with open(key_path, "r") as f:
                private_key = f.read()
        else:
            raise ValueError(f"Private key file not found: {key_path}")
    elif private_key_pem:
        # Replace literal \n with actual newlines if needed
        private_key = private_key_pem.replace("\\n", "\n")
    else:
        if use_demo:
            raise ValueError(
                "Either KALSHI_DEMO_PRIVATE_KEY_PATH (valid file path) or "
                "KALSHI_DEMO_PRIVATE_KEY_PEM must be set in .env file"
            )
        else:
            raise ValueError(
                "Either KALSHI_PRIVATE_KEY_PATH (valid file path) or "
                "KALSHI_PRIVATE_KEY_PEM must be set in .env file"
            )

    config = Configuration(
        host=host_url,
        retries=3,  # Auto-retry transient failures
    )
    config.api_key_id = api_key_id
    config.private_key_pem = private_key

    return KalshiClient(config)



if __name__ == "__main__":
    # Initialize client
    client = init_kalshi_client(True)

    # Test connection by fetching balance
    try:
        balance = client.get_balance()
        print(f"Connection successful! Balance: {balance}")
    except Exception as e:
        print(f"Connection failed: {e}")
