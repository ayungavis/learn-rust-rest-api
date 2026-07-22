use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use thiserror::Error;
use tower_http::request_id::RequestId;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("route not found")]
    NotFound { request_id: String },
    #[error("request timed out")]
    RequestTimeout { request_id: String },
    #[error("internal server error")]
    Internal { request_id: String },
}

#[derive(Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: &'static str,
    request_id: String,
}

impl AppError {
    pub fn not_found(request_id: &RequestId) -> Self {
        Self::NotFound {
            request_id: request_id_value(request_id),
        }
    }

    pub fn request_timeout(request_id: &RequestId) -> Self {
        Self::RequestTimeout {
            request_id: request_id_value(request_id),
        }
    }

    pub fn internal(request_id: &RequestId) -> Self {
        Self::Internal {
            request_id: request_id_value(request_id),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message, request_id) = match self {
            Self::NotFound { request_id } => (
                StatusCode::NOT_FOUND,
                "ROUTE_NOT_FOUND",
                "The requested route does not exist",
                request_id,
            ),
            Self::RequestTimeout { request_id } => (
                StatusCode::REQUEST_TIMEOUT,
                "REQUEST_TIMEOUT",
                "The request took too long",
                request_id,
            ),
            Self::Internal { request_id } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                "An internal error occured",
                request_id,
            ),
        };
        (
            status,
            Json(ErrorResponse {
                code,
                message,
                request_id,
            }),
        )
            .into_response()
    }
}

fn request_id_value(request_id: &RequestId) -> String {
    match request_id.header_value().to_str() {
        Ok(value) => value.to_owned(),
        Err(error) => {
            tracing::error!(error = ?error, "request ID contains invalid characters");
            "invalid-request-id".to_owned()
        }
    }
}
