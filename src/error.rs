use std::fmt::Debug;

use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use thiserror::Error;
use tower_http::request_id::RequestId;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("request validation failed")]
    Validation {
        request_id: String,
        details: Vec<FieldError>,
    },
    #[error("route not found")]
    NotFound { request_id: String },
    #[error("request timed out")]
    RequestTimeout { request_id: String },
    #[error("internal server error")]
    Internal { request_id: String },
}

#[derive(Debug, Serialize)]
pub struct FieldError {
    pub field: &'static str,
    pub message: &'static str,
}

#[derive(Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: &'static str,
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Vec<FieldError>>,
}

impl AppError {
    pub fn validation(request_id: &RequestId, details: Vec<FieldError>) -> Self {
        Self::Validation {
            request_id: request_id_value(request_id),
            details,
        }
    }

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

    pub fn internal<E>(request_id: &RequestId, operation: &'static str, error: &E) -> Self
    where
        E: Debug + ?Sized,
    {
        tracing::error!(operation, error = ?error, "internal operation failed");

        Self::Internal {
            request_id: request_id_value(request_id),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message, request_id, details) = match self {
            Self::Validation {
                request_id,
                details,
            } => (
                StatusCode::BAD_REQUEST,
                "VALIDATION_ERROR",
                "The request contains invalid fields",
                request_id,
                Some(details),
            ),
            Self::NotFound { request_id } => (
                StatusCode::NOT_FOUND,
                "ROUTE_NOT_FOUND",
                "The requested route does not exist",
                request_id,
                None,
            ),
            Self::RequestTimeout { request_id } => (
                StatusCode::REQUEST_TIMEOUT,
                "REQUEST_TIMEOUT",
                "The request took too long",
                request_id,
                None,
            ),
            Self::Internal { request_id } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                "An internal error occured",
                request_id,
                None,
            ),
        };
        (
            status,
            Json(ErrorResponse {
                code,
                message,
                request_id,
                details,
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
