//! WebSocket client for challenge communication

pub mod types;
pub mod tdx;
pub mod encryption;
pub mod connection;
pub mod messages;
pub mod validators;
pub mod attestation;
pub mod migration_handler;
pub mod message_loop;
pub mod validator_manager;

pub use types::{ChallengeWsClient, ConnectionState, EncryptedEnvelope};
pub use connection::connect_with_reconnect;
pub use tdx::verify_tdx_quote;
pub use attestation::perform_attestation;
pub use migration_handler::{handle_migrations, wait_for_migrations_applied, send_orm_ready};
pub use message_loop::handle_message_loop;
pub use validator_manager::{send_initial_validator_list, spawn_validator_update_task};
