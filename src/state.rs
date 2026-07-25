use sqlx::PgPool;

use crate::Mailer;

#[derive(Clone)]
pub struct AppState {
    pub database: PgPool,
    pub mailer: Mailer,
}
