use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub database: PgPool,
}
