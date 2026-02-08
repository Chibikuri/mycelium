use std::sync::Arc;

use tokio::signal;

use crate::platform::Platform;
use crate::server::AppState;

/// Wait for a shutdown signal (SIGINT or SIGTERM).
pub async fn wait_for_shutdown() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C, initiating shutdown...");
        }
        _ = terminate => {
            tracing::info!("Received SIGTERM, initiating shutdown...");
        }
    }
}

/// Perform graceful shutdown: remove :working labels from all in-flight issues.
pub async fn graceful_shutdown(state: &Arc<AppState>) {
    tracing::info!("Starting graceful shutdown...");

    let in_flight_issues = state.get_in_flight_issues().await;

    if in_flight_issues.is_empty() {
        tracing::info!("No in-flight issues to clean up");
        return;
    }

    tracing::info!(
        count = in_flight_issues.len(),
        "Removing :working labels from in-flight issues"
    );

    let working_label = format!("{}:working", state.config.github.trigger_label);

    for issue in in_flight_issues {
        tracing::info!(
            repo = %issue.repo_full_name,
            issue = issue.issue_number,
            "Removing :working label"
        );

        if let Err(e) = state
            .platform
            .remove_label(
                issue.installation_id,
                &issue.repo_full_name,
                issue.issue_number,
                &working_label,
            )
            .await
        {
            tracing::warn!(
                repo = %issue.repo_full_name,
                issue = issue.issue_number,
                error = %e,
                "Failed to remove :working label during shutdown"
            );
        }
    }

    tracing::info!("Graceful shutdown complete");
}
