use anyhow::Result;
use axum::{
    body::{Body, to_bytes},
    http::{
        Method, Request, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS,
            ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, ORIGIN,
        },
    },
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

use crate::common::{FRONTEND_ORIGIN, build_test_app};

#[expect(
    dead_code,
    reason = "Shared integration-test helpers are used by other test crates"
)]
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

#[tokio::test]
async fn cors_preflight_should_allow_configured_frontend_origin() -> Result<()> {
    let database = PgPoolOptions::new()
        .connect_lazy("postgres://rust_catalog:local_password@localhost/rust_catalog")?;

    let app = build_test_app(database).await?;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/v1/products")
                .header(ORIGIN, FRONTEND_ORIGIN)
                .header(ACCESS_CONTROL_REQUEST_METHOD, Method::POST.as_str())
                .header(ACCESS_CONTROL_REQUEST_HEADERS, "authorization,content-type")
                .body(Body::empty())?,
        )
        .await?;

    let status = response.status();
    let headers = response.headers();

    let allows_origin = headers
        .get(ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_some_and(|value| value == FRONTEND_ORIGIN);

    let allows_post = headers
        .get(ACCESS_CONTROL_ALLOW_METHODS)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(",").any(|method| method.trim() == "POST"));

    let allowed_headers = headers
        .get(ACCESS_CONTROL_ALLOW_HEADERS)
        .and_then(|value| value.to_str().ok());

    let allows_authorization = allowed_headers.is_some_and(|value| {
        value
            .split(",")
            .any(|header| header.trim().eq_ignore_ascii_case("authorization"))
    });

    let allows_content_type = allowed_headers.is_some_and(|value| {
        value
            .split(",")
            .any(|header| header.trim().eq_ignore_ascii_case("content-type"))
    });

    assert_eq!(
        (
            status,
            allows_origin,
            allows_post,
            allows_authorization,
            allows_content_type
        ),
        (StatusCode::OK, true, true, true, true)
    );

    Ok(())
}

#[tokio::test]
async fn cors_response_should_expose_request_id_to_frontend() -> Result<()> {
    let database = PgPoolOptions::new()
        .connect_lazy("postgres://rust_catalog:local_password@localhost/rust_catalog")?;

    let app = build_test_app(database).await?;

    let response = app
        .oneshot(
            Request::get("/api/v1/health/live")
                .header(ORIGIN, FRONTEND_ORIGIN)
                .body(Body::empty())?,
        )
        .await?;

    let headers = response.headers();

    let allows_origin = headers
        .get(ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_some_and(|value| value == FRONTEND_ORIGIN);

    let exposes_request_id = headers
        .get(ACCESS_CONTROL_EXPOSE_HEADERS)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(",")
                .any(|header| header.trim().eq_ignore_ascii_case("x-request-id"))
        });

    let has_request_id = headers.contains_key("x-request-id");

    assert_eq!(
        (allows_origin, exposes_request_id, has_request_id),
        (true, true, true)
    );

    Ok(())
}

#[tokio::test]
async fn cors_should_not_echo_unconfigured_origin() -> Result<()> {
    let database = PgPoolOptions::new()
        .connect_lazy("postgres://rust_catalog:local_password@localhost/rust_catalog")?;

    let app = build_test_app(database).await?;

    let response = app
        .oneshot(
            Request::get("/api/v1/health/live")
                .header(ORIGIN, "https://mailicious.example")
                .body(Body::empty())?,
        )
        .await?;

    let allowed_origin = response
        .headers()
        .get(ACCESS_CONTROL_ALLOW_ORIGIN)
        .and_then(|value| value.to_str().ok());

    assert_ne!(allowed_origin, Some("https://mailicious.example"));

    Ok(())
}
