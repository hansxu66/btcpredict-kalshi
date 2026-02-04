@echo off
echo Starting KalshiStrat Dashboard...

:: Start Redis server
start "Redis" cmd /k "redis-server"

:: Wait a moment for Redis to start
timeout /t 2 /nobreak >nul

:: Start Rust monitor
start "Rust Monitor" cmd /k "cd /d C:\Users\18312\PycharmProjects\KalshiStrat && cargo run"

:: Start Dashboard backend
start "Dashboard" cmd /k "cd /d C:\Users\18312\PycharmProjects\KalshiStrat\dashboard\backend && python -m uvicorn main:app --reload --host 0.0.0.0 --port 8000"

echo.
echo All services starting...
echo - Redis: localhost:6379
echo - Dashboard: http://localhost:8000
echo.
echo Close this window or press any key to exit.
pause >nul
