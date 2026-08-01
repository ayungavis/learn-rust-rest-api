use sqlx::PgPool;

use crate::{Mailer, ObjectStorage};

#[derive(Clone)]
pub struct AppState {
    pub database: PgPool,
    pub mailer: Mailer,
    pub storage: ObjectStorage,
}
