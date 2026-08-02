use utoipa::{
    Modify, OpenApi,
    openapi::{
        Components,
        security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    },
};

use crate::health::HealthResponse;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Rust Catalog API",
        description = "REST API for authentication, profiles, and product management"
    ),
    paths(
        crate::health::live,
        crate::health::ready
    ),
    components(
        schemas(HealthResponse)
    ),
    modifiers(&SecurityAddon),
    tags(
        (
            name = "Health",
            description = "Application health and readiness endpoints"
        )
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
}
