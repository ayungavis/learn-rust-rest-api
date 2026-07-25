use argon2::{
    Argon2, PasswordHasher,
    password_hash::{
        SaltString,
        rand_core::{OsRng, RngCore},
    },
};
use axum::{Extension, Json, extract::State, http::StatusCode};
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

struct PendingConfirmation {
    email: String,
    token: String,
}

struct ConfirmationToken {
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

    let password_length = input.password.chars().count();
    if !(15..=128).contains(&password_length) {
        details.push(FieldError {
            field: "password",
            message: "Password must contain between 15 and 128 characters",
        });
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
) -> Result<Option<PendingConfirmation>, sqlx::Error> {
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

    let token = generate_confirmation_token();

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

    Ok(Some(PendingConfirmation {
        email: user.email,
        token: token.encoded,
    }))
}

fn generate_confirmation_token() -> ConfirmationToken {
    let mut raw = [0_u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut raw);

    ConfirmationToken {
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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use argon2::{
        Argon2,
        password_hash::{PasswordHash, PasswordVerifier},
    };

    use crate::auth::{decode_and_hash_token, generate_confirmation_token};

    use super::{RegisterRequest, hash_password, validate_registration};

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
        let token = generate_confirmation_token();

        let decoded_hash = decode_and_hash_token(&token.encoded)?;

        assert_eq!(decoded_hash, token.hash);

        Ok(())
    }
}
