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
    product::{ListProductsResponse, PaginationResponse, ProductRequest, ProductResponse},
    profile::{
        ChangePasswordRequest, ChangePasswordResponse, ProfileResponse, UpdateProfileRequest,
    },
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Rust Catalog API",
        description = "REST API for authentication, profiles, and product management"
    ),
    paths(
        // Health
        crate::health::live,
        crate::health::ready,
        // Authentication
        crate::auth::register,
        crate::auth::confirm_email,
        crate::auth::login,
        crate::auth::logout,
        crate::auth::forgot_password,
        crate::auth::reset_password,
        // Profile
        crate::profile::get_profile,
        crate::profile::update_profile,
        crate::profile::change_password,
        // Products
        crate::product::list_products,
        crate::product::get_product,
        crate::product::create_product,
        crate::product::update_product,
        crate::product::delete_product,
        crate::product::upload_product_image
    ),
    components(
        schemas(
            // Common
            ErrorResponse,
            PaginationResponse,
            // Health
            HealthResponse,
            // Authentication
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
            // Profile
            ProfileResponse,
            UpdateProfileRequest,
            ChangePasswordRequest,
            ChangePasswordResponse,
            // Products
            ProductRequest,
            ProductResponse,
            ListProductsResponse,
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
        (
            name = "Profile",
            description = "Current user profile and password management"
        ),
        (
            name = "Products",
            description = "Public product catalog and authenticated product management"
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
            "/api/v1/auth/logout",
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

    #[test]
    fn openapi_should_include_profile_paths() {
        let document = ApiDoc::openapi();
        let paths = &document.paths.paths;
        let profile_paths = ["/api/v1/profile", "/api/v1/profile/password"];

        assert!(profile_paths.iter().all(|path| paths.contains_key(*path)))
    }

    #[test]
    fn openapi_should_include_product_paths() {
        let document = ApiDoc::openapi();
        let paths = &document.paths.paths;
        let product_paths = [
            "/api/v1/products",
            "/api/v1/products/{product_id}",
            "/api/v1/products/{product_id}/image",
        ];

        assert!(product_paths.iter().all(|path| paths.contains_key(*path)))
    }
}
