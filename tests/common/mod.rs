use anyhow::Result;
use axum::Router;
use rust_catalog_api::{AppState, Mailer, ObjectStorage, build_router};
use sqlx::PgPool;

pub async fn build_test_app(database: PgPool) -> Result<Router> {
    let mailer = Mailer::new(
        "smtp://localhost:1025",
        "Rust Catalog <noreply@example.com>",
        "http://localhost:5173".to_owned(),
    )?;

    let storage = ObjectStorage::new_test().await;

    Ok(build_router(AppState {
        database,
        mailer,
        storage,
    }))
}
