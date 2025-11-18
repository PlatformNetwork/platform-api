//! Challenge route handlers

pub mod list;
pub mod get;
pub mod crud;
pub mod emissions;
pub mod active;
pub mod specs;
pub mod debug;
pub mod jobs;
pub mod env_vars;

use axum::{routing::{get, post}, Router};
use crate::state::AppState;

/// Create challenges router
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/challenges", get(list::list_challenges).post(crud::create_challenge))
        .route("/challenges/active", get(active::get_active_challenges))
        .route("/challenges/specs", get(specs::get_challenge_specs))
        .route("/challenges/public", get(list::list_challenges_public))
        .route("/challenges/debug", get(debug::debug_challenges))
        .route(
            "/challenges/:id",
            get(get::get_challenge)
                .put(crud::update_challenge)
                .delete(crud::delete_challenge),
        )
        .route("/challenges/:id/public", get(get::get_challenge_public))
        .route("/challenges/:id/emissions", get(emissions::get_challenge_emissions))
        .route("/challenges/:id/jobs", get(jobs::get_challenge_jobs))
        .route(
            "/challenges/:compose_hash/env-vars",
            post(env_vars::store_challenge_env_vars),
        )
}

