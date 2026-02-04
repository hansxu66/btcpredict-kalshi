"""
Dashboard Backend - FastAPI server with Redis subscriber and WebSocket broadcast.

Subscribes to Redis pub/sub channels and broadcasts updates to connected browser clients.

Usage:
    uvicorn main:app --host 0.0.0.0 --port 8000 --reload

Environment:
    REDIS_URL - Redis connection URL (default: redis://localhost:6379)
"""

import asyncio
import json
import os
from contextlib import asynccontextmanager
from datetime import datetime
from typing import Dict, Set

import redis.asyncio as redis
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse

# Configuration
REDIS_URL = os.getenv("REDIS_URL", "redis://localhost:6379")
REDIS_CHANNELS = ["calculator:updates"]

# State storage
latest_state: Dict[str, dict] = {
    "calculator": {},  # ticker_id -> latest update
}


class ConnectionManager:
    """Manages WebSocket connections to browser clients."""

    def __init__(self):
        self.active_connections: Set[WebSocket] = set()

    async def connect(self, websocket: WebSocket):
        await websocket.accept()
        self.active_connections.add(websocket)
        # Send current state to new client
        await websocket.send_json({
            "type": "state_snapshot",
            "data": latest_state,
            "timestamp": datetime.utcnow().isoformat()
        })

    def disconnect(self, websocket: WebSocket):
        self.active_connections.discard(websocket)

    async def broadcast(self, message: dict):
        """Broadcast message to all connected clients."""
        disconnected = set()
        for connection in self.active_connections:
            try:
                await connection.send_json(message)
            except Exception:
                disconnected.add(connection)
        # Clean up disconnected clients
        self.active_connections -= disconnected


manager = ConnectionManager()


async def redis_subscriber():
    """Subscribe to Redis channels and broadcast updates."""
    print(f"[REDIS] Connecting to {REDIS_URL}")

    while True:
        try:
            client = redis.from_url(REDIS_URL)
            pubsub = client.pubsub()

            await pubsub.subscribe(*REDIS_CHANNELS)
            print(f"[REDIS] Subscribed to: {REDIS_CHANNELS}")

            async for message in pubsub.listen():
                if message["type"] == "message":
                    channel = message["channel"].decode()
                    data = message["data"].decode()

                    try:
                        parsed = json.loads(data)

                        # Update state
                        if channel == "calculator:updates":
                            ticker_id = parsed.get("ticker_id", "unknown")
                            latest_state["calculator"][ticker_id] = parsed
                            source = "calculator"
                            fair_price = parsed.get("fair_price")
                            fair_std = parsed.get("fair_std")
                            print(f"[REDIS] Calculator update: {ticker_id} fair={fair_price} std={fair_std}")
                        else:
                            source = "unknown"
                            print(f"[REDIS] Unknown channel: {channel}")

                        # Broadcast to WebSocket clients
                        await manager.broadcast({
                            "type": "update",
                            "source": source,
                            "data": parsed,
                        })
                        print(f"[WS] Broadcast to {len(manager.active_connections)} clients")

                    except json.JSONDecodeError as e:
                        print(f"[REDIS] JSON decode error: {e}")

        except redis.ConnectionError as e:
            print(f"[REDIS] Connection error: {e}, reconnecting in 5s...")
            await asyncio.sleep(5)
        except Exception as e:
            print(f"[REDIS] Error: {e}, reconnecting in 5s...")
            await asyncio.sleep(5)


@asynccontextmanager
async def lifespan(app: FastAPI):
    """Startup and shutdown events."""
    # Start Redis subscriber task
    subscriber_task = asyncio.create_task(redis_subscriber())
    print("[SERVER] Started Redis subscriber")

    yield

    # Cleanup
    subscriber_task.cancel()
    try:
        await subscriber_task
    except asyncio.CancelledError:
        pass
    print("[SERVER] Stopped Redis subscriber")


app = FastAPI(
    title="Trading Dashboard API",
    description="Real-time trading data dashboard",
    lifespan=lifespan
)

# CORS for local development
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


@app.get("/")
async def root():
    """Serve the dashboard frontend."""
    frontend_path = os.path.join(os.path.dirname(__file__), "..", "frontend", "index.html")
    if os.path.exists(frontend_path):
        return FileResponse(frontend_path)
    return {"message": "Dashboard API", "status": "running"}


@app.get("/api/health")
async def health():
    """Health check endpoint."""
    return {
        "status": "ok",
        "connected_clients": len(manager.active_connections),
        "calculator_tickers": len(latest_state["calculator"]),
    }


@app.get("/api/state")
async def get_state():
    """Get current state snapshot."""
    return {
        "calculator": latest_state["calculator"],
        "timestamp": datetime.utcnow().isoformat()
    }


@app.websocket("/ws")
async def websocket_endpoint(websocket: WebSocket):
    """WebSocket endpoint for real-time updates."""
    await manager.connect(websocket)
    print(f"[WS] Client connected ({len(manager.active_connections)} total)")

    try:
        while True:
            # Keep connection alive, receive any client messages
            data = await websocket.receive_text()
            if data == "ping":
                await websocket.send_json({"type": "pong"})
    except WebSocketDisconnect:
        manager.disconnect(websocket)
        print(f"[WS] Client disconnected ({len(manager.active_connections)} remaining)")


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8000)
