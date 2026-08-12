use std::time::Duration;

use axum::{
    BoxError, Extension, Router,
    error_handling::HandleErrorLayer,
    extract::DefaultBodyLimit,
    http::{
        HeaderName, HeaderValue, Method,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    routing::{get, post, put},
};
pub use mail::Mailer;
pub use object_cleanup::run_object_cleanup_worker;
pub use state::AppState;
pub use storage::ObjectStorage;
use tower::{ServiceBuilder, timeout::TimeoutLayer};
use tower_http::{
    cors::CorsLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::{
    api_doc::ApiDoc,
    auth::{confirm_email, forgot_password, login, logout, register, reset_password},
    error::AppError,
    health::{live, ready},
    product::{
        create_product, delete_product, get_product, list_products, update_product,
        upload_product_image,
    },
    profile::{change_password, get_profile, update_profile},
};

mod api_doc;
mod auth;
mod error;
mod health;
pub mod mail;
mod object_cleanup;
mod password;
mod product;
mod profile;
mod state;
mod storage;

pub fn build_router(state: AppState, frontend_origin: HeaderValue) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(frontend_origin)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE])
        .expose_headers([HeaderName::from_static("x-request-id")]);

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
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/forgot-password", post(forgot_password))
        .route("/api/v1/auth/reset-password", post(reset_password))
        .route("/api/v1/auth/confirm-email", post(confirm_email))
        .route("/api/v1/profile", get(get_profile).patch(update_profile))
        .route("/api/v1/profile/password", put(change_password))
        .route("/api/v1/products", get(list_products).post(create_product))
        .route(
            "/api/v1/products/{product_id}",
            get(get_product).put(update_product).delete(delete_product),
        )
        .route(
            "/api/v1/products/{product_id}/image",
            put(upload_product_image).layer(DefaultBodyLimit::max(6 * 1024 * 1024)),
        )
        .merge(SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .fallback(route_not_found)
        .layer(cors)
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
