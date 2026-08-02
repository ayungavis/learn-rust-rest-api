use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    status: &'static str,
}

#[utoipa::path(
    get,
    path = "/api/v1/health/live",
    tag = "Health",
    responses(
        (
            status = 200,
            description = "Application process is running",
            body = HealthResponse
        )
    )
)]
pub async fn live() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[utoipa::path(
    get,
    path = "/api/v1/health/ready",
    tag = "Health",
    responses(
        (
            status = 200,
            description = "Application and database are ready",
            body = HealthResponse
        ),
        (
            status = 503,
            description = "Database is unavailable"
        )
    )
)]
pub async fn ready(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.database)
        .await
    {
        Ok(_) => (StatusCode::OK, Json(HealthResponse { status: "ready" })),
        Err(error) => {
            tracing::error!(error = ?error, "database readiness check failed");

            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthResponse {
                    status: "unavailable",
                }),
            )
        }
    }
}
