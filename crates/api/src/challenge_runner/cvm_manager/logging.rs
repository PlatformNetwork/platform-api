//! CVM logging functionality

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

/// Get CVM logs
pub async fn get_cvm_logs(
    gateway_url: &str,
    instance_id: &str,
    lines: Option<u32>,
) -> Result<String> {
    debug!(
        instance_id = instance_id,
        lines = ?lines,
        "Getting CVM logs"
    );

    let mut logs_url = format!("{}/instances/{}/logs", gateway_url, instance_id);
    
    if let Some(lines) = lines {
        logs_url.push_str(&format!("?lines={}", lines));
    }

    let client = reqwest::Client::new();
    let response = client
        .get(&logs_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM logs")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM logs: {}", error_text));
    }

    let logs = response.text().await
        .context("Failed to read logs")?;

    debug!(
        instance_id = instance_id,
        log_length = logs.len(),
        "Retrieved CVM logs"
    );

    Ok(logs)
}

/// Get structured CVM logs
pub async fn get_structured_cvm_logs(
    gateway_url: &str,
    instance_id: &str,
    filter: &LogFilter,
) -> Result<Vec<LogEntry>> {
    debug!(
        instance_id = instance_id,
        filter = ?filter,
        "Getting structured CVM logs"
    );

    let logs_url = format!("{}/instances/{}/logs/structured", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .post(&logs_url)
        .header("Content-Type", "application/json")
        .json(filter)
        .send()
        .await
        .context("Failed to get structured CVM logs")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get structured CVM logs: {}", error_text));
    }

    let logs: Vec<LogEntry> = response.json().await
        .context("Failed to parse structured logs")?;

    debug!(
        instance_id = instance_id,
        entry_count = logs.len(),
        "Retrieved structured CVM logs"
    );

    Ok(logs)
}

/// Stream CVM logs in real-time
pub async fn stream_cvm_logs(
    gateway_url: &str,
    instance_id: &str,
    filter: Option<LogFilter>,
) -> Result<tokio::sync::mpsc::Receiver<LogEntry>> {
    info!(
        instance_id = instance_id,
        filter = ?filter,
        "Starting CVM log stream"
    );

    let (tx, rx) = tokio::sync::mpsc::channel(1000);

    let stream_url = format!("{}/instances/{}/logs/stream", gateway_url, instance_id);

    // Spawn streaming task
    let instance_id = instance_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = log_stream_task(stream_url, filter, tx).await {
            error!(
                instance_id = instance_id,
                error = %e,
                "Log stream task failed"
            );
        }
    });

    Ok(rx)
}

/// Log streaming background task
async fn log_stream_task(
    stream_url: String,
    filter: Option<LogFilter>,
    tx: tokio::sync::mpsc::Sender<LogEntry>,
) -> Result<()> {
    use futures_util::StreamExt;

    let client = reqwest::Client::new();
    let mut request_builder = client.get(&stream_url);

    if let Some(filter) = filter {
        request_builder = request_builder.json(&filter);
    }

    let response = request_builder
        .send()
        .await
        .context("Failed to start log stream")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Log stream failed with status: {}", response.status()));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("Failed to read log chunk")?;
        
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        
        // Process complete lines
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if !line.trim().is_empty() {
                if let Ok(log_entry) = parse_log_line(&line) {
                    if tx.send(log_entry).await.is_err() {
                        // Channel closed, stop streaming
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Parse log line into structured entry
fn parse_log_line(line: &str) -> Result<LogEntry> {
    // Try to parse as JSON first
    if let Ok(json_entry) = serde_json::from_str::<serde_json::Value>(line) {
        return parse_json_log_entry(json_entry);
    }

    // Fallback to simple text parsing
    parse_text_log_line(line)
}

/// Parse JSON log entry
fn parse_json_log_entry(json: serde_json::Value) -> Result<LogEntry> {
    let timestamp = json.get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    let level = json.get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("info")
        .to_string();

    let message = json.get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut metadata = HashMap::new();
    if let Some(obj) = json.as_object() {
        for (key, value) in obj {
            if key != "timestamp" && key != "level" && key != "message" {
                if let Some(str_val) = value.as_str() {
                    metadata.insert(key.clone(), str_val.to_string());
                } else {
                    metadata.insert(key.clone(), value.to_string());
                }
            }
        }
    }

    Ok(LogEntry {
        timestamp,
        level,
        message,
        metadata,
        source: "json".to_string(),
    })
}

/// Parse text log line
fn parse_text_log_line(line: &str) -> Result<LogEntry> {
    // Simple regex-based parsing for common log formats
    // This is a simplified implementation
    
    let timestamp = chrono::Utc::now(); // Fallback to current time
    
    // Try to extract log level
    let level = if line.to_uppercase().contains("ERROR") {
        "error".to_string()
    } else if line.to_uppercase().contains("WARN") {
        "warn".to_string()
    } else if line.to_uppercase().contains("INFO") {
        "info".to_string()
    } else if line.to_uppercase().contains("DEBUG") {
        "debug".to_string()
    } else {
        "info".to_string()
    };

    let message = line.trim().to_string();
    let metadata = HashMap::new();

    Ok(LogEntry {
        timestamp,
        level,
        message,
        metadata,
        source: "text".to_string(),
    })
}

/// Get log statistics
pub async fn get_log_statistics(
    gateway_url: &str,
    instance_id: &str,
    time_range: Option<TimeRange>,
) -> Result<LogStatistics> {
    debug!(
        instance_id = instance_id,
        time_range = ?time_range,
        "Getting log statistics"
    );

    let mut stats_url = format!("{}/instances/{}/logs/stats", gateway_url, instance_id);
    
    if let Some(time_range) = time_range {
        stats_url.push_str(&format!(
            "?start={}&end={}",
            time_range.start.timestamp(),
            time_range.end.timestamp()
        ));
    }

    let client = reqwest::Client::new();
    let response = client
        .get(&stats_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get log statistics")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get log statistics: {}", error_text));
    }

    let stats: LogStatistics = response.json().await
        .context("Failed to parse log statistics")?;

    debug!(
        instance_id = instance_id,
        total_entries = stats.total_entries,
        "Retrieved log statistics"
    );

    Ok(stats)
}

/// Search logs
pub async fn search_logs(
    gateway_url: &str,
    instance_id: &str,
    search_query: &LogSearchQuery,
) -> Result<Vec<LogEntry>> {
    debug!(
        instance_id = instance_id,
        query = search_query.query,
        "Searching logs"
    );

    let search_url = format!("{}/instances/{}/logs/search", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .post(&search_url)
        .header("Content-Type", "application/json")
        .json(search_query)
        .send()
        .await
        .context("Failed to search logs")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to search logs: {}", error_text));
    }

    let results: Vec<LogEntry> = response.json().await
        .context("Failed to parse search results")?;

    debug!(
        instance_id = instance_id,
        result_count = results.len(),
        "Log search completed"
    );

    Ok(results)
}

/// Log entry structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub level: String,
    pub message: String,
    pub metadata: HashMap<String, String>,
    pub source: String,
}

/// Log filter
#[derive(Debug, Serialize)]
pub struct LogFilter {
    pub level: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub until: Option<chrono::DateTime<chrono::Utc>>,
    pub pattern: Option<String>,
    pub limit: Option<u32>,
}

/// Time range
#[derive(Debug, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
}

/// Log search query
#[derive(Debug, Serialize)]
pub struct LogSearchQuery {
    pub query: String,
    pub level: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub until: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Log statistics
#[derive(Debug, Deserialize)]
pub struct LogStatistics {
    pub total_entries: u64,
    pub error_count: u64,
    pub warn_count: u64,
    pub info_count: u64,
    pub debug_count: u64,
    pub time_range: Option<TimeRange>,
    pub top_errors: Vec<String>,
    pub top_warnings: Vec<String>,
}
