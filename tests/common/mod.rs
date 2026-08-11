use anyhow::Result;
use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::CONTENT_TYPE, request::Builder},
    response::Response,
};
use rust_catalog_api::{AppState, Mailer, ObjectStorage, build_router};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

pub const VERIFIED_USER_EMAIL: &str = "learner@example.com";
pub const VERIFIED_USER_PASSWORD: &str = "correct horse battery staple";
pub const VERIFIED_USER_DISPLAY_NAME: &str = "Rust Learner";

pub async fn build_test_app(database: PgPool) -> Result<Router> {
    let (app, _, _) = build_test_app_with_dependencies(database).await?;
    Ok(app)
}

#[allow(dead_code)]
pub async fn build_test_app_with_storage(database: PgPool) -> Result<(Router, ObjectStorage)> {
    let (app, _, storage) = build_test_app_with_dependencies(database).await?;
    Ok((app, storage))
}

#[allow(dead_code)]
pub async fn build_test_app_with_mailer(database: PgPool) -> Result<(Router, Mailer)> {
    let (app, mailer, _) = build_test_app_with_dependencies(database).await?;
    Ok((app, mailer))
}

async fn build_test_app_with_dependencies(
    database: PgPool,
) -> Result<(Router, Mailer, ObjectStorage)> {
    let mailer = Mailer::new_test(
        "Rust Catalog <noreply@example.com>",
        "http://localhost:5173".to_owned(),
    )?;

    let storage = ObjectStorage::new_test().await;

    let app = build_router(AppState {
        database,
        mailer: mailer.clone(),
        storage: storage.clone(),
    });

    Ok((app, mailer, storage))
}

pub async fn login_verified_user(app: Router) -> Result<String> {
    login_user(app, VERIFIED_USER_EMAIL, VERIFIED_USER_PASSWORD).await
}

pub async fn login_user(app: Router, email: &str, password: &str) -> Result<String> {
    let response = send_json(
        app,
        Request::post("/api/v1/auth/login"),
        json!({
            "email": email,
            "password": password
        }),
    )
    .await?;

    let status = response.status();
    let payload = response_json(response).await?;

    if status != StatusCode::OK {
        anyhow::bail!("test setup failed: login returned {status}: {payload}");
    }

    let Some(access_token) = payload.get("access_token").and_then(Value::as_str) else {
        anyhow::bail!("login response does not contain access token: {payload}");
    };

    Ok(access_token.to_owned())
}

pub async fn insert_verified_user(database: &PgPool) -> Result<Uuid> {
    insert_verified_user_with_credentials(
        database,
        VERIFIED_USER_EMAIL,
        VERIFIED_USER_PASSWORD,
        VERIFIED_USER_DISPLAY_NAME,
    )
    .await
}

pub async fn insert_verified_user_with_credentials(
    database: &PgPool,
    email: &str,
    password: &str,
    display_name: &str,
) -> Result<Uuid> {
    let user_id = Uuid::now_v7();
    let password = password.to_owned();

    let password_hash = tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);

        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
    })
    .await??;

    sqlx::query(
        r#"
        INSERT INTO users (
            id,
            email,
            password_hash,
            display_name,
            email_verified_at
        )
        VALUES (
            $1,
            $2,
            $3,
            $4,
            now()
        )
        "#,
    )
    .bind(user_id)
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .execute(database)
    .await?;

    Ok(user_id)
}

pub async fn send_json(app: Router, request_builder: Builder, payload: Value) -> Result<Response> {
    let body = serde_json::to_vec(&payload)?;

    let request = request_builder
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))?;

    let response = app.oneshot(request).await?;

    Ok(response)
}

pub async fn response_json(response: Response) -> Result<Value> {
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    let payload = serde_json::from_slice(&body)?;

    Ok(payload)
}
