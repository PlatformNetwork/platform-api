/// Extract compose_hash from event log if available
pub fn extract_compose_hash_from_event_log(event_log: &str) -> Option<String> {
    if let Ok(event_log_json) = serde_json::from_str::<serde_json::Value>(event_log) {
        event_log_json.as_array().and_then(|events| {
            for event in events {
                if let Some(event_type) = event.get("event").and_then(|e| e.as_str()) {
                    if event_type == "compose-hash" {
                        if let Some(payload) = event.get("event_payload").and_then(|p| p.as_str()) {
                            return Some(payload.to_string());
                        }
                    }
                }
            }
            None
        })
    } else {
        None
    }
}

/// Extract app_id from event log
pub fn extract_app_id_from_event_log(event_log: &str) -> Option<String> {
    if let Ok(event_log_json) = serde_json::from_str::<serde_json::Value>(event_log) {
        event_log_json.as_array().and_then(|events| {
            for event in events {
                if let Some(event_type) = event.get("event").and_then(|e| e.as_str()) {
                    if event_type == "app-id" {
                        return event
                            .get("event_payload")
                            .and_then(|p| p.as_str())
                            .map(|s| s.to_string());
                    }
                }
            }
            None
        })
    } else {
        None
    }
}

/// Extract instance_id from event log
pub fn extract_instance_id_from_event_log(event_log: &str) -> Option<String> {
    if let Ok(event_log_json) = serde_json::from_str::<serde_json::Value>(event_log) {
        event_log_json.as_array().and_then(|events| {
            for event in events {
                if let Some(event_type) = event.get("event").and_then(|e| e.as_str()) {
                    if event_type == "instance-id" {
                        return event
                            .get("event_payload")
                            .and_then(|p| p.as_str())
                            .map(|s| s.to_string());
                    }
                }
            }
            None
        })
    } else {
        None
    }
}
