use anyhow::Result;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

use crate::common::build_test_app;

mod common;

#[tokio::test]
async fn live_should_return_ok_without_database_connection() -> Result<()> {
    let database = PgPoolOptions::new()
        .connect_lazy("postgres://rust_catalog:local_password@localhost/rust_catalog")?;

    let app = build_test_app(database).await?;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health/live")
                .body(Body::empty())?,
        )
        .await?;

    let status = response.status();
    let has_request_id = response.headers().contains_key("x-request-id");
    let body = to_bytes(response.into_body(), 1024).await?;
    let payload: Value = serde_json::from_slice(&body)?;

    assert_eq!(
        (status, has_request_id, payload),
        (StatusCode::OK, true, json!({ "status": "ok" }))
    );

    Ok(())
}

#[sqlx::test]
async fn ready_should_return_ok_when_database_is_available(database: PgPool) -> Result<()> {
    let app = build_test_app(database).await?;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health/ready")
                .body(Body::empty())?,
        )
        .await?;

    let status = response.status();
    let body = to_bytes(response.into_body(), 1024).await?;
    let payload: Value = serde_json::from_slice(&body)?;

    assert_eq!(
        (status, payload),
        (StatusCode::OK, json!({ "status": "ready" }))
    );

    Ok(())
}

#[tokio::test]
async fn unknown_route_should_return_consistent_error() -> Result<()> {
    let database = PgPoolOptions::new()
        .connect_lazy("postgres://rust_catalog:local_password@localhost/rust_catalog")?;

    let app = build_test_app(database).await?;

    let response = app
        .oneshot(Request::builder().uri("/unknown").body(Body::empty())?)
        .await?;

    let status = response.status();

    let header_request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let body = to_bytes(response.into_body(), 1024).await?;
    let payload: Value = serde_json::from_slice(&body)?;

    let body_request_id = payload
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let request_ids_match = header_request_id.is_some() && header_request_id == body_request_id;

    assert_eq!((status, request_ids_match), (StatusCode::NOT_FOUND, true));

    Ok(())
}
