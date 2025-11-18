use anyhow::Result;
use clap::Parser;
use platform_api::{create_router, AppConfig, AppState};
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod tls;
use tls::serve_https;

/// Platform API Server
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    /// Server host
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Server port
    #[arg(long, default_value = "3000")]
    port: u16,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,

    /// TLS certificate path
    #[arg(long)]
    tls_cert: Option<String>,

    /// TLS private key path
    #[arg(long)]
    tls_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| args.log_level.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Platform API Server");

    // Load configuration
    let config = load_config(&args.config)?;

    // Create application state
    let state = AppState::new(config.clone()).await?;

    // Initialize ChallengeRunner if database pool is available
    let state = if let Some(pool) = &state.database_pool {
        use platform_api::challenge_runner::{ChallengeRunner, ChallengeRunnerConfig};
        let runner_config = ChallengeRunnerConfig::default();
        // Pass ORM gateway, validator_challenge_status, redis_client, and validator_connections to ChallengeRunner
        let runner = Arc::new(ChallengeRunner::new(
            runner_config,
            (**pool).clone(),
            state.orm_gateway.clone(),
            Some(state.validator_challenge_status.clone()),
            state.redis_client.clone(),
            Some(state.validator_connections.clone()),
        ));
        info!("ChallengeRunner initialized for auto-starting challenges with ORM bridge");

        // Clone state and set runner
        let mut new_state = state.clone();
        new_state.challenge_runner = Some(runner);
        new_state
    } else {
        state
    };

    // Start background task to sync challenges from PostgreSQL
    let state_arc = Arc::new(state);
    platform_api::background::start_challenge_sync_task(state_arc.clone());

    // Start background task to sync metagraph hotkeys from Bittensor chain
    platform_api::background::start_metagraph_sync_task();

    // Create router
    let app = create_router((*state_arc).clone());

    // Start server - use config port from environment variable or args
    let port = if std::env::var("SERVER_PORT").is_ok() {
        config.server_port
    } else {
        args.port
    };
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Check if TLS is enabled
    if let (Some(cert_path), Some(key_path)) = (args.tls_cert, args.tls_key) {
        info!("Starting HTTPS server on {}", addr);
        serve_https(app, addr, &cert_path, &key_path).await?;
    } else {
        info!("Starting HTTP server on {}", addr);
        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
    }

    Ok(())
}

fn load_config(_path: &str) -> Result<AppConfig> {
    // For now, return a default configuration
    // In a real implementation, this would load from the specified file

    // Check if we're in dev mode (for logging purposes only)
    let dev_mode = env::var("DEV_MODE").unwrap_or_else(|_| "false".to_string()) == "true";

    // Generate random secrets in dev mode (not hardcoded)
    // In production, fail fast if secrets are missing
    let generate_random_key = || {
        use rand::RngCore;
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        hex::encode(key)
    };

    // Encryption disabled - no longer using STORAGE_ENCRYPTION_KEY or KBS_ENCRYPTION_KEY
    tracing::info!("Storage and KBS encryption disabled");

    Ok(AppConfig {
        server_port: env::var("SERVER_PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .expect("Invalid SERVER_PORT"),
        server_host: env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
        database_url: env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://localhost/platform".to_string()),
        storage_config: platform_api_storage::StorageConfig {
            backend_type: env::var("STORAGE_BACKEND").unwrap_or_else(|_| "postgres".to_string()),
            s3_bucket: Some("platform-storage".to_string()),
            s3_region: Some("us-east-1".to_string()),
            minio_endpoint: None,
            encryption_key: "disabled".to_string(),
        },
        attestation_config: platform_api_attestation::TdxConfig::from_env(),
        kbs_config: platform_api_kbs::KbsConfig {
            key_derivation_algorithm: "HKDF".to_string(),
            key_size: 256,
            session_timeout: 3600,
            max_sessions: 1000,
        },
        scheduler_config: platform_api_scheduler::SchedulerConfig {
            max_concurrent_jobs: 100,
            job_timeout: 1800,
            retry_attempts: 3,
            retry_delay: 60,
            cleanup_interval: 300,
        },
        builder_config: platform_api_builder::BuilderConfig {
            build_timeout: 1800,
            max_concurrent_builds: 10,
            docker_registry: "localhost:5000".to_string(),
            github_token: None,
            build_cache_size: 1024 * 1024 * 1024, // 1GB
        },
        metrics_config: platform_api::MetricsConfig {
            enabled: true,
            port: 9090,
            path: "/metrics".to_string(),
            collect_interval: 60,
        },
    })
}
