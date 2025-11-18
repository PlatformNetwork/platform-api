//! WebSocket client for challenge communication

pub mod types;
pub mod tdx;
pub mod encryption;
pub mod connection;
pub mod messages;
pub mod validators;

pub use types::{ChallengeWsClient, ConnectionState, EncryptedEnvelope};
pub use connection::connect_with_reconnect;
pub use tdx::verify_tdx_quote;
