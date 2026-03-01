#![warn(clippy::all)]
#![forbid(unsafe_code)]

//! OneClaw — Multi-agent AI assistant with router architecture.

pub mod agent;
pub mod channels;
pub mod config;
pub mod identity;
pub mod memory;
pub mod providers;
pub mod router;
pub mod tools;
