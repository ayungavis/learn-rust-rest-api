use axum::{
    Extension, Json,
    body::Bytes,
    extract::{
        Multipart, Path, Query, State,
        multipart::{MultipartError, MultipartRejection},
    },
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, prelude::FromRow};
use time::OffsetDateTime;
use tower_http::request_id::RequestId;
use uuid::Uuid;

use crate::{
    AppState,
    auth::AuthenticatedSession,
    error::{AppError, FieldError},
};

const MAX_PRICE_CENTS: i64 = 1_000_000_000;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024; // 5 MB

#[derive(Deserialize)]
pub struct ProductRequest {
    name: String,
    description: String,
    price_cents: i64,
}

#[derive(Serialize)]
pub struct ProductResponse {
    id: String,
    owner_id: String,
    name: String,
    description: String,
    price_cents: i64,
    image_url: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

#[derive(Deserialize)]
pub struct ListProductsQuery {
    limit: i64,
    offset: i64,
}

#[derive(Serialize)]
pub struct ListProductsResponse {
    data: Vec<ProductResponse>,
    pagination: PaginationResponse,
}

#[derive(Serialize)]
pub struct PaginationResponse {
    limit: i64,
    offset: i64,
    count: usize,
}

#[derive(Debug, thiserror::Error)]
enum ImageInputError {
    #[error("multipart body is invalid")]
    Multipart(#[from] MultipartError),
    #[error("image field is missing")]
    MissingImage,
    #[error("multipart field is not supported")]
    UnexpectedField,
    #[error("image field was provided more than once")]
    DuplicateImage,
    #[error("image exceeds the allowed size")]
    TooLarge,
    #[error("image format is unsupported")]
    UnsupportedFormat,
}

struct ValidatedImage {
    bytes: Bytes,
    content_type: &'static str,
}

struct ValidatedProductRequest {
    name: String,
    description: String,
    price_cents: i64,
}

struct Pagination {
    limit: i64,
    offset: i64,
}

#[derive(FromRow)]
struct Product {
    id: Uuid,
    owner_id: Uuid,
    name: String,
    description: String,
    price_cents: i64,
    image_key: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

#[derive(FromRow)]
struct DeletedProduct {
    image_key: Option<String>,
}

impl ProductResponse {
    fn from_product(product: Product, state: &AppState) -> Self {
        let image_url = product.image_key.as_deref().map(|key| {
            state
                .storage
                .public_url(key, product.updated_at.unix_timestamp_nanos())
        });

        Self {
            id: product.id.to_string(),
            owner_id: product.owner_id.to_string(),
            name: product.name,
            description: product.description,
            price_cents: product.price_cents,
            image_url,

            created_at: product.created_at,
            updated_at: product.updated_at,
        }
    }
}

impl ImageInputError {
    fn field_error(&self) -> FieldError {
        let message = match self {
            Self::Multipart(_) => "Multipart body is invalid or exceeds the allowed size",
            Self::MissingImage => "Multipart field named image is required",
            Self::UnexpectedField => "Only the image multipart field is accepted",
            Self::DuplicateImage => "Only one image may be uploaded",
            Self::TooLarge => "Image must not exceed 5 MB",
            Self::UnsupportedFormat => "Image must be JPEG, PNG, or WEBP",
        };

        FieldError {
            field: "image",
            message,
        }
    }
}

pub async fn list_products(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Query(query): Query<ListProductsQuery>,
) -> Result<Json<ListProductsResponse>, AppError> {
    let pagination =
        validate_pagination(query).map_err(|details| AppError::validation(&request_id, details))?;

    let products = find_products(&state.database, pagination.limit, pagination.offset)
        .await
        .map_err(|error| AppError::internal(&request_id, "find_products", &error))?;

    let data: Vec<ProductResponse> = products
        .into_iter()
        .map(|product| ProductResponse::from_product(product, &state))
        .collect();

    let count = data.len();

    Ok(Json(ListProductsResponse {
        data,
        pagination: PaginationResponse {
            limit: pagination.limit,
            offset: pagination.offset,
            count,
        },
    }))
}

pub async fn get_product(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(product_id): Path<String>,
) -> Result<Json<ProductResponse>, AppError> {
    let product_id = parse_product_id(&product_id)
        .map_err(|error| AppError::validation(&request_id, vec![error]))?;

    let product = find_product(&state.database, product_id)
        .await
        .map_err(|error| AppError::internal(&request_id, "find_product", &error))?
        .ok_or_else(|| AppError::product_not_found(&request_id))?;

    Ok(Json(ProductResponse::from_product(product, &state)))
}

pub async fn create_product(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    session: AuthenticatedSession,
    Json(input): Json<ProductRequest>,
) -> Result<(StatusCode, Json<ProductResponse>), AppError> {
    let product =
        validate_product(input).map_err(|details| AppError::validation(&request_id, details))?;

    let product = insert_product(&state.database, session.user_id, &product)
        .await
        .map_err(|error| AppError::internal(&request_id, "insert_product", &error))?;

    Ok((
        StatusCode::CREATED,
        Json(ProductResponse::from_product(product, &state)),
    ))
}

pub async fn update_product(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    session: AuthenticatedSession,
    Path(product_id): Path<String>,
    Json(input): Json<ProductRequest>,
) -> Result<Json<ProductResponse>, AppError> {
    let product_id = parse_product_id(&product_id)
        .map_err(|error| AppError::validation(&request_id, vec![error]))?;

    let input =
        validate_product(input).map_err(|details| AppError::validation(&request_id, details))?;

    let product = save_product(&state.database, product_id, session.user_id, &input)
        .await
        .map_err(|error| AppError::internal(&request_id, "save_product", &error))?
        .ok_or_else(|| AppError::product_not_found(&request_id))?;

    Ok(Json(ProductResponse::from_product(product, &state)))
}

pub async fn delete_product(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    session: AuthenticatedSession,
    Path(product_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let product_id = parse_product_id(&product_id)
        .map_err(|error| AppError::validation(&request_id, vec![error]))?;

    let deleted =
        delete_product_and_schedule_image_cleanup(&state.database, product_id, session.user_id)
            .await
            .map_err(|error| {
                AppError::internal(
                    &request_id,
                    "delete_product_and_schedule_image_cleanup",
                    &error,
                )
            })?;

    if !deleted {
        return Err(AppError::product_not_found(&request_id));
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn upload_product_image(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    session: AuthenticatedSession,
    Path(product_id): Path<String>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Json<ProductResponse>, AppError> {
    let product_id = parse_product_id(&product_id)
        .map_err(|error| AppError::validation(&request_id, vec![error]))?;

    ensure_product_owner(&state.database, product_id, session.user_id)
        .await
        .map_err(|error| AppError::internal(&request_id, "ensure_product_owner", &error))?
        .then_some(())
        .ok_or_else(|| AppError::product_not_found(&request_id))?;

    let multipart = multipart.map_err(|_| {
        AppError::validation(
            &request_id,
            vec![FieldError {
                field: "image",
                message: "Request must contain valid multipart form data",
            }],
        )
    })?;

    let image = read_image(multipart)
        .await
        .map_err(|error| AppError::validation(&request_id, vec![error.field_error()]))?;

    let object_key = format!("products/{product_id}/image");

    state
        .storage
        .upload(&object_key, image.content_type, image.bytes)
        .await
        .map_err(|error| AppError::internal(&request_id, "upload_product_image", &error))?;

    let product = save_product_image_key(&state.database, product_id, session.user_id, &object_key)
        .await
        .map_err(|error| AppError::internal(&request_id, "save_product_image_key", &error))?
        .ok_or_else(|| AppError::product_not_found(&request_id))?;

    Ok(Json(ProductResponse::from_product(product, &state)))
}

fn validate_product(input: ProductRequest) -> Result<ValidatedProductRequest, Vec<FieldError>> {
    let name = input.name.trim().to_owned();
    let description = input.description.trim().to_owned();
    let mut details = Vec::new();

    if !(1..=150).contains(&name.chars().count()) {
        details.push(FieldError {
            field: "name",
            message: "Name must contain between 1 and 150 characters",
        })
    }

    if description.chars().count() > 5_000 {
        details.push(FieldError {
            field: "description",
            message: "Description must not exceed 5000 characters",
        })
    }

    if !(0..=MAX_PRICE_CENTS).contains(&input.price_cents) {
        details.push(FieldError {
            field: "price_cents",
            message: "Price must contain a value between 0 and 1000000000 cents",
        })
    }

    if !details.is_empty() {
        return Err(details);
    }

    Ok(ValidatedProductRequest {
        name,
        description,
        price_cents: input.price_cents,
    })
}

fn validate_pagination(query: ListProductsQuery) -> Result<Pagination, Vec<FieldError>> {
    let mut details = Vec::new();

    if !(1..=100).contains(&query.limit) {
        details.push(FieldError {
            field: "limit",
            message: "Limit must containe a value between 1 and 100",
        })
    }

    if query.offset < 0 {
        details.push(FieldError {
            field: "offset",
            message: "Offset must be zero or greater",
        })
    }

    if !details.is_empty() {
        return Err(details);
    }

    Ok(Pagination {
        limit: query.limit,
        offset: query.offset,
    })
}

fn parse_product_id(product_id: &str) -> Result<Uuid, FieldError> {
    Uuid::parse_str(product_id).map_err(|_| FieldError {
        field: "product_id",
        message: "Product ID must be a valid UUID",
    })
}

async fn read_image(mut multipart: Multipart) -> Result<ValidatedImage, ImageInputError> {
    let mut image = None;

    while let Some(field) = multipart.next_field().await? {
        if field.name() != Some("image") {
            return Err(ImageInputError::UnexpectedField);
        }

        if image.is_some() {
            return Err(ImageInputError::DuplicateImage);
        }

        let bytes = field.bytes().await?;

        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(ImageInputError::TooLarge);
        }

        let content_type =
            detect_image_content_type(&bytes).ok_or(ImageInputError::UnsupportedFormat)?;

        image = Some(ValidatedImage {
            bytes,
            content_type,
        });
    }

    image.ok_or(ImageInputError::MissingImage)
}

fn detect_image_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }

    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some("image/png");
    }

    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Some("image/webp");
    }

    None
}

async fn find_products(
    database: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<Vec<Product>, sqlx::Error> {
    sqlx::query_as::<_, Product>(
        r#"
        SELECT
            id,
            owner_id,
            name,
            description,
            price_cents,
            image_key,
            created_at,
            updated_at
        FROM products
        ORDER BY created_at DESC, id DESC
        LIMIT $1
        OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(database)
    .await
}

async fn find_product(database: &PgPool, product_id: Uuid) -> Result<Option<Product>, sqlx::Error> {
    sqlx::query_as::<_, Product>(
        r#"
        SELECT
            id,
            owner_id,
            name,
            description,
            price_cents,
            image_key,
            created_at,
            updated_at
        FROM products
        WHERE id = $1
        "#,
    )
    .bind(product_id)
    .fetch_optional(database)
    .await
}

async fn insert_product(
    database: &PgPool,
    owner_id: Uuid,
    input: &ValidatedProductRequest,
) -> Result<Product, sqlx::Error> {
    sqlx::query_as::<_, Product>(
        r#"
        INSERT INTO products (
            id,
            owner_id,
            name,
            description,
            price_cents
        )
        VALUES ($1, $2, $3, $4, $5)
        RETURNING
            id,
            owner_id,
            name,
            description,
            price_cents,
            image_key,
            created_at,
            updated_at
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(owner_id)
    .bind(&input.name)
    .bind(&input.description)
    .bind(input.price_cents)
    .fetch_one(database)
    .await
}

async fn save_product(
    database: &PgPool,
    product_id: Uuid,
    owner_id: Uuid,
    input: &ValidatedProductRequest,
) -> Result<Option<Product>, sqlx::Error> {
    sqlx::query_as::<_, Product>(
        r#"
        UPDATE products
        SET name = $1,
            description = $2,
            price_cents = $3,
            updated_at = now()
        WHERE id = $4
            AND owner_id = $5
        RETURNING
            id,
            owner_id,
            name,
            description,
            price_cents,
            image_key,
            created_at,
            updated_at
        "#,
    )
    .bind(&input.name)
    .bind(&input.description)
    .bind(input.price_cents)
    .bind(product_id)
    .bind(owner_id)
    .fetch_optional(database)
    .await
}

async fn delete_product_and_schedule_image_cleanup(
    database: &PgPool,
    product_id: Uuid,
    owner_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let mut transaction = database.begin().await?;

    let deleted = sqlx::query_as::<_, DeletedProduct>(
        r#"
        DELETE FROM products
        WHERE id = $1
            AND owner_id = $2
        RETURNING image_key
        "#,
    )
    .bind(product_id)
    .bind(owner_id)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(deleted) = deleted else {
        transaction.rollback().await?;
        return Ok(false);
    };

    if let Some(image_key) = deleted.image_key {
        sqlx::query(
            r#"
            INSERT INTO object_deletion_jobs (id, object_key)
            VALUES ($1, $2)
            ON CONFLICT (object_key) DO NOTHING
            "#,
        )
        .bind(Uuid::now_v7())
        .bind(image_key)
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;

    Ok(true)
}

async fn ensure_product_owner(
    database: &PgPool,
    product_id: Uuid,
    owner_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM products
            WHERE id = $1
                AND owner_id = $2
        )
        "#,
    )
    .bind(product_id)
    .bind(owner_id)
    .fetch_one(database)
    .await
}

async fn save_product_image_key(
    database: &PgPool,
    product_id: Uuid,
    owner_id: Uuid,
    image_key: &str,
) -> Result<Option<Product>, sqlx::Error> {
    sqlx::query_as::<_, Product>(
        r#"
        UPDATE products
        SET image_key = $1,
            updated_at = now()
        WHERE id = $2
            AND owner_id = $3
        RETURNING
            id,
            owner_id,
            name,
            description,
            price_cents,
            image_key,
            created_at,
            updated_at
        "#,
    )
    .bind(image_key)
    .bind(product_id)
    .bind(owner_id)
    .fetch_optional(database)
    .await
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{
        ListProductsQuery, ProductRequest, detect_image_content_type, parse_product_id,
        validate_pagination, validate_product,
    };

    #[test]
    fn validate_product_should_trim_name() -> Result<()> {
        let product = validate_product(ProductRequest {
            name: "    Rust Book   ".to_owned(),
            description: "Learn Rust".to_owned(),
            price_cents: 150_000,
        })
        .map_err(|details| anyhow::anyhow!("validation failed: {details:?}"))?;

        assert_eq!(product.name, "Rust Book");

        Ok(())
    }

    #[test]
    fn validate_product_should_reject_negative_price() {
        let result = validate_product(ProductRequest {
            name: "Rust Book".to_owned(),
            description: "Learn Rust".to_owned(),
            price_cents: -1,
        });

        assert!(result.is_err())
    }

    #[test]
    fn validate_pagination_should_reject_limit_above_maximum() {
        let result = validate_pagination(ListProductsQuery {
            limit: 101,
            offset: 0,
        });

        assert!(result.is_err())
    }

    #[test]
    fn parse_product_id_should_reject_invalid_uuid() {
        let result = parse_product_id("invalid-id");

        assert!(result.is_err())
    }

    #[test]
    fn detect_image_content_type_should_accept_png_signature() {
        let bytes = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

        assert_eq!(detect_image_content_type(&bytes), Some("image/png"),);
    }

    #[test]
    fn detect_image_content_type_should_reject_untrusted_bytes() {
        assert_eq!(detect_image_content_type(b"<script>"), None,);
    }
}
