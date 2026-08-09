use anyhow::Result;
use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
        request::Builder,
    },
    response::Response,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::build_test_app;

mod common;

const VERIFIED_USER_EMAIL: &str = "learner@example.com";
const VERIFIED_USER_PASSWORD: &str = "correct horse battery staple";
const VERIFIED_USER_DISPLAY_NAME: &str = "Rust Learner";
const UPDATED_USER_DISPLAY_NAME: &str = "Rust API Learner";
const NEW_VERIFIED_USER_PASSWORD: &str = "new correct horse battery staple";

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

#[sqlx::test]
async fn verified_user_should_log_in_and_read_profile(database: PgPool) -> Result<()> {
    let user_id = insert_verified_user(&database).await?;
    let app = build_test_app(database).await?;

    let login_response = post_json(
        app.clone(),
        "/api/v1/auth/login",
        json!({
            "email": VERIFIED_USER_EMAIL,
            "password": VERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let login_status = login_response.status();
    let login_payload = response_json(login_response).await?;

    let Some(access_token) = login_payload.get("access_token").and_then(Value::as_str) else {
        anyhow::bail!("login response does not contain access token: {login_payload}");
    };

    let profile_request = Request::get("/api/v1/profile")
        .header(AUTHORIZATION, format!("Bearer {access_token}"))
        .body(Body::empty())?;

    let profile_response = app.oneshot(profile_request).await?;

    let profile_status = profile_response.status();
    let profile_payload = response_json(profile_response).await?;

    assert_eq!(
        (
            login_status,
            !access_token.is_empty(),
            login_payload.get("token_type").and_then(Value::as_str),
            login_payload.get("expires_in").and_then(Value::as_u64),
            profile_status,
            profile_payload
        ),
        (
            StatusCode::OK,
            true,
            Some("Bearer"),
            Some(604_800),
            StatusCode::OK,
            json!({
                "id": user_id.to_string(),
                "email": VERIFIED_USER_EMAIL,
                "display_name": VERIFIED_USER_DISPLAY_NAME,
                "email_verified": true
            })
        )
    );

    Ok(())
}

#[sqlx::test]
async fn profile_should_return_unauthorized_without_bearer_token(database: PgPool) -> Result<()> {
    let app = build_test_app(database).await?;

    let response = app
        .oneshot(Request::get("/api/v1/profile").body(Body::empty())?)
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
            Some("AUTHENTICATION_REQUIRED"),
            Some("A valid Bearer token is required"),
            true
        )
    );

    Ok(())
}

#[sqlx::test]
async fn logout_should_revoke_current_session(database: PgPool) -> Result<()> {
    insert_verified_user(&database).await?;

    let app = build_test_app(database).await?;

    let access_token = login_verified_user(app.clone()).await?;

    let logout_request = Request::post("/api/v1/auth/logout")
        .header(AUTHORIZATION, format!("Bearer {access_token}"))
        .body(Body::empty())?;

    let logout_response = app.clone().oneshot(logout_request).await?;
    let logout_status = logout_response.status();

    let profile_request = Request::get("/api/v1/profile")
        .header(AUTHORIZATION, format!("Bearer {access_token}"))
        .body(Body::empty())?;

    let profile_response = app.oneshot(profile_request).await?;
    let profile_status = profile_response.status();

    let has_bearer_challenge = profile_response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == "Bearer");

    let profile_payload = response_json(profile_response).await?;

    assert_eq!(
        (
            logout_status,
            profile_status,
            has_bearer_challenge,
            profile_payload.get("code").and_then(Value::as_str),
            profile_payload.get("message").and_then(Value::as_str)
        ),
        (
            StatusCode::NO_CONTENT,
            StatusCode::UNAUTHORIZED,
            true,
            Some("AUTHENTICATION_REQUIRED"),
            Some("A valid Bearer token is required")
        )
    );

    Ok(())
}

#[sqlx::test]
async fn profile_update_should_return_and_persist_normalized_display_name(
    database: PgPool,
) -> Result<()> {
    let user_id = insert_verified_user(&database).await?;
    let app = build_test_app(database).await?;
    let access_token = login_verified_user(app.clone()).await?;

    let update_response = send_json(
        app.clone(),
        Request::patch("/api/v1/profile").header(AUTHORIZATION, format!("Bearer {access_token}")),
        json!({
            "display_name": format!("    {UPDATED_USER_DISPLAY_NAME}    ")
        }),
    )
    .await?;

    let update_status = update_response.status();
    let update_payload = response_json(update_response).await?;

    let profile_response = app
        .oneshot(
            Request::get("/api/v1/profile")
                .header(AUTHORIZATION, format!("Bearer {access_token}"))
                .body(Body::empty())?,
        )
        .await?;

    let profile_status = profile_response.status();
    let profile_payload = response_json(profile_response).await?;

    let expected_profile = json!({
        "id": user_id.to_string(),
        "email": VERIFIED_USER_EMAIL,
        "display_name": UPDATED_USER_DISPLAY_NAME,
        "email_verified": true
    });

    assert_eq!(
        (
            update_status,
            &update_payload,
            profile_status,
            &profile_payload
        ),
        (
            StatusCode::OK,
            &expected_profile,
            StatusCode::OK,
            &expected_profile
        )
    );

    Ok(())
}

#[sqlx::test]
async fn password_change_should_replace_credentials_and_revoke_existing_session(
    database: PgPool,
) -> Result<()> {
    insert_verified_user(&database).await?;

    let app = build_test_app(database).await?;
    let access_token = login_verified_user(app.clone()).await?;

    let change_response = send_json(
        app.clone(),
        Request::put("/api/v1/profile/password")
            .header(AUTHORIZATION, format!("Bearer {access_token}")),
        json!({
            "current_password": VERIFIED_USER_PASSWORD,
            "new_password": NEW_VERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let change_status = change_response.status();
    let change_payload = response_json(change_response).await?;

    let revoked_session_response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/profile")
                .header(AUTHORIZATION, format!("Bearer {access_token}"))
                .body(Body::empty())?,
        )
        .await?;

    let revoked_session_status = revoked_session_response.status();

    let old_password_response = post_json(
        app.clone(),
        "/api/v1/auth/login",
        json!({
            "email": VERIFIED_USER_EMAIL,
            "password": VERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let old_password_status = old_password_response.status();

    let new_password_response = post_json(
        app.clone(),
        "/api/v1/auth/login",
        json!({
            "email": VERIFIED_USER_EMAIL,
            "password": NEW_VERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let new_password_status = new_password_response.status();

    assert_eq!(
        (
            change_status,
            change_payload,
            revoked_session_status,
            old_password_status,
            new_password_status,
        ),
        (
            StatusCode::OK,
            json!({
                "message": "You have successfully changed your password, please re-log in."
            }),
            StatusCode::UNAUTHORIZED,
            StatusCode::UNAUTHORIZED,
            StatusCode::OK
        )
    );

    Ok(())
}

#[sqlx::test]
async fn profile_update_should_return_validation_error_when_display_name_is_blank(
    database: PgPool,
) -> Result<()> {
    insert_verified_user(&database).await?;

    let app = build_test_app(database).await?;
    let access_token = login_verified_user(app.clone()).await?;

    let response = send_json(
        app,
        Request::patch("/api/v1/profile").header(AUTHORIZATION, format!("Bearer {access_token}")),
        json!({
            "display_name": "       "
        }),
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;

    let expected_result = json!([{
        "field": "display_name",
        "message": "Display name must contain between 1 and 100 characters"
    }]);

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
            Some(&expected_result)
        )
    );

    Ok(())
}

#[sqlx::test]
async fn password_change_should_reject_incorrect_current_password_without_revoking_session(
    database: PgPool,
) -> Result<()> {
    insert_verified_user(&database).await?;

    let app = build_test_app(database).await?;
    let access_token = login_verified_user(app.clone()).await?;

    let change_response = send_json(
        app.clone(),
        Request::put("/api/v1/profile/password")
            .header(AUTHORIZATION, format!("Bearer {access_token}")),
        json!({
            "current_password": "incorrect current password",
            "new_password": NEW_VERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let change_status = change_response.status();
    let change_payload = response_json(change_response).await?;

    let profile_response = app
        .oneshot(
            Request::get("/api/v1/profile")
                .header(AUTHORIZATION, format!("Bearer {access_token}"))
                .body(Body::empty())?,
        )
        .await?;

    let profile_status = profile_response.status();

    assert_eq!(
        (
            change_status,
            change_payload.get("code").and_then(Value::as_str),
            change_payload.get("message").and_then(Value::as_str),
            change_payload.get("details").is_none(),
            profile_status,
        ),
        (
            StatusCode::BAD_REQUEST,
            Some("CURRENT_PASSWORD_INCORRECT"),
            Some("The current password is incorrect"),
            true,
            StatusCode::OK
        )
    );

    Ok(())
}

#[sqlx::test]
async fn password_change_should_return_validation_error_when_new_password_is_too_short(
    database: PgPool,
) -> Result<()> {
    insert_verified_user(&database).await?;

    let app = build_test_app(database).await?;
    let access_token = login_verified_user(app.clone()).await?;

    let response = send_json(
        app,
        Request::put("/api/v1/profile/password")
            .header(AUTHORIZATION, format!("Bearer {access_token}")),
        json!({
            "current_password": VERIFIED_USER_PASSWORD,
            "new_password": "too short"
        }),
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;

    let expected_result = json!([{
        "field": "new_password",
        "message": "Password must contain between 15 and 128 characters"
    }]);

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
            Some(&expected_result)
        )
    );

    Ok(())
}

async fn login_verified_user(app: Router) -> Result<String> {
    let response = post_json(
        app,
        "/api/v1/auth/login",
        json!({
            "email": VERIFIED_USER_EMAIL,
            "password": VERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;

    if status != StatusCode::OK {
        anyhow::bail!("test setup failed: login returned {status}: {payload}");
    }

    let Some(access_token) = payload.get("access_token").and_then(Value::as_str) else {
        anyhow::bail!("login response does not contain access token: {payload}");
    };

    Ok(access_token.to_owned())
}

async fn insert_verified_user(database: &PgPool) -> Result<Uuid> {
    let user_id = Uuid::now_v7();

    let password_hash = tokio::task::spawn_blocking(|| {
        let salt = SaltString::generate(&mut OsRng);

        Argon2::default()
            .hash_password(VERIFIED_USER_PASSWORD.as_bytes(), &salt)
            .map(|hash| hash.to_string())
    })
    .await??;

    sqlx::query(
        r#"
        INSERT INTO users (
            id,
            email,
            password_hash,
            display_name,
            email_verified_at
        )
        VALUES (
            $1,
            $2,
            $3,
            $4,
            now()
        )
        "#,
    )
    .bind(user_id)
    .bind(VERIFIED_USER_EMAIL)
    .bind(password_hash)
    .bind(VERIFIED_USER_DISPLAY_NAME)
    .execute(database)
    .await?;

    Ok(user_id)
}

async fn post_json(app: Router, uri: &str, payload: Value) -> Result<Response> {
    send_json(app, Request::post(uri), payload).await
}

async fn send_json(app: Router, request_builder: Builder, payload: Value) -> Result<Response> {
    let body = serde_json::to_vec(&payload)?;

    let request = request_builder
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
