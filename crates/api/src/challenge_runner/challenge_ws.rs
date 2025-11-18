//! Legacy module - re-exports from ws module
//!
//! This module is kept for backward compatibility.
//! New code should use `crate::challenge_runner::ws` directly.

use crate::challenge_runner::ws;

pub use ws::{
    ChallengeWsClient, ConnectionState, EncryptedEnvelope,
};

impl ChallengeWsClient {
    /// Connect with reconnection logic and handle messages
    /// Handles ORM queries from challenge SDK and executes them via ORM gateway
    pub async fn connect_with_reconnect<F>(&self, callback: F) -> anyhow::Result<()>
    where
        F: Fn(serde_json::Value, tokio::sync::mpsc::Sender<serde_json::Value>) + Send + Sync + 'static,
    {
        ws::connection::connect_with_reconnect(self, callback).await
    }
}
