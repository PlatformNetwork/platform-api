mod auth;
mod handler;
mod messages;
mod orm;
mod utils;
mod authentication;
mod message_handler;
mod connection_manager;

use crate::state::AppState;
use axum::Router;

pub use messages::ValidatorNotification;
pub use handler::validator_websocket;
pub use authentication::{handle_unauthenticated_message, complete_authentication};
pub use message_handler::handle_authenticated_messages;
pub use connection_manager::{handle_validator_connection, spawn_health_check_task, shutdown_connections};

/// Create WebSocket router
pub fn create_router() -> Router<AppState> {
    Router::new().route(
        "/validators/:hotkey/ws",
        axum::routing::get(handler::validator_websocket),
    )
}
