//! WebSocket client and server functionality for challenge communication
//!
//! This crate provides WebSocket client and server implementations for
//! secure, encrypted communication between validators and the platform API.

pub mod client;
pub mod encryption;
pub mod handlers;
pub mod messages;
pub mod server;
pub mod tdx_verification;

