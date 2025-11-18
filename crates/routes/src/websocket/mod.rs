mod auth;
mod handler;
mod messages;
mod orm;
mod utils;

use platform_api::state::AppState;
use axum::Router;

pub use messages::ValidatorNotification;

/// Create WebSocket router
pub fn create_router() -> Router<AppState> {
    Router::new().route(
        "/validators/:hotkey/ws",
        axum::routing::get(handler::validator_websocket),
    )
}
