use axum::{Extension, Json, extract::State};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, prelude::FromRow};
use tower_http::request_id::RequestId;
use uuid::Uuid;

use crate::{
    AppState,
    auth::AuthenticatedSession,
    error::{AppError, FieldError},
};

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    display_name: String,
}

#[derive(Serialize)]
pub struct ProfileResponse {
    id: String,
    email: String,
    display_name: String,
    email_verified: bool,
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
