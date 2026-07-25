mod config;

use std::time::Duration;

use anyhow::{Context, Result};
use rust_catalog_api::{AppState, Mailer, build_router};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

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

    let state = AppState { database, mailer };

    let app = build_router(state);
    let listener = TcpListener::bind(config.address)
        .await
        .with_context(|| format!("failed to bind HTTP listener to {}", config.address))?;

    tracing::info!(address = %config.address, "API listening");
    axum::serve(listener, app).await?;

    Ok(())
}
