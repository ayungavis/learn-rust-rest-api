use anyhow::Result;
use axum::{Json, Router, routing::get};
use serde::Serialize;
use tokio::net::TcpListener;

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn live() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = Router::new().route("/api/v1/health/live", get(live));
    let listener = TcpListener::bind("0.0.0.0:3000").await?;

    println!("API litening on http://localhost:3000");
    axum::serve(listener, app).await?;

    Ok(())
}
