mod config;

use std::time::Duration;

use anyhow::{Context, Result};
use rust_catalog_api::{AppState, Mailer, ObjectStorage, build_router, run_object_cleanup_worker};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    dotenvy::dotenv().ok();

    let config = Config::from_env()?;
    let database = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(2))
        .connect(&config.database_url)
        .await
        .context("failed to connect to database")?;

    sqlx::migrate!()
        .run(&database)
        .await
        .context("failed to run database migrations")?;

    let mailer = Mailer::new(&config.smtp_url, &config.mail_from, config.frontend_url)
        .context("failed to configure email transport")?;

    let storage = ObjectStorage::from_env().await?;

    let state = AppState {
        database,
        mailer,
        storage,
    };

    let cancellation = CancellationToken::new();

    let cleanup_worker = tokio::spawn(run_object_cleanup_worker(
        state.database.clone(),
        state.storage.clone(),
        cancellation.child_token(),
    ));

    let app = build_router(state);
    let listener = TcpListener::bind(config.address)
        .await
        .with_context(|| format!("failed to bind HTTP listener to {}", config.address))?;

    tracing::info!(address = %config.address, "API listening");

    let server_cancellation = cancellation.clone();

    let server_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    tracing::info!("shutdown signal received")
                }
                Err(error) => {
                    tracing::error!(
                        error = ?error,
                        "failed to listen for shutdown signal"
                    )
                }
            }

            server_cancellation.cancel();
        })
        .await;

    cancellation.cancel();

    cleanup_worker
        .await
        .context("object cleanup worker task failed")?;

    server_result.context("HTPP server failed")?;

    Ok(())
}
