use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{Extension, Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use tower_http::request_id::RequestId;
use uuid::Uuid;

use crate::{
    AppState,
    error::{AppError, FieldError},
};

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

struct ValidatedRegistration {
    email: String,
    password: String,
    display_name: String,
}

#[derive(Debug, Error)]
enum HashPasswordError {
    #[error("password hashing task failed")]
    Task(#[from] tokio::task::JoinError),
    #[error("Password hashing failed")]
    Hash(#[from] argon2::password_hash::Error),
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

    match insert_user(&state.database, &email, &password_hash, &display_name).await {
        Ok(()) => {}
        Err(error) if is_unique_violation(&error) => {}
        Err(error) => {
            return Err(AppError::internal(
                &request_id,
                "insert_registered_user",
                &error,
            ));
        }
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(RegisterResponse {
            message: "Registration request accepted",
        }),
    ))
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

async fn insert_user(
    database: &PgPool,
    email: &str,
    password_hash: &str,
    display_name: &str,
) -> Result<(), sqlx::Error> {
    let uuid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, display_name) VALUES ($1, $2, $3, $4)",
    )
    .bind(uuid)
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .execute(database)
    .await?;

    Ok(())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(error) => error.is_unique_violation(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use argon2::{
        Argon2,
        password_hash::{PasswordHash, PasswordVerifier},
    };

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
}
