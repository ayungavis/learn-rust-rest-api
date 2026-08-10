use anyhow::Result;
use axum::{
    body::Body,
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{
    build_test_app, insert_verified_user, login_verified_user, response_json, send_json,
};

mod common;

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
