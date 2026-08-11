use anyhow::Result;
use axum::{
    Router,
    body::Body,
    http::{
        Request, StatusCode,
        header::{AUTHORIZATION, WWW_AUTHENTICATE},
    },
    response::Response,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use crate::common::{
    VERIFIED_USER_DISPLAY_NAME, VERIFIED_USER_EMAIL, VERIFIED_USER_PASSWORD, build_test_app,
    build_test_app_with_mailer, insert_verified_user, login_verified_user, response_json,
    send_json,
};

mod common;

const UPDATED_USER_DISPLAY_NAME: &str = "Rust API Learner";
const NEW_VERIFIED_USER_PASSWORD: &str = "new correct horse battery staple";

const UNVERIFIED_USER_EMAIL: &str = "new-learner@example.com";
const UNVERIFIED_USER_PASSWORD: &str = "another correct horse battery staple";

const CONFIRMATION_URL_PREFIX: &str = "http://localhost:5173/confirm-email?token=";
const RESET_PASSWORD_URL_PREFIX: &str = "http://localhost:5173/reset-password?token=";

#[sqlx::test]
async fn registered_user_should_confirm_email_and_log_in(database: PgPool) -> Result<()> {
    let (app, mailer) = build_test_app_with_mailer(database).await?;

    let register_response = post_json(
        app.clone(),
        "/api/v1/auth/register",
        json!({
            "email": UNVERIFIED_USER_EMAIL,
            "password": UNVERIFIED_USER_PASSWORD,
            "display_name": "New Rust Learner"
        }),
    )
    .await?;

    let register_status = register_response.status();
    let register_payload = response_json(register_response).await?;

    let messages = mailer.test_messages().await?;

    let [message] = messages.as_slice() else {
        anyhow::bail!("expected one confirmation email, got {}", messages.len());
    };

    let token = email_token(message, CONFIRMATION_URL_PREFIX)?;

    let unverified_login_response = post_json(
        app.clone(),
        "/api/v1/auth/login",
        json!({
            "email": UNVERIFIED_USER_EMAIL,
            "password": UNVERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let unverified_login_status = unverified_login_response.status();
    let unverified_login_payload = response_json(unverified_login_response).await?;

    let confirm_response = post_json(
        app.clone(),
        "/api/v1/auth/confirm-email",
        json!({
            "token": token
        }),
    )
    .await?;

    let confirm_status = confirm_response.status();
    let confirm_payload = response_json(confirm_response).await?;

    let verified_login_response = post_json(
        app,
        "/api/v1/auth/login",
        json!({
            "email": UNVERIFIED_USER_EMAIL,
            "password": UNVERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let verified_login_status = verified_login_response.status();

    assert_eq!(
        (
            register_status,
            register_payload,
            unverified_login_status,
            unverified_login_payload.get("code").and_then(Value::as_str),
            confirm_status,
            confirm_payload,
            verified_login_status
        ),
        (
            StatusCode::ACCEPTED,
            json!({
                "message": "Registration request accepted"
            }),
            StatusCode::FORBIDDEN,
            Some("EMAIL_NOT_VERIFIED"),
            StatusCode::OK,
            json!({
                "message": "Email confirmed"
            }),
            StatusCode::OK
        )
    );

    Ok(())
}

#[sqlx::test]
async fn verified_user_should_reset_password_and_revoke_existing_session(
    database: PgPool,
) -> Result<()> {
    insert_verified_user(&database).await?;

    let (app, mailer) = build_test_app_with_mailer(database).await?;

    let existing_access_token = login_verified_user(app.clone()).await?;

    let forgot_response = post_json(
        app.clone(),
        "/api/v1/auth/forgot-password",
        json!({
            "email": VERIFIED_USER_EMAIL
        }),
    )
    .await?;

    let forgot_status = forgot_response.status();
    let forgot_payload = response_json(forgot_response).await?;

    let messages = mailer.test_messages().await?;

    let [message] = messages.as_slice() else {
        anyhow::bail!(
            "
            expected one password reset email, got {}",
            messages.len()
        );
    };

    let reset_token = email_token(message, RESET_PASSWORD_URL_PREFIX)?;

    let reset_response = post_json(
        app.clone(),
        "/api/v1/auth/reset-password",
        json!({
            "token": reset_token,
            "new_password": NEW_VERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let reset_status = reset_response.status();
    let reset_payload = response_json(reset_response).await?;

    let revoked_session_response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/profile")
                .header(AUTHORIZATION, format!("Bearer {existing_access_token}"))
                .body(Body::empty())?,
        )
        .await?;

    let revoked_session_status = revoked_session_response.status();
    let revoked_session_payload = response_json(revoked_session_response).await?;

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
    let old_password_payload = response_json(old_password_response).await?;

    let new_password_response = post_json(
        app,
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
            forgot_status,
            forgot_payload,
            reset_status,
            reset_payload,
            revoked_session_status,
            revoked_session_payload.get("code").and_then(Value::as_str),
            old_password_status,
            old_password_payload.get("code").and_then(Value::as_str),
            new_password_status
        ),
        (
            StatusCode::ACCEPTED,
            json!({
                "message": "If the account exists, password reset instructions will be sent to your email."
            }),
            StatusCode::OK,
            json!({
                "message": "Password reset successfully"
            }),
            StatusCode::UNAUTHORIZED,
            Some("AUTHENTICATION_REQUIRED"),
            StatusCode::UNAUTHORIZED,
            Some("INVALID_CREDENTIALS"),
            StatusCode::OK
        )
    );

    Ok(())
}

#[sqlx::test]
async fn password_reset_token_should_be_single_use(database: PgPool) -> Result<()> {
    insert_verified_user(&database).await?;

    let (app, mailer) = build_test_app_with_mailer(database).await?;

    let forgot_response = post_json(
        app.clone(),
        "/api/v1/auth/forgot-password",
        json!({
            "email": VERIFIED_USER_EMAIL
        }),
    )
    .await?;

    let forgot_status = forgot_response.status();
    if forgot_status != StatusCode::ACCEPTED {
        let payload = response_json(forgot_response).await?;

        anyhow::bail!(
            "test setup failed: forgot password returned \
            {forgot_status}: {payload}"
        );
    };

    let messages = mailer.test_messages().await?;

    let [message] = messages.as_slice() else {
        anyhow::bail!("expected one password reset email, got {}", messages.len());
    };

    let reset_token = email_token(message, RESET_PASSWORD_URL_PREFIX)?;

    let first_reset_response = post_json(
        app.clone(),
        "/api/v1/auth/reset-password",
        json!({
            "token": &reset_token,
            "new_password": NEW_VERIFIED_USER_PASSWORD
        }),
    )
    .await?;

    let first_reset_status = first_reset_response.status();

    let reused_token_response = post_json(
        app,
        "/api/v1/auth/reset-password",
        json!({
            "token": reset_token,
            "new_password": "yet another correct horse battery staple"
        }),
    )
    .await?;

    let reused_token_status = reused_token_response.status();
    let reused_token_payload = response_json(reused_token_response).await?;

    assert_eq!(
        (
            first_reset_status,
            reused_token_status,
            reused_token_payload.get("code").and_then(Value::as_str),
            reused_token_payload.get("message").and_then(Value::as_str),
            reused_token_payload.get("details").is_none()
        ),
        (
            StatusCode::OK,
            StatusCode::UNAUTHORIZED,
            Some("INVALID_OR_EXPIRED_TOKEN"),
            Some("The password reset token is invalid or expired"),
            true
        )
    );

    Ok(())
}

#[sqlx::test]
async fn forgot_password_should_not_reveal_missing_account(database: PgPool) -> Result<()> {
    let (app, mailer) = build_test_app_with_mailer(database).await?;

    let response = post_json(
        app,
        "/api/v1/auth/forgot-password",
        json!({
            "email": "missing@example.com"
        }),
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;
    let messages = mailer.test_messages().await?;

    assert_eq!(
        (status, payload, messages.is_empty()),
        (
            StatusCode::ACCEPTED,
            json!({
                "message": "If the account exists, password reset instructions will be sent to your email."
            }),
            true
        )
    );

    Ok(())
}

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

async fn post_json(app: Router, uri: &str, payload: Value) -> Result<Response> {
    send_json(app, Request::post(uri), payload).await
}

fn email_token(message: &str, url_prefix: &str) -> Result<String> {
    let decoded_message = message
        .replace("=\r\n", "")
        .replace("=\n", "")
        .replace("=3D", "=");

    let Some((_, content)) = decoded_message.split_once(url_prefix) else {
        anyhow::bail!("email does not contain the expected URL");
    };

    let Some(token) = content.split_whitespace().next() else {
        anyhow::bail!("confirmation email does not contain a token");
    };

    Ok(token.to_owned())
}
