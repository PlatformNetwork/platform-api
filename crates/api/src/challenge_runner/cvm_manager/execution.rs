//! CVM command execution functionality

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

use super::core::CvmCommandResult;

/// Execute command in CVM
pub async fn execute_cvm_command(
    gateway_url: &str,
    instance_id: &str,
    command: &str,
) -> Result<CvmCommandResult> {
    debug!(
        instance_id = instance_id,
        command = command,
        "Executing CVM command"
    );

    let execute_url = format!("{}/instances/{}/execute", gateway_url, instance_id);

    #[derive(Serialize)]
    struct ExecuteRequest {
        command: String,
        timeout_seconds: u64,
        working_directory: Option<String>,
        environment: HashMap<String, String>,
    }

    let execute_request = ExecuteRequest {
        command: command.to_string(),
        timeout_seconds: 30,
        working_directory: None,
        environment: HashMap::new(),
    };

    let start_time = std::time::Instant::now();

    let client = reqwest::Client::new();
    let response = client
        .post(&execute_url)
        .header("Content-Type", "application/json")
        .json(&execute_request)
        .send()
        .await
        .context("Failed to execute CVM command")?;

    let execution_time_ms = start_time.elapsed().as_millis() as u64;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to execute CVM command: {}", error_text));
    }

    let result: CommandExecutionResult = response.json().await
        .context("Failed to parse command result")?;

    let command_result = CvmCommandResult {
        exit_code: result.exit_code,
        stdout: result.stdout,
        stderr: result.stderr,
        execution_time_ms,
    };

    debug!(
        instance_id = instance_id,
        exit_code = command_result.exit_code,
        execution_time_ms = command_result.execution_time_ms,
        "CVM command executed"
    );

    Ok(command_result)
}

/// Execute command with custom options
pub async fn execute_cvm_command_with_options(
    gateway_url: &str,
    instance_id: &str,
    command: &str,
    options: &ExecutionOptions,
) -> Result<CvmCommandResult> {
    debug!(
        instance_id = instance_id,
        command = command,
        timeout_seconds = options.timeout_seconds,
        "Executing CVM command with options"
    );

    let execute_url = format!("{}/instances/{}/execute", gateway_url, instance_id);

    #[derive(Serialize)]
    struct ExecuteRequest {
        command: String,
        timeout_seconds: u64,
        working_directory: Option<String>,
        environment: HashMap<String, String>,
        shell: Option<String>,
        user: Option<String>,
    }

    let execute_request = ExecuteRequest {
        command: command.to_string(),
        timeout_seconds: options.timeout_seconds,
        working_directory: options.working_directory.clone(),
        environment: options.environment.clone(),
        shell: options.shell.clone(),
        user: options.user.clone(),
    };

    let start_time = std::time::Instant::now();

    let client = reqwest::Client::new();
    let response = client
        .post(&execute_url)
        .header("Content-Type", "application/json")
        .json(&execute_request)
        .send()
        .await
        .context("Failed to execute CVM command")?;

    let execution_time_ms = start_time.elapsed().as_millis() as u64;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to execute CVM command: {}", error_text));
    }

    let result: CommandExecutionResult = response.json().await
        .context("Failed to parse command result")?;

    let command_result = CvmCommandResult {
        exit_code: result.exit_code,
        stdout: result.stdout,
        stderr: result.stderr,
        execution_time_ms,
    };

    debug!(
        instance_id = instance_id,
        exit_code = command_result.exit_code,
        execution_time_ms = command_result.execution_time_ms,
        "CVM command executed with options"
    );

    Ok(command_result)
}

/// Execute batch commands
pub async fn execute_cvm_batch_commands(
    gateway_url: &str,
    instance_id: &str,
    commands: Vec<String>,
    options: Option<ExecutionOptions>,
) -> Result<Vec<CvmCommandResult>> {
    info!(
        instance_id = instance_id,
        command_count = commands.len(),
        "Executing CVM batch commands"
    );

    let batch_url = format!("{}/instances/{}/execute/batch", gateway_url, instance_id);

    #[derive(Serialize)]
    struct BatchRequest {
        commands: Vec<String>,
        timeout_seconds: u64,
        working_directory: Option<String>,
        environment: HashMap<String, String>,
        shell: Option<String>,
        user: Option<String>,
        stop_on_error: bool,
    }

    let default_options = ExecutionOptions::default();
    let opts = options.unwrap_or(default_options);

    let batch_request = BatchRequest {
        commands,
        timeout_seconds: opts.timeout_seconds,
        working_directory: opts.working_directory,
        environment: opts.environment,
        shell: opts.shell,
        user: opts.user,
        stop_on_error: true, // Stop on error by default
    };

    let client = reqwest::Client::new();
    let response = client
        .post(&batch_url)
        .header("Content-Type", "application/json")
        .json(&batch_request)
        .send()
        .await
        .context("Failed to execute batch commands")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to execute batch commands: {}", error_text));
    }

    let results: Vec<CommandExecutionResult> = response.json().await
        .context("Failed to parse batch results")?;

    let command_results: Vec<CvmCommandResult> = results
        .into_iter()
        .map(|result| CvmCommandResult {
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
            execution_time_ms: result.execution_time_ms.unwrap_or(0),
        })
        .collect();

    info!(
        instance_id = instance_id,
        result_count = command_results.len(),
        "Batch commands executed"
    );

    Ok(command_results)
}

/// Execute interactive command
pub async fn execute_cvm_interactive_command(
    gateway_url: &str,
    instance_id: &str,
    command: &str,
) -> Result<tokio::sync::mpsc::Receiver<String>> {
    info!(
        instance_id = instance_id,
        command = command,
        "Starting interactive CVM command"
    );

    let (tx, rx) = tokio::sync::mpsc::channel(1000);

    let interactive_url = format!("{}/instances/{}/execute/interactive", gateway_url, instance_id);

    // Spawn interactive task
    let instance_id = instance_id.to_string();
    let command = command.to_string();
    tokio::spawn(async move {
        if let Err(e) = interactive_command_task(interactive_url, command, tx).await {
            error!(
                instance_id = instance_id,
                error = %e,
                "Interactive command task failed"
            );
        }
    });

    Ok(rx)
}

/// Interactive command background task
async fn interactive_command_task(
    interactive_url: String,
    command: String,
    tx: tokio::sync::mpsc::Sender<String>,
) -> Result<()> {
    use futures_util::StreamExt;

    #[derive(Serialize)]
    struct InteractiveRequest {
        command: String,
    }

    let request = InteractiveRequest { command };

    let client = reqwest::Client::new();
    let response = client
        .post(&interactive_url)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .context("Failed to start interactive command")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Interactive command failed: {}", response.status()));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("Failed to read interactive chunk")?;
        
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        
        // Process complete lines
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if !line.trim().is_empty() {
                if tx.send(line).await.is_err() {
                    // Channel closed, stop streaming
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Get command execution history
pub async fn get_command_execution_history(
    gateway_url: &str,
    instance_id: &str,
    limit: Option<u32>,
) -> Result<Vec<CommandHistoryEntry>> {
    debug!(
        instance_id = instance_id,
        limit = ?limit,
        "Getting command execution history"
    );

    let mut history_url = format!("{}/instances/{}/execute/history", gateway_url, instance_id);
    
    if let Some(limit) = limit {
        history_url.push_str(&format!("?limit={}", limit));
    }

    let client = reqwest::Client::new();
    let response = client
        .get(&history_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get command history")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get command history: {}", error_text));
    }

    let history: Vec<CommandHistoryEntry> = response.json().await
        .context("Failed to parse command history")?;

    debug!(
        instance_id = instance_id,
        entry_count = history.len(),
        "Retrieved command execution history"
    );

    Ok(history)
}

/// Command execution result from CVM
#[derive(Debug, Deserialize)]
struct CommandExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub execution_time_ms: Option<u64>,
    pub signal: Option<i32>,
}

/// Execution options
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionOptions {
    pub timeout_seconds: u64,
    pub working_directory: Option<String>,
    pub environment: HashMap<String, String>,
    pub shell: Option<String>,
    pub user: Option<String>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            timeout_seconds: 30,
            working_directory: None,
            environment: HashMap::new(),
            shell: Some("/bin/bash".to_string()),
            user: None,
        }
    }
}

/// Command history entry
#[derive(Debug, Deserialize)]
pub struct CommandHistoryEntry {
    pub id: String,
    pub command: String,
    pub exit_code: i32,
    pub execution_time_ms: u64,
    pub executed_at: chrono::DateTime<chrono::Utc>,
    pub user: Option<String>,
    pub working_directory: Option<String>,
}

/// Kill running command
pub async fn kill_running_command(
    gateway_url: &str,
    instance_id: &str,
    command_id: &str,
) -> Result<()> {
    info!(
        instance_id = instance_id,
        command_id = command_id,
        "Killing running command"
    );

    let kill_url = format!("{}/instances/{}/execute/{}/kill", gateway_url, instance_id, command_id);

    let client = reqwest::Client::new();
    let response = client
        .post(&kill_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to kill command")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to kill command: {}", error_text));
    }

    info!(
        instance_id = instance_id,
        command_id = command_id,
        "Command killed successfully"
    );

    Ok(())
}

/// Get running commands
pub async fn get_running_commands(
    gateway_url: &str,
    instance_id: &str,
) -> Result<Vec<RunningCommandInfo>> {
    debug!(
        instance_id = instance_id,
        "Getting running commands"
    );

    let running_url = format!("{}/instances/{}/execute/running", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&running_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get running commands")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get running commands: {}", error_text));
    }

    let running_commands: Vec<RunningCommandInfo> = response.json().await
        .context("Failed to parse running commands")?;

    debug!(
        instance_id = instance_id,
        running_count = running_commands.len(),
        "Retrieved running commands"
    );

    Ok(running_commands)
}

/// Running command information
#[derive(Debug, Deserialize)]
pub struct RunningCommandInfo {
    pub id: String,
    pub command: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub user: Option<String>,
    pub working_directory: Option<String>,
    pub pid: Option<u32>,
    pub cpu_usage_percent: Option<f64>,
    pub memory_usage_mb: Option<u64>,
}
