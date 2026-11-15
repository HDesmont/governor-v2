//! Governor v2.0 library — public API for integration and testing.
//!
//! Re-exports all internal modules so that integration tests and
//! external consumers can access Governor's functionality.

pub mod cli;
pub mod clipboard;
pub mod context;
pub mod health;
pub mod hud;
pub mod init;
pub mod integrity;
pub mod lock;
pub mod pool;
pub mod project;
pub mod relay;
pub mod session;
pub mod tui;
pub mod config;