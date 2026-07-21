use std::{env, net::SocketAddr};

use anyhow::{Context, Result};

pub struct Config {
    pub address: SocketAddr,
    pub database_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        load_dotenv()?;

        let address = env::var("APP_ADDRESS")
            .context("APP_ADDRESS environment variable is required")?
            .parse::<SocketAddr>()
            .context("APP_ADDRESS mut use the format IP:PORT")?;

        let database_url =
            env::var("DATABASE_URL").context("DATABASE_URL environment variable is required")?;

        Ok(Self {
            address,
            database_url,
        })
    }
}

fn load_dotenv() -> Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(error) if error.not_found() => Ok(()),
        Err(error) => Err(error).context("failed to load .env file"),
    }
}
