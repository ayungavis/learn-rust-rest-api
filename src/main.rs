mod config;
mod health;
mod state;

use std::time::Duration;

use anyhow::{Context, Result};
use axum::{Router, routing::get};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;

use crate::{
    config::Config,
    health::{live, ready},
    state::AppState,
};

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
    let state = AppState { database };

    let app = Router::new()
        .route("/api/v1/health/live", get(live))
        .route("/api/v1/health/ready", get(ready))
        .with_state(state);
    let listener = TcpListener::bind(config.address)
        .await
        .with_context(|| format!("failed to bind HTTP listener to {}", config.address))?;

    tracing::info!(address = %config.address, "API listening");
    axum::serve(listener, app).await?;

    Ok(())
}
