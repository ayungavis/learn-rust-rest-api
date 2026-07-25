use std::time::Duration;

use axum::{
    BoxError, Extension, Router,
    error_handling::HandleErrorLayer,
    routing::{get, post},
};
pub use mail::Mailer;
pub use state::AppState;
use tower::{ServiceBuilder, timeout::TimeoutLayer};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::{
    auth::{confirm_email, register},
    error::AppError,
    health::{live, ready},
};

mod auth;
mod error;
mod health;
pub mod mail;
mod state;

pub fn build_router(state: AppState) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(false))
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(HandleErrorLayer::new(handle_middleware_error))
        .layer(TimeoutLayer::new(Duration::from_secs(10)));

    Router::new()
        .route("/api/v1/health/live", get(live))
        .route("/api/v1/health/ready", get(ready))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/confirm-email", post(confirm_email))
        .fallback(route_not_found)
        .layer(middleware)
        .with_state(state)
}

async fn route_not_found(Extension(request_id): Extension<RequestId>) -> AppError {
    AppError::not_found(&request_id)
}

async fn handle_middleware_error(
    Extension(request_id): Extension<RequestId>,
    error: BoxError,
) -> AppError {
    if error.is::<tower::timeout::error::Elapsed>() {
        return AppError::request_timeout(&request_id);
    }

    tracing::error!(error = ?error, "unhandled middleware error");
    AppError::internal(&request_id, "middleware", error.as_ref())
}
