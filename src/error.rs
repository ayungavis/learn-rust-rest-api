use std::fmt::Debug;

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::WWW_AUTHENTICATE},
    response::IntoResponse,
};
use serde::Serialize;
use thiserror::Error;
use tower_http::request_id::RequestId;
use utoipa::ToSchema;

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
    #[error("email confirmation token is invalid or expired")]
    InvalidConfirmationToken { request_id: String },
    #[error("email or password is incorrect")]
    InvalidCredentials { request_id: String },
    #[error("email address is not verified")]
    EmailNotVerified { request_id: String },
    #[error("authentication is required")]
    AuthenticationRequired { request_id: String },
    #[error("password reset token is invalid or expired")]
    InvalidPasswordResetToken { request_id: String },
    #[error("current password is incorrect")]
    CurrentPasswordIncorrect { request_id: String },
    #[error("product not found")]
    ProductNotFound { request_id: String },
    #[error("internal server error")]
    Internal { request_id: String },
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FieldError {
    pub field: &'static str,
    pub message: &'static str,
}

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
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

    pub fn invalid_confirmation_token(request_id: &RequestId) -> Self {
        Self::InvalidConfirmationToken {
            request_id: request_id_value(request_id),
        }
    }

    pub fn invalid_credentials(request_id: &RequestId) -> Self {
        Self::InvalidCredentials {
            request_id: request_id_value(request_id),
        }
    }

    pub fn email_not_verified(request_id: &RequestId) -> Self {
        Self::EmailNotVerified {
            request_id: request_id_value(request_id),
        }
    }

    pub fn authentication_required(request_id: &RequestId) -> Self {
        Self::AuthenticationRequired {
            request_id: request_id_value(request_id),
        }
    }

    pub fn missing_request_id() -> Self {
        tracing::error!("request ID extension is missing");

        Self::Internal {
            request_id: "missing-request-id".to_owned(),
        }
    }

    pub fn invalid_password_reset_token(request_id: &RequestId) -> Self {
        Self::InvalidPasswordResetToken {
            request_id: request_id_value(request_id),
        }
    }

    pub fn current_password_incorrect(request_id: &RequestId) -> Self {
        Self::CurrentPasswordIncorrect {
            request_id: request_id_value(request_id),
        }
    }

    pub fn product_not_found(request_id: &RequestId) -> Self {
        Self::ProductNotFound {
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
        let requires_authentication = matches!(
            &self,
            Self::InvalidCredentials { .. } | Self::AuthenticationRequired { .. }
        );

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
            Self::InvalidConfirmationToken { request_id } => (
                StatusCode::BAD_REQUEST,
                "INVALID_OR_EXPIRED_TOKEN",
                "The email confirmation token is invalid or expired",
                request_id,
                None,
            ),
            Self::InvalidCredentials { request_id } => (
                StatusCode::UNAUTHORIZED,
                "INVALID_CREDENTIALS",
                "The email or password is incorrect",
                request_id,
                None,
            ),
            Self::EmailNotVerified { request_id } => (
                StatusCode::FORBIDDEN,
                "EMAIL_NOT_VERIFIED",
                "Confirm your email before signing in",
                request_id,
                None,
            ),
            Self::AuthenticationRequired { request_id } => (
                StatusCode::UNAUTHORIZED,
                "AUTHENTICATION_REQUIRED",
                "A valid Bearer token is required",
                request_id,
                None,
            ),
            Self::InvalidPasswordResetToken { request_id } => (
                StatusCode::UNAUTHORIZED,
                "INVALID_OR_EXPIRED_TOKEN",
                "The password reset token is invalid or expired",
                request_id,
                None,
            ),
            Self::CurrentPasswordIncorrect { request_id } => (
                StatusCode::BAD_REQUEST,
                "CURRENT_PASSWORD_INCORRECT",
                "The current password is incorrect",
                request_id,
                None,
            ),
            Self::ProductNotFound { request_id } => (
                StatusCode::NOT_FOUND,
                "PRODUCT_NOT_FOUND",
                "The requested product does not exist",
                request_id,
                None,
            ),
            Self::Internal { request_id } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                "An internal error occurred",
                request_id,
                None,
            ),
        };

        let mut response = (
            status,
            Json(ErrorResponse {
                code,
                message,
                request_id,
                details,
            }),
        )
            .into_response();

        if requires_authentication {
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }

        response
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
