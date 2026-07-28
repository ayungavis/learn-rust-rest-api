use argon2::{
    Argon2,
    password_hash::{
        Error as PasswordHashError, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
        rand_core::{OsRng, RngCore},
    },
};
use axum::{
    Extension, Json,
    extract::{FromRequestParts, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION, request::Parts},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, prelude::FromRow};
use thiserror::Error;
use tower_http::request_id::RequestId;
use uuid::Uuid;

use crate::{
    AppState,
    error::{AppError, FieldError},
};

const SESSION_TTL_SECONDS: u64 = 7 * 24 * 60 * 60; // 7 days

const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$Awsq1081k1ZMw1B1JrZpVQ$Y7JdeK5+ihsnGZLY00kSsZE1Ml31WK/FQld1hecbg0M";

const TOKEN_BYTES: usize = 32;

#[derive(Deserialize)]
pub struct RegisterRequest {
    email: String,
    password: String,
    display_name: String,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    message: &'static str,
}

#[derive(Deserialize)]
pub struct ConfirmEmailRequest {
    token: String,
}

#[derive(Serialize)]
pub struct ConfirmEmailResponse {
    message: &'static str,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
}

#[derive(Deserialize)]
pub struct ForgotPasswordRequest {
    email: String,
}

#[derive(Serialize)]
pub struct ForgotPasswordResponse {
    message: &'static str,
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    token: String,
    new_password: String,
}

#[derive(Serialize)]
pub struct ResetPasswordResponse {
    message: &'static str,
}

struct ValidatedRegistration {
    email: String,
    password: String,
    display_name: String,
}

#[derive(FromRow)]
struct RegistrationUser {
    id: Uuid,
    email: String,
    email_verified: bool,
}

#[derive(FromRow)]
struct LoginUser {
    id: Uuid,
    password_hash: String,
    email_verified: bool,
}

#[derive(FromRow)]
pub struct AuthenticatedSession {
    id: Uuid,
}

#[derive(FromRow)]
struct PasswordResetUser {
    id: Uuid,
    email: String,
}

struct PendingEmailToken {
    email: String,
    token: String,
}

struct OpaqueToken {
    encoded: String,
    hash: Vec<u8>,
}

#[derive(Debug, Error)]
enum HashPasswordError {
    #[error("password hashing task failed")]
    Task(#[from] tokio::task::JoinError),
    #[error("Password hashing failed")]
    Hash(#[from] argon2::password_hash::Error),
}

#[derive(Debug, Error)]
enum DecodeTokenError {
    #[error("token encoding is invalid")]
    Encoding(#[from] base64::DecodeError),
    #[error("token length is invalid")]
    Length,
}

#[derive(Debug, Error)]
enum VerifyPasswordError {
    #[error("password verification task failed")]
    Task(#[from] tokio::task::JoinError),
    #[error("password verification failed")]
    Verify(#[from] PasswordHashError),
}

pub async fn register(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(input): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), AppError> {
    let registration = validate_registration(input)
        .map_err(|details| AppError::validation(&request_id, details))?;

    let ValidatedRegistration {
        email,
        password,
        display_name,
    } = registration;

    let password_hash = hash_password(password)
        .await
        .map_err(|error| AppError::internal(&request_id, "hash_password", &error))?;

    let pending_confirmation =
        create_registration(&state.database, &email, &password_hash, &display_name)
            .await
            .map_err(|error| AppError::internal(&request_id, "create_registration", &error))?;

    if let Some(confirmation) = pending_confirmation {
        state
            .mailer
            .send_email_confirmation(&confirmation.email, &confirmation.token)
            .await
            .map_err(|error| AppError::internal(&request_id, "send_email_confirmation", &error))?;
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(RegisterResponse {
            message: "Registration request accepted",
        }),
    ))
}

pub async fn confirm_email(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(input): Json<ConfirmEmailRequest>,
) -> Result<Json<ConfirmEmailResponse>, AppError> {
    let token_hash = decode_and_hash_token(&input.token)
        .map_err(|_| AppError::invalid_confirmation_token(&request_id))?;

    let confirmed = consume_confirmation_token(&state.database, &token_hash)
        .await
        .map_err(|error| AppError::internal(&request_id, "consume_confirmation_token", &error))?;

    if !confirmed {
        return Err(AppError::invalid_confirmation_token(&request_id));
    }

    Ok(Json(ConfirmEmailResponse {
        message: "Email confirmed",
    }))
}

pub async fn login(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(input): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    if !(1..=128).contains(&input.password.chars().count()) {
        return Err(AppError::invalid_credentials(&request_id));
    }

    let email = input.email.trim().to_lowercase();

    let user = find_login_user(&state.database, &email)
        .await
        .map_err(|error| AppError::internal(&request_id, "find_login_user", &error))?;

    let password_hash = user
        .as_ref()
        .map_or(DUMMY_PASSWORD_HASH, |user| user.password_hash.as_str())
        .to_owned();

    let password_valid = verify_password(input.password, password_hash)
        .await
        .map_err(|error| AppError::internal(&request_id, "verify_password", &error))?;

    if !password_valid {
        return Err(AppError::invalid_credentials(&request_id));
    }

    let Some(user) = user else {
        return Err(AppError::invalid_credentials(&request_id));
    };

    if !user.email_verified {
        return Err(AppError::email_not_verified(&request_id));
    }

    let token = generate_opaque_token();

    insert_session(&state.database, user.id, &token.hash)
        .await
        .map_err(|error| AppError::internal(&request_id, "insert_session", &error))?;

    Ok(Json(LoginResponse {
        access_token: token.encoded,
        token_type: "Bearer",
        expires_in: SESSION_TTL_SECONDS,
    }))
}

pub async fn logout(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    session: AuthenticatedSession,
) -> Result<StatusCode, AppError> {
    revoke_session(&state.database, session.id)
        .await
        .map_err(|error| AppError::internal(&request_id, "revoke_session", &error))?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn forgot_password(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(input): Json<ForgotPasswordRequest>,
) -> Result<(StatusCode, Json<ForgotPasswordResponse>), AppError> {
    let email = input.email.trim().to_lowercase();

    let pending_reset = create_password_reset(&state.database, &email)
        .await
        .map_err(|error| AppError::internal(&request_id, "create_password_reset", &error))?;

    if let Some(reset) = pending_reset {
        state
            .mailer
            .send_password_reset(&reset.email, &reset.token)
            .await
            .map_err(|error| AppError::internal(&request_id, "send_password_reset", &error))?;
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(ForgotPasswordResponse {
            message: "If the account exists, password reset instructions will be sent to your email.",
        }),
    ))
}

pub async fn reset_password(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(input): Json<ResetPasswordRequest>,
) -> Result<Json<ResetPasswordResponse>, AppError> {
    if let Some(error) = password_validation_error(&input.new_password) {
        return Err(AppError::validation(&request_id, vec![error]));
    }

    let token_hash = decode_and_hash_token(&input.token)
        .map_err(|_| AppError::invalid_password_reset_token(&request_id))?;

    let password_hash = hash_password(input.new_password)
        .await
        .map_err(|error| AppError::internal(&request_id, "hash_password", &error))?;

    let reset = consume_password_reset_token(&state.database, &token_hash, &password_hash)
        .await
        .map_err(|error| AppError::internal(&request_id, "consume_password_reset_token", &error))?;

    if !reset {
        return Err(AppError::invalid_password_reset_token(&request_id));
    }

    Ok(Json(ResetPasswordResponse {
        message: "Password reset successfully",
    }))
}

impl FromRequestParts<AppState> for AuthenticatedSession {
    type Rejection = AppError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let request_id = parts
            .extensions
            .get::<RequestId>()
            .cloned()
            .ok_or_else(AppError::missing_request_id)?;

        let encoded_token = bearer_token(&parts.headers)
            .ok_or_else(|| AppError::authentication_required(&request_id))?;

        let token_hash = decode_and_hash_token(encoded_token)
            .map_err(|_| AppError::authentication_required(&request_id))?;

        let session = find_active_session(&state.database, &token_hash)
            .await
            .map_err(|error| AppError::internal(&request_id, "find_active_session", &error))?;

        session.ok_or_else(|| AppError::authentication_required(&request_id))
    }
}

fn validate_registration(input: RegisterRequest) -> Result<ValidatedRegistration, Vec<FieldError>> {
    let email = input.email.trim().to_lowercase();
    let display_name = input.display_name.trim().to_owned();
    let mut details = Vec::new();

    if !is_valid_email(&email) {
        details.push(FieldError {
            field: "email",
            message: "Email address is invalid",
        });
    }

    if let Some(error) = password_validation_error(&input.password) {
        details.push(error);
    }

    if !(1..=100).contains(&display_name.chars().count()) {
        details.push(FieldError {
            field: "display_name",
            message: "Display name must contain between 1 and 100 characters",
        });
    }

    if details.is_empty() {
        return Ok(ValidatedRegistration {
            email,
            password: input.password,
            display_name,
        });
    }

    Err(details)
}

fn is_valid_email(email: &str) -> bool {
    if !email.is_ascii()
        || email.len() > 254
        || email.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return false;
    }

    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };

    !local.is_empty()
        && local.len() <= 64
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@')
}

async fn hash_password(password: String) -> Result<String, HashPasswordError> {
    let password_hash = tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
    })
    .await??;

    Ok(password_hash)
}

async fn create_registration(
    database: &PgPool,
    email: &str,
    password_hash: &str,
    display_name: &str,
) -> Result<Option<PendingEmailToken>, sqlx::Error> {
    let mut transaction = database.begin().await?;

    let user = sqlx::query_as::<_, RegistrationUser>(
        r#"
        INSERT INTO users (
            id,
            email,
            password_hash,
            display_name
        )
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (email)
        DO UPDATE SET email = EXCLUDED.email
        RETURNING
            id,
            email,
            email_verified_at IS NOT NULL AS email_verified
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .fetch_one(&mut *transaction)
    .await?;

    if user.email_verified {
        transaction.commit().await?;
        return Ok(None);
    }

    let token = generate_opaque_token();

    sqlx::query(
        r#"
        UPDATE one_time_tokens
        SET used_at = now()
        WHERE user_id = $1
            AND purpose = 'email_verification'
            AND used_at IS NULL
        "#,
    )
    .bind(user.id)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO one_time_tokens (
            id,
            user_id,
            purpose,
            token_hash,
            expires_at
        )
        VALUES (
            $1,
            $2,
            'email_verification',
            $3,
            now() + interval '30 minutes'
        )
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(user.id)
    .bind(&token.hash)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    Ok(Some(PendingEmailToken {
        email: user.email,
        token: token.encoded,
    }))
}

fn generate_opaque_token() -> OpaqueToken {
    let mut raw = [0_u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut raw);

    OpaqueToken {
        encoded: URL_SAFE_NO_PAD.encode(raw),
        hash: Sha256::digest(raw).to_vec(),
    }
}

fn decode_and_hash_token(encoded: &str) -> Result<Vec<u8>, DecodeTokenError> {
    let raw = URL_SAFE_NO_PAD.decode(encoded)?;

    let raw: [u8; TOKEN_BYTES] = raw.try_into().map_err(|_| DecodeTokenError::Length)?;

    Ok(Sha256::digest(raw).to_vec())
}

async fn consume_confirmation_token(
    database: &PgPool,
    token_hash: &[u8],
) -> Result<bool, sqlx::Error> {
    let mut transaction = database.begin().await?;

    let user_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        UPDATE one_time_tokens
        SET used_at = now()
        WHERE token_hash = $1
            AND purpose = 'email_verification'
            AND used_at IS NULL
            AND expires_at > now()
        RETURNING user_id
        "#,
    )
    .bind(token_hash)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(user_id) = user_id else {
        transaction.rollback().await?;
        return Ok(false);
    };

    sqlx::query(
        r#"
        UPDATE users
        SET
            email_verified_at =
                COALESCE(email_verified_at, now()),
            updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    Ok(true)
}

async fn find_login_user(database: &PgPool, email: &str) -> Result<Option<LoginUser>, sqlx::Error> {
    sqlx::query_as::<_, LoginUser>(
        r#"
        SELECT
            id,
            password_hash,
            email_verified_at IS NOT NULL
                AS email_verified
        FROM users
        WHERE email = $1
        "#,
    )
    .bind(email)
    .fetch_optional(database)
    .await
}

async fn verify_password(
    password: String,
    encoded_hash: String,
) -> Result<bool, VerifyPasswordError> {
    let verified = tokio::task::spawn_blocking(move || {
        let parsed_hash = PasswordHash::new(&encoded_hash)?;

        match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
            Ok(()) => Ok(true),
            Err(PasswordHashError::Password) => Ok(false),
            Err(error) => Err(error),
        }
    })
    .await??;

    Ok(verified)
}

async fn insert_session(
    database: &PgPool,
    user_id: Uuid,
    token_hash: &[u8],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO sessions (
        id,
            user_id,
            token_hash,
            expires_at
        )
        VALUES (
            $1,
            $2,
            $3,
            now() + interval '7 days'
        )
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(token_hash)
    .execute(database)
    .await?;

    Ok(())
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let authorization = headers.get(AUTHORIZATION)?.to_str().ok()?;

    let (scheme, token) = authorization.split_once(" ")?;

    if scheme.eq_ignore_ascii_case("Bearer")
        && !token.is_empty()
        && !token.chars().any(char::is_whitespace)
    {
        return Some(token);
    }

    None
}

async fn find_active_session(
    database: &PgPool,
    token_hash: &[u8],
) -> Result<Option<AuthenticatedSession>, sqlx::Error> {
    sqlx::query_as::<_, AuthenticatedSession>(
        r#"
        SELECT id
        FROM sessions
        WHERE token_hash = $1
            AND revoked_at IS NULL
            AND expires_at > now()
        "#,
    )
    .bind(token_hash)
    .fetch_optional(database)
    .await
}

async fn revoke_session(database: &PgPool, session_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE sessions
        SET revoked_at =
            COALESCE(revoked_at, now())
        WHERE id = $1
        "#,
    )
    .bind(session_id)
    .execute(database)
    .await?;

    Ok(())
}

fn password_validation_error(password: &str) -> Option<FieldError> {
    if (15..=128).contains(&password.chars().count()) {
        return None;
    }

    Some(FieldError {
        field: "password",
        message: "Password must contain between 15 and 128 characters",
    })
}

async fn create_password_reset(
    database: &PgPool,
    email: &str,
) -> Result<Option<PendingEmailToken>, sqlx::Error> {
    let mut transaction = database.begin().await?;

    let user = sqlx::query_as::<_, PasswordResetUser>(
        r#"
        SELECT id, email
        FROM users
        WHERE email = $1
            AND email_verified_at IS NOT NULL
        FOR UPDATE
        "#,
    )
    .bind(email)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(user) = user else {
        transaction.rollback().await?;
        return Ok(None);
    };

    let token = generate_opaque_token();

    sqlx::query(
        r#"
        UPDATE one_time_tokens
        SET used_at = now()
        WHERE user_id = $1
            AND purpose = 'password_reset'
            AND used_at IS NULL
        "#,
    )
    .bind(user.id)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO one_time_tokens (
            id,
            user_id,
            purpose,
            token_hash,
            expires_at
        )
        VALUES (
            $1,
            $2,
            'password_reset',
            $3,
            now() + interval '30 minutes'
        )
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(user.id)
    .bind(&token.hash)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    Ok(Some(PendingEmailToken {
        email: user.email,
        token: token.encoded,
    }))
}

async fn consume_password_reset_token(
    database: &PgPool,
    token_hash: &[u8],
    password_hash: &str,
) -> Result<bool, sqlx::Error> {
    let mut transaction = database.begin().await?;

    let user_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        UPDATE one_time_tokens
        SET used_at = now()
        WHERE token_hash = $1
            AND purpose = 'password_reset'
            AND used_at IS NULL
            AND expires_at > now()
        RETURNING user_id
        "#,
    )
    .bind(token_hash)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(user_id) = user_id else {
        transaction.rollback().await?;
        return Ok(false);
    };

    sqlx::query(
        r#"
        UPDATE users
        SET password_hash = $1,
            updated_at = now()
        WHERE id = $2
        "#,
    )
    .bind(password_hash)
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        r#"
        UPDATE sessions
        SET revoked_at =
            COALESCE(revoked_at, now())
        WHERE user_id = $1
            AND revoked_at IS NULL
        "#,
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use argon2::{
        Argon2,
        password_hash::{PasswordHash, PasswordVerifier},
    };
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};

    use crate::auth::{decode_and_hash_token, generate_opaque_token, verify_password};

    use super::{
        RegisterRequest, bearer_token, hash_password, password_validation_error,
        validate_registration,
    };

    #[test]
    fn registration_validation_should_normalize_user_fields() -> Result<()> {
        let result = validate_registration(RegisterRequest {
            email: "  LEARNER@EXAMPLE.COM ".to_owned(),
            password: "correct horse battery staple".to_owned(),
            display_name: "  Rust Learner  ".to_owned(),
        })
        .map_err(|details| anyhow::anyhow!("validation failed: {details:?}"))?;

        assert_eq!(
            (result.email, result.display_name),
            ("learner@example.com".to_owned(), "Rust Learner".to_owned())
        );

        Ok(())
    }

    #[tokio::test]
    async fn password_hash_should_verify_original_password() -> Result<()> {
        let password = "correct horse battery staple".to_owned();
        let encoded_hash = hash_password(password.to_owned()).await?;
        let parsed_hash = PasswordHash::new(&encoded_hash)?;
        let verification = Argon2::default().verify_password(password.as_bytes(), &parsed_hash);

        assert!(verification.is_ok());

        Ok(())
    }

    #[test]
    fn confirmation_token_should_hash_to_stored_value() -> Result<()> {
        let token = generate_opaque_token();

        let decoded_hash = decode_and_hash_token(&token.encoded)?;

        assert_eq!(decoded_hash, token.hash);

        Ok(())
    }

    #[tokio::test]
    async fn password_verification_should_reject_wrong_password() -> Result<()> {
        let encoded_hash = hash_password("correct password value".to_owned()).await?;

        let verified = verify_password("wrong password value".to_owned(), encoded_hash).await?;

        assert!(!verified);

        Ok(())
    }

    #[test]
    fn bearer_token_should_extract_valid_token() {
        let mut headers = HeaderMap::new();

        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer valid-token"),
        );

        assert_eq!(bearer_token(&headers), Some("valid-token"))
    }

    #[test]
    fn password_validation_should_reject_short_password() {
        assert!(password_validation_error("too-short").is_some());
    }
}
