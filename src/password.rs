use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{Error as PasswordHashError, SaltString, rand_core::OsRng},
};
use thiserror::Error;

use crate::error::FieldError;

#[derive(Debug, Error)]
pub(crate) enum HashPasswordError {
    #[error("password hashing task failed")]
    Task(#[from] tokio::task::JoinError),
    #[error("password hashing failed")]
    Hash(#[from] argon2::password_hash::Error),
}

#[derive(Debug, Error)]
pub(crate) enum VerifyPasswordError {
    #[error("password verification task failed")]
    Task(#[from] tokio::task::JoinError),
    #[error("password verification failed")]
    Verify(#[from] PasswordHashError),
}

pub(crate) fn validation_error(password: &str) -> Option<FieldError> {
    if (15..=128).contains(&password.chars().count()) {
        return None;
    }

    Some(FieldError {
        field: "password",
        message: "Password must contain between 15 and 128 characters",
    })
}

pub(crate) async fn hash(password: String) -> Result<String, HashPasswordError> {
    let password_hash = tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);

        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
    })
    .await??;

    Ok(password_hash)
}

pub(crate) async fn verify(
    password: String,
    encoded_hash: String,
) -> Result<bool, VerifyPasswordError> {
    let verified = tokio::task::spawn_blocking(move || {
        let parsed_hash = PasswordHash::new(&encoded_hash)?;

        match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
            Ok(_) => Ok(true),
            Err(PasswordHashError::Password) => Ok(false),
            Err(error) => Err(error),
        }
    })
    .await??;

    Ok(verified)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{hash, validation_error, verify};

    #[tokio::test]
    async fn password_hash_should_verify_original_password() -> Result<()> {
        let password = "correct horse battery staple";
        let encoded_hash = hash(password.to_owned()).await?;
        let verified = verify(password.to_owned(), encoded_hash).await?;

        assert!(verified);

        Ok(())
    }

    #[tokio::test]
    async fn password_verification_should_reject_wrong_password() -> Result<()> {
        let encoded_hash = hash("correct password value".to_owned()).await?;
        let verified = verify("wrong password value".to_owned(), encoded_hash).await?;

        assert!(!verified);

        Ok(())
    }

    #[test]
    fn password_validation_should_reject_short_password() {
        assert!(validation_error("too-short").is_some());
    }
}
