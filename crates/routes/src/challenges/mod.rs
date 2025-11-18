mod handlers;
mod types;

use axum::{
    routing::{get, post},
    Router,
};

use platform_api::state::AppState;

pub use handlers::*;
pub use types::*;

/// Create challenges router
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/challenges", get(list_challenges).post(create_challenge))
        .route("/challenges/active", get(get_active_challenges))
        .route("/challenges/specs", get(get_challenge_specs))
        .route("/challenges/public", get(list_challenges_public))
        .route("/challenges/debug", get(debug_challenges))
        .route(
            "/challenges/:id",
            get(get_challenge)
                .put(update_challenge)
                .delete(delete_challenge),
        )
        .route("/challenges/:id/public", get(get_challenge_public))
        .route("/challenges/:id/emissions", get(get_challenge_emissions))
        .route("/challenges/:id/jobs", get(get_challenge_jobs))
        .route(
            "/challenges/:compose_hash/env-vars",
            post(store_challenge_env_vars),
        )
}

