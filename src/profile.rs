use axum::{Extension, Json, extract::State};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, prelude::FromRow};
use tower_http::request_id::RequestId;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    AppState,
    auth::AuthenticatedSession,
    error::{AppError, ErrorResponse, FieldError},
    password::{
        hash as hash_password, validation_error as password_validation_error,
        verify as verify_password,
    },
};

#[derive(Deserialize, ToSchema)]
pub struct UpdateProfileRequest {
    display_name: String,
}

#[derive(Serialize, ToSchema)]
pub struct ProfileResponse {
    id: String,
    email: String,
    display_name: String,
    email_verified: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

#[derive(Serialize, ToSchema)]
pub struct ChangePasswordResponse {
    message: &'static str,
}

#[derive(FromRow)]
struct Profile {
    id: Uuid,
    email: String,
    display_name: String,
    email_verified: bool,
}

impl From<Profile> for ProfileResponse {
    fn from(profile: Profile) -> Self {
        Self {
            id: profile.id.to_string(),
            email: profile.email,
            display_name: profile.display_name,
            email_verified: profile.email_verified,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/profile",
    tag = "Profile",
    security(
        ("bearer_auth" = [])
    ),
    responses(
        ( status = StatusCode::OK, description = "Current user profile", body = ProfileResponse),
        ( status = StatusCode::UNAUTHORIZED, description = "Bearer token is missing, invalid, or expired", body = ErrorResponse ),
        ( status = StatusCode::INTERNAL_SERVER_ERROR, description = "Profile could not be loaded", body = ErrorResponse )
    )
)]
pub async fn get_profile(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    session: AuthenticatedSession,
) -> Result<Json<ProfileResponse>, AppError> {
    let profile = find_profile(&state.database, session.user_id)
        .await
        .map_err(|error| AppError::internal(&request_id, "find_profile", &error))?;

    Ok(Json(profile.into()))
}

#[utoipa::path(
    patch,
    path = "/api/v1/profile",
    tag = "Profile",
    security(
        ("bearer_auth" = [])
    ),
    request_body = UpdateProfileRequest,
    responses(
        ( status = StatusCode::OK, description = "Profile updated successfully", body = ProfileResponse ),
        ( status = StatusCode::BAD_REQUEST, description = "Display name did not pass validation", body = ErrorResponse ),
        ( status = StatusCode::UNAUTHORIZED, description = "Bearer token is missing, invalid, or expired", body = ErrorResponse ),
        ( status = StatusCode::INTERNAL_SERVER_ERROR, description = "Profile could not be updated", body = ErrorResponse )
    )
)]
pub async fn update_profile(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    session: AuthenticatedSession,
    Json(input): Json<UpdateProfileRequest>,
) -> Result<Json<ProfileResponse>, AppError> {
    let display_name = normalize_display_name(input.display_name)
        .map_err(|error| AppError::validation(&request_id, vec![error]))?;

    let profile = save_profile(&state.database, session.user_id, &display_name)
        .await
        .map_err(|error| AppError::internal(&request_id, "save_profile", &error))?;

    Ok(Json(profile.into()))
}

#[utoipa::path(
    put,
    path = "/api/v1/profile/password",
    tag = "Profile",
    security(
        ("bearer_auth" = [])
    ),
    request_body = ChangePasswordRequest,
    responses(
        ( status = StatusCode::OK, description = "Password changed and existing session revoked", body = ChangePasswordResponse ),
        ( status = StatusCode::BAD_REQUEST, description = "Password validation failed or current password is incorrect", body = ErrorResponse ),
        ( status = StatusCode::UNAUTHORIZED, description = "Bearer token is missing, invalid, or expired", body = ErrorResponse ),
        ( status = StatusCode::INTERNAL_SERVER_ERROR, description = "Password could not be changed", body = ErrorResponse )
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    session: AuthenticatedSession,
    Json(input): Json<ChangePasswordRequest>,
) -> Result<Json<ChangePasswordResponse>, AppError> {
    if let Some(mut error) = password_validation_error(&input.new_password) {
        error.field = "new_password";

        return Err(AppError::validation(&request_id, vec![error]));
    }

    if input.current_password == input.new_password {
        return Err(AppError::validation(
            &request_id,
            vec![FieldError {
                field: "new_password",
                message: "New password must be different from current password",
            }],
        ));
    }

    let current_hash = find_password_hash(&state.database, session.user_id)
        .await
        .map_err(|error| AppError::internal(&request_id, "find_password_hash", &error))?;

    let current_password_valid = verify_password(input.current_password, current_hash)
        .await
        .map_err(|error| AppError::internal(&request_id, "verify_password", &error))?;

    if !current_password_valid {
        return Err(AppError::current_password_incorrect(&request_id));
    }

    let new_password_hash = hash_password(input.new_password)
        .await
        .map_err(|error| AppError::internal(&request_id, "hash_password", &error))?;

    save_password_and_revoke_sessions(&state.database, session.user_id, &new_password_hash)
        .await
        .map_err(|error| {
            AppError::internal(&request_id, "save_password_and_revoke_sessions", &error)
        })?;

    Ok(Json(ChangePasswordResponse {
        message: "You have successfully changed your password, please re-log in.",
    }))
}

fn normalize_display_name(display_name: String) -> Result<String, FieldError> {
    let display_name = display_name.trim();

    if (1..=100).contains(&display_name.chars().count()) {
        return Ok(display_name.to_owned());
    }

    Err(FieldError {
        field: "display_name",
        message: "Display name must contain between 1 and 100 characters ",
    })
}

async fn find_profile(database: &PgPool, user_id: Uuid) -> Result<Profile, sqlx::Error> {
    sqlx::query_as::<_, Profile>(
        r#"
        SELECT
            id,
            email,
            display_name,
            email_verified_at IS NOT NULL
                AS email_verified
        FROM users
        WHERE id = $1
        "#,
    )
    .bind(user_id)
    .fetch_one(database)
    .await
}

async fn save_profile(
    database: &PgPool,
    user_id: Uuid,
    display_name: &str,
) -> Result<Profile, sqlx::Error> {
    sqlx::query_as::<_, Profile>(
        r#"
        UPDATE users
        SET display_name = $1,
            updated_at = now()
        WHERE id = $2
        RETURNING
            id,
            email,
            display_name,
            email_verified_at IS NOT NULL
                AS email_verified
        "#,
    )
    .bind(display_name)
    .bind(user_id)
    .fetch_one(database)
    .await
}

async fn find_password_hash(database: &PgPool, user_id: Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT password_hash
        FROM users
        WHERE id = $1
        "#,
    )
    .bind(user_id)
    .fetch_one(database)
    .await
}

async fn save_password_and_revoke_sessions(
    database: &PgPool,
    user_id: Uuid,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    let mut transaction = database.begin().await?;

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
        SET revoked_at = COALESCE(revoked_at, now())
        WHERE user_id = $1
            AND revoked_at IS NULL
        "#,
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::normalize_display_name;

    #[test]
    fn display_name_should_be_trimmed() -> Result<()> {
        let display_name = normalize_display_name("    Rust Learner   ".to_owned())
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;

        assert_eq!(display_name, "Rust Learner");

        Ok(())
    }
}
