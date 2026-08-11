use anyhow::Result;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{
    build_test_app, insert_verified_user, insert_verified_user_with_credentials, login_user,
    login_verified_user, response_json, send_json,
};

mod common;

const OTHER_USER_EMAIL: &str = "other@example.com";
const OTHER_USER_PASSWORD: &str = "another correct horse battery staple";
const OTHER_USER_DISPLAY_NAME: &str = "Other User";

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
