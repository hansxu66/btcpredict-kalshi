//! Kalshi & Multi-Exchange BTC Real-Time Monitor Library
//!
//! This library provides modules for:
//! - Authentication with Kalshi and crypto exchange APIs
//! - WebSocket monitoring for real-time data
//! - REST API trading operations
//! - Calculator for baseline price computation
//! - Redis client for real-time pub/sub
//! - Fair value estimation for binary options
//! - Market making strategy engine

pub mod auth;
pub mod calculator;
pub mod fair_value;
pub mod market_maker;
pub mod redis_client;
pub mod trading_apis;
pub mod types;
pub mod websockets;
