#![warn(clippy::all)]
#![forbid(unsafe_code)]
#![allow(dead_code)]

//! OneClaw — Multi-agent AI assistant with router architecture.

pub mod agent;
pub mod channels;
pub mod config;
pub mod coordination;
pub mod cron;
pub mod daemon;
pub mod doctor;
pub mod goals;
pub mod health;
pub mod heartbeat;
pub mod hooks;
pub mod identity;
pub mod memory;
pub mod providers;
pub mod router;
pub mod service;
pub mod tools;
pub mod update;
