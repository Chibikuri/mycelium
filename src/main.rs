use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use mycelium::config::AppConfig;
use mycelium::server::{create_router, AppState};

#[derive(Parser)]
#[command(name = "mycelium", about = "AI-powered GitHub issue resolver")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long)]
    config: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    let config = AppConfig::load(cli.config.as_deref())?;

    tracing::info!(
        host = %config.server.host,
        port = %config.server.port,
        "Starting Mycelium server"
    );

    let state = Arc::new(AppState::new(config.clone()).await?);

    // Start the task queue processor
    let queue_state = Arc::clone(&state);
    tokio::spawn(async move {
        mycelium::queue::run_queue_processor(queue_state).await;
    });

    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(format!(
        "{}:{}",
        config.server.host, config.server.port
    ))
    .await?;

    tracing::info!("Listening on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;

    Ok(())
}
