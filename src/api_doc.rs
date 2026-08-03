use utoipa::{
    Modify, OpenApi,
    openapi::{
        Components,
        security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    },
};

use crate::{
    auth::{
        ConfirmEmailRequest, ConfirmEmailResponse, ForgotPasswordRequest, ForgotPasswordResponse,
        LoginRequest, LoginResponse, RegisterRequest, RegisterResponse, ResetPasswordRequest,
        ResetPasswordResponse,
    },
    error::ErrorResponse,
    health::HealthResponse,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Rust Catalog API",
        description = "REST API for authentication, profiles, and product management"
    ),
    paths(
        crate::health::live,
        crate::health::ready,
        crate::auth::register,
        crate::auth::confirm_email,
        crate::auth::login,
        crate::auth::logout,
        crate::auth::forgot_password,
        crate::auth::reset_password,
    ),
    components(
        schemas(
            HealthResponse,
            ErrorResponse,
            RegisterRequest,
            RegisterResponse,
            ConfirmEmailRequest,
            ConfirmEmailResponse,
            LoginRequest,
            LoginResponse,
            ForgotPasswordRequest,
            ForgotPasswordResponse,
            ResetPasswordRequest,
            ResetPasswordResponse,
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (
            name = "Health",
            description = "Application health and readiness endpoints"
        ),
        (
            name = "Authentication",
            description = "Registration, sessions, email confirmation, and password recovery"
        ),
    )
)]
pub(crate) struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Components::new);

        components.add_security_scheme(
            "bearer_auth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("Opaque session token")
                    .build(),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use utoipa::OpenApi;

    use super::ApiDoc;

    #[test]
    fn openapi_should_include_liveness_path() {
        assert!(
            ApiDoc::openapi()
                .paths
                .paths
                .contains_key("/api/v1/health/live")
        )
    }

    #[test]
    fn openapi_should_include_authentication_paths() {
        let document = ApiDoc::openapi();
        let paths = &document.paths.paths;
        let authentication_paths = [
            "/api/v1/auth/register",
            "/api/v1/auth/login",
            "/api/v1/auth/confirm-email",
            "/api/v1/auth/forgot-password",
            "/api/v1/auth/reset-password",
        ];

        assert!(
            authentication_paths
                .iter()
                .all(|path| paths.contains_key(*path))
        )
    }
}
