use anyhow::Result;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Request, StatusCode,
        header::{CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    response::Response,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use crate::common::build_test_app;

mod common;

#[sqlx::test]
async fn register_should_return_bad_request_when_email_is_invalid(database: PgPool) -> Result<()> {
    let app = build_test_app(database).await?;

    let response = post_json(
        app,
        "/api/v1/auth/register",
        json!({
            "email": "invalid-email-id",
            "password": "correct horse battery staple",
            "display_name": "Rust Learner"
        }),
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;

    let expected_details = json!([
        {
            "field": "email",
            "message": "Email address is invalid"
        }
    ]);

    assert_eq!(
        (
            status,
            payload.get("code").and_then(Value::as_str),
            payload.get("message").and_then(Value::as_str),
            payload.get("details")
        ),
        (
            StatusCode::BAD_REQUEST,
            Some("VALIDATION_ERROR"),
            Some("The request contains invalid fields"),
            Some(&expected_details)
        )
    );

    Ok(())
}

#[sqlx::test]
async fn login_should_return_unauthorized_when_credentials_are_invalid(
    database: PgPool,
) -> Result<()> {
    let app = build_test_app(database).await?;

    let response = post_json(
        app,
        "/api/v1/auth/login",
        json!({
            "email": "missing@example.com",
            "password": "wrong password value"
        }),
    )
    .await?;

    let status = response.status();

    let has_bearer_challenge = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == "Bearer");

    let payload = response_json(response).await?;

    assert_eq!(
        (
            status,
            has_bearer_challenge,
            payload.get("code").and_then(Value::as_str),
            payload.get("message").and_then(Value::as_str),
            payload.get("details").is_none()
        ),
        (
            StatusCode::UNAUTHORIZED,
            true,
            Some("INVALID_CREDENTIALS"),
            Some("The email or password is incorrect"),
            true
        )
    );

    Ok(())
}

async fn post_json(app: Router, uri: &str, payload: Value) -> Result<Response> {
    let body = serde_json::to_vec(&payload)?;

    let request = Request::post(uri)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))?;

    let response = app.oneshot(request).await?;

    Ok(response)
}

async fn response_json(response: Response) -> Result<Value> {
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    let payload = serde_json::from_slice(&body)?;

    Ok(payload)
}
