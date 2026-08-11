use anyhow::Result;
use axum::{
    Router,
    body::Body,
    http::{
        Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    response::Response,
};
use rust_catalog_api::ObjectStorage;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{
    build_test_app, build_test_app_with_storage, insert_verified_user,
    insert_verified_user_with_credentials, login_user, login_verified_user, response_json,
    send_json,
};

mod common;

const OTHER_USER_EMAIL: &str = "other@example.com";
const OTHER_USER_PASSWORD: &str = "another correct horse battery staple";
const OTHER_USER_DISPLAY_NAME: &str = "Other User";

const MULTIPART_BOUNDARY: &str = "rust-catalog-boundary";
const PNG_BYTES: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c,
    0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0xf8, 0x0f, 0x00,
    0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
    0xae, 0x42, 0x60, 0x82,
];
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

#[sqlx::test]
async fn created_product_should_be_visible_in_public_list(database: PgPool) -> Result<()> {
    let owner_id = insert_verified_user(&database).await?;
    let owner_id = owner_id.to_string();

    let app = build_test_app(database).await?;
    let access_token = login_verified_user(app.clone()).await?;

    let create_response = send_json(
        app.clone(),
        Request::post("/api/v1/products").header(AUTHORIZATION, format!("Bearer {access_token}")),
        json!({
            "name": "     Mechanical Keyboard    ",
            "description": "   A compact mechanical keyboard    ",
            "price_cents": 1_250_000
        }),
    )
    .await?;

    let create_status = create_response.status();
    let created_product = response_json(create_response).await?;

    let Some(product_id) = created_product.get("id").and_then(Value::as_str) else {
        anyhow::bail!("create product response does not contain an ID: {created_product}");
    };

    let product_id = product_id.to_owned();

    let list_response = app
        .oneshot(Request::get("/api/v1/products?limit=10&offset=0").body(Body::empty())?)
        .await?;

    let list_status = list_response.status();
    let list_payload = response_json(list_response).await?;

    let listed_product = list_payload
        .get("data")
        .and_then(Value::as_array)
        .and_then(|products| products.first());

    let expected_pagination = json!({
        "limit": 10,
        "offset": 0,
        "count": 1
    });

    assert_eq!(
        (
            create_status,
            Uuid::parse_str(&product_id).is_ok(),
            created_product.get("owner_id").and_then(Value::as_str),
            created_product.get("name").and_then(Value::as_str),
            created_product.get("description").and_then(Value::as_str),
            created_product.get("price_cents").and_then(Value::as_i64),
            created_product.get("image_url").is_some_and(Value::is_null),
            created_product
                .get("created_at")
                .and_then(Value::as_str)
                .is_some(),
            created_product
                .get("updated_at")
                .and_then(Value::as_str)
                .is_some(),
            list_status,
            list_payload.get("pagination"),
            listed_product
        ),
        (
            StatusCode::CREATED,
            true,
            Some(owner_id.as_str()),
            Some("Mechanical Keyboard"),
            Some("A compact mechanical keyboard"),
            Some(1_250_000),
            true,
            true,
            true,
            StatusCode::OK,
            Some(&expected_pagination),
            Some(&created_product)
        )
    );

    Ok(())
}

#[sqlx::test]
async fn owner_should_get_update_and_delete_product(database: PgPool) -> Result<()> {
    insert_verified_user(&database).await?;

    let app = build_test_app(database).await?;
    let access_token = login_verified_user(app.clone()).await?;

    let created_product = create_test_product(app.clone(), &access_token).await?;

    let Some(product_id) = created_product.get("id").and_then(Value::as_str) else {
        anyhow::bail!("created product doesn't not contain an ID: {created_product}");
    };

    let product_id = product_id.to_owned();
    let product_uri = format!("/api/v1/products/{product_id}");

    let get_response = app
        .clone()
        .oneshot(Request::get(&product_uri).body(Body::empty())?)
        .await?;

    let get_status = get_response.status();
    let get_payload = response_json(get_response).await?;

    let update_response = send_json(
        app.clone(),
        Request::put(&product_uri).header(AUTHORIZATION, format!("Bearer {access_token}")),
        json!({
            "name": "Updated Product",
            "description": "Updated product description",
            "price_cents": 375_000
        }),
    )
    .await?;

    let update_status = update_response.status();
    let update_payload = response_json(update_response).await?;

    let delete_response = app
        .clone()
        .oneshot(
            Request::delete(&product_uri)
                .header(AUTHORIZATION, format!("Bearer {access_token}"))
                .body(Body::empty())?,
        )
        .await?;

    let delete_status = delete_response.status();

    let deleted_get_response = app
        .clone()
        .oneshot(Request::get(&product_uri).body(Body::empty())?)
        .await?;

    let deleted_get_status = deleted_get_response.status();
    let deleted_get_payload = response_json(deleted_get_response).await?;

    assert_eq!(
        (
            get_status,
            &get_payload,
            update_status,
            update_payload.get("id").and_then(Value::as_str),
            update_payload.get("owner_id").and_then(Value::as_str),
            update_payload.get("name").and_then(Value::as_str),
            update_payload.get("description").and_then(Value::as_str),
            update_payload.get("price_cents").and_then(Value::as_i64),
            delete_status,
            deleted_get_status,
            deleted_get_payload.get("code").and_then(Value::as_str),
        ),
        (
            StatusCode::OK,
            &created_product,
            StatusCode::OK,
            Some(product_id.as_str()),
            created_product.get("owner_id").and_then(Value::as_str),
            Some("Updated Product"),
            Some("Updated product description"),
            Some(375_000),
            StatusCode::NO_CONTENT,
            StatusCode::NOT_FOUND,
            Some("PRODUCT_NOT_FOUND"),
        )
    );

    Ok(())
}

#[sqlx::test]
async fn non_owner_should_not_update_or_delete_product(database: PgPool) -> Result<()> {
    insert_verified_user(&database).await?;

    insert_verified_user_with_credentials(
        &database,
        OTHER_USER_EMAIL,
        OTHER_USER_PASSWORD,
        OTHER_USER_DISPLAY_NAME,
    )
    .await?;

    let app = build_test_app(database).await?;

    let owner_token = login_verified_user(app.clone()).await?;
    let other_user_token = login_user(app.clone(), OTHER_USER_EMAIL, OTHER_USER_PASSWORD).await?;

    let created_product = create_test_product(app.clone(), &owner_token).await?;

    let Some(product_id) = created_product.get("id").and_then(Value::as_str) else {
        anyhow::bail!("created product does not contain an ID: {created_product}");
    };

    let product_id = product_id.to_owned();
    let product_uri = format!("/api/v1/products/{product_id}");

    let update_response = send_json(
        app.clone(),
        Request::put(&product_uri).header(AUTHORIZATION, format!("Bearer {other_user_token}")),
        json!({
            "name": "Hijacked Product",
            "description": "This update must be rejected",
            "price_cents": 1
        }),
    )
    .await?;

    let update_status = update_response.status();
    let update_payload = response_json(update_response).await?;

    let delete_response = app
        .clone()
        .oneshot(
            Request::delete(&product_uri)
                .header(AUTHORIZATION, format!("Bearer {other_user_token}"))
                .body(Body::empty())?,
        )
        .await?;

    let delete_status = delete_response.status();
    let delete_payload = response_json(delete_response).await?;

    let get_response = app
        .oneshot(Request::get(&product_uri).body(Body::empty())?)
        .await?;

    let get_status = get_response.status();
    let get_payload = response_json(get_response).await?;

    assert_eq!(
        (
            update_status,
            update_payload.get("code").and_then(Value::as_str),
            delete_status,
            delete_payload.get("code").and_then(Value::as_str),
            get_status,
            get_payload
        ),
        (
            StatusCode::NOT_FOUND,
            Some("PRODUCT_NOT_FOUND"),
            StatusCode::NOT_FOUND,
            Some("PRODUCT_NOT_FOUND"),
            StatusCode::OK,
            created_product
        )
    );

    Ok(())
}

#[sqlx::test]
async fn owner_should_upload_png_to_object_storage(database: PgPool) -> Result<()> {
    let (app, storage, access_token, product_id) = setup_owned_product(database).await?;

    let product_uri = format!("/api/v1/products/{product_id}");
    let object_key = format!("products/{product_id}/image");
    let expected_url_prefix = format!("https://cdn.example.test/{object_key}?v=");

    let upload_response = upload_test_image(
        app.clone(),
        &access_token,
        &product_id,
        &[("image", PNG_BYTES)],
    )
    .await?;

    let upload_status = upload_response.status();
    let upload_payload = response_json(upload_response).await?;

    let stored_object = storage.test_object(&object_key).await;

    let get_response = app
        .oneshot(Request::get(&product_uri).body(Body::empty())?)
        .await?;

    let get_status = get_response.status();
    let get_payload = response_json(get_response).await?;

    let image_url_is_public = upload_payload
        .get("image_url")
        .and_then(Value::as_str)
        .is_some_and(|url| url.starts_with(&expected_url_prefix));

    let stored_content_type = stored_object
        .as_ref()
        .map(|(content_type, _)| content_type.as_str());

    let stored_bytes = stored_object.as_ref().map(|(_, bytes)| bytes.as_ref());

    assert_eq!(
        (
            upload_status,
            image_url_is_public,
            stored_content_type,
            stored_bytes,
            get_status,
            get_payload.get("image_url")
        ),
        (
            StatusCode::OK,
            true,
            Some("image/png"),
            Some(PNG_BYTES),
            StatusCode::OK,
            upload_payload.get("image_url")
        )
    );

    Ok(())
}

#[sqlx::test]
async fn unsupported_image_should_return_validation_error(database: PgPool) -> Result<()> {
    let (app, storage, access_token, product_id) = setup_owned_product(database).await?;

    let object_key = format!("products/{product_id}/image");

    let response = upload_test_image(
        app,
        &access_token,
        &product_id,
        &[("image", b"not an image")],
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;

    let object_was_not_stored = storage.test_object(&object_key).await.is_none();

    assert_eq!(
        (
            status,
            payload.get("code").and_then(Value::as_str),
            first_image_error(&payload),
            object_was_not_stored
        ),
        (
            StatusCode::BAD_REQUEST,
            Some("VALIDATION_ERROR"),
            Some(("image", "Image must be JPEG, PNG, or WEBP")),
            true
        )
    );

    Ok(())
}

#[sqlx::test]
async fn duplicate_image_field_should_return_validation_error(database: PgPool) -> Result<()> {
    let (app, storage, access_token, product_id) = setup_owned_product(database).await?;

    let object_key = format!("products/{product_id}/image");

    let response = upload_test_image(
        app,
        &access_token,
        &product_id,
        &[("image", PNG_BYTES), ("image", PNG_BYTES)],
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;

    let object_was_not_stored = storage.test_object(&object_key).await.is_none();

    assert_eq!(
        (
            status,
            payload.get("code").and_then(Value::as_str),
            first_image_error(&payload),
            object_was_not_stored
        ),
        (
            StatusCode::BAD_REQUEST,
            Some("VALIDATION_ERROR"),
            Some(("image", "Only one image may be uploaded")),
            true
        )
    );

    Ok(())
}

#[sqlx::test]
async fn oversized_image_should_return_validation_error(database: PgPool) -> Result<()> {
    let (app, storage, access_token, product_id) = setup_owned_product(database).await?;

    let object_key = format!("products/{product_id}/image");

    let oversized_image = vec![0; MAX_IMAGE_BYTES + 1];

    let response = upload_test_image(
        app,
        &access_token,
        &product_id,
        &[("image", &oversized_image)],
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;

    let object_was_not_stored = storage.test_object(&object_key).await.is_none();

    assert_eq!(
        (
            status,
            payload.get("code").and_then(Value::as_str),
            first_image_error(&payload),
            object_was_not_stored
        ),
        (
            StatusCode::BAD_REQUEST,
            Some("VALIDATION_ERROR"),
            Some(("image", "Image must not exceed 5 MB")),
            true
        )
    );

    Ok(())
}

#[sqlx::test]
async fn non_owner_should_not_upload_product_image(database: PgPool) -> Result<()> {
    insert_verified_user(&database).await?;

    insert_verified_user_with_credentials(
        &database,
        OTHER_USER_EMAIL,
        OTHER_USER_PASSWORD,
        OTHER_USER_DISPLAY_NAME,
    )
    .await?;

    let (app, storage) = build_test_app_with_storage(database).await?;

    let owner_token = login_verified_user(app.clone()).await?;

    let other_user_token = login_user(app.clone(), OTHER_USER_EMAIL, OTHER_USER_PASSWORD).await?;

    let created_product = create_test_product(app.clone(), &owner_token).await?;

    let Some(product_id) = created_product.get("id").and_then(Value::as_str) else {
        anyhow::bail!("created product does not contain an ID: {created_product}");
    };

    let product_id = product_id.to_owned();

    let object_key = format!("products/{product_id}/image");

    let response =
        upload_test_image(app, &other_user_token, &product_id, &[("image", PNG_BYTES)]).await?;

    let status = response.status();
    let payload = response_json(response).await?;

    let object_was_not_stored = storage.test_object(&object_key).await.is_none();

    assert_eq!(
        (
            status,
            payload.get("code").and_then(Value::as_str),
            object_was_not_stored
        ),
        (StatusCode::NOT_FOUND, Some("PRODUCT_NOT_FOUND"), true)
    );

    Ok(())
}

async fn create_test_product(app: Router, access_token: &str) -> Result<Value> {
    let response = send_json(
        app,
        Request::post("/api/v1/products").header(AUTHORIZATION, format!("Bearer {access_token}")),
        json!({
            "name": "Original Product",
            "description": "Original product description",
            "price_cents": 250_000
        }),
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;

    if status != StatusCode::CREATED {
        anyhow::bail!("test setup failed: product creation returned {status}: {payload}");
    }

    Ok(payload)
}

async fn setup_owned_product(database: PgPool) -> Result<(Router, ObjectStorage, String, String)> {
    insert_verified_user(&database).await?;

    let (app, storage) = build_test_app_with_storage(database).await?;

    let access_token = login_verified_user(app.clone()).await?;

    let created_product = create_test_product(app.clone(), &access_token).await?;

    let Some(product_id) = created_product.get("id").and_then(Value::as_str) else {
        anyhow::bail!("created product does not contain an ID: {created_product}");
    };

    Ok((app, storage, access_token, product_id.to_owned()))
}

async fn upload_test_image(
    app: Router,
    access_token: &str,
    product_id: &str,
    fields: &[(&str, &[u8])],
) -> Result<Response> {
    let image_uri = format!("/api/v1/products/{product_id}/image");

    let response = app
        .oneshot(
            Request::put(image_uri)
                .header(AUTHORIZATION, format!("Bearer {access_token}"))
                .header(
                    CONTENT_TYPE,
                    format!(
                        "multipart/form-data; \
                        boundary={MULTIPART_BOUNDARY}"
                    ),
                )
                .body(Body::from(multipart_body(fields)))?,
        )
        .await?;

    Ok(response)
}

fn multipart_body(fields: &[(&str, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();

    for (name, bytes) in fields {
        let header = format!(
            "--{MULTIPART_BOUNDARY}\r\n\
            Content-Disposition: form-data; \
            name=\"{name}\"; filename=\"test.bin\"\r\n\
            Content-Type: application/octet-stream\r\n\
            \r\n"
        );

        body.extend_from_slice(header.as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}--\r\n").as_bytes());

    body
}

fn first_image_error(payload: &Value) -> Option<(&str, &str)> {
    let detail = payload.get("details")?.as_array()?.first()?;

    Some((
        detail.get("field")?.as_str()?,
        detail.get("message")?.as_str()?,
    ))
}
