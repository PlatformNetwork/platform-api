//! Type aliases for WebSocket handlers

use std::sync::Arc;

/// Type alias for WebSocket sender
pub type WsSender = Arc<
    tokio::sync::Mutex<
        futures_util::stream::SplitSink<axum::extract::ws::WebSocket, axum::extract::ws::Message>,
    >,
>;

/// Type alias for message channel sender
pub type MessageSender = Arc<tokio::sync::mpsc::Sender<String>>;

