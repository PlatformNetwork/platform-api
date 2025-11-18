use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct HandshakeMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub validator_hotkey: String,
}

#[derive(Debug, Deserialize)]
pub struct AttestationMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub quote: Option<String>,
    pub event_log: Option<String>,
    pub measurements: Option<Vec<String>>,
    #[serde(default)]
    pub vm_config: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SecureMessage {
    pub message_type: String,
    pub data: serde_json::Value,
    pub timestamp: u64,
    pub nonce: String,
    pub signature: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidatorNotification {
    pub msg_type: String,
    pub job_id: Option<Uuid>,
    pub challenge_id: Option<Uuid>,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handshake_message() {
        let json = r#"{"type":"handshake","validator_hotkey":"5DD123..."}"#;
        let msg: HandshakeMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type, "handshake");
    }
}
