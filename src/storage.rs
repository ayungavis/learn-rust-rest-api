use std::{collections::HashMap, env, sync::Arc};

use aws_config::{BehaviorVersion, Region, retry::RetryConfig};
use aws_sdk_s3::{
    Client,
    config::Credentials,
    error::SdkError,
    operation::{delete_object::DeleteObjectError, put_object::PutObjectError},
    primitives::ByteStream,
};
use axum::body::Bytes;
use thiserror::Error;
use tokio::sync::RwLock;

const R2_MAX_ATTEMPTS: u32 = 3;

/// Client for storing public product images in Cloudflare R2
#[derive(Clone)]
pub struct ObjectStorage {
    backend: ObjectStorageBackend,
    public_base_url: String,
}

#[derive(Clone)]
enum ObjectStorageBackend {
    R2 {
        client: Client,
        bucket: String,
    },
    InMemory {
        objects: Arc<RwLock<HashMap<String, InMemoryObject>>>,
    },
}

#[derive(Clone)]
struct InMemoryObject {
    content_type: String,
    bytes: Bytes,
}

/// Errors produced while constructing the R2 client
#[derive(Debug, Error)]
pub enum ObjectStorageInitError {
    #[error("required environment variable {name} is missing")]
    MissingEnvironment {
        name: &'static str,
        #[source]
        source: env::VarError,
    },
    #[error("required environment vairable {name} is empty")]
    EmptyEnvironment { name: &'static str },
}

/// Errors produced by R2 object operations
#[derive(Debug, Error)]
pub enum ObjectStorageError {
    #[error("R2 put object request failed")]
    PutObject(#[from] SdkError<PutObjectError>),
    #[error("R2 delete object request failed")]
    DeleteObject(#[from] SdkError<DeleteObjectError>),
}

impl ObjectStorage {
    /// Creates an R2 client from the required 'R2_*' environment variables
    ///
    /// # Errors
    ///
    /// Returns an error when a required environment variable is missing or empty
    pub async fn from_env() -> Result<Self, ObjectStorageInitError> {
        let account_id = required_environment("R2_ACCOUNT_ID")?;
        let access_key_id = required_environment("R2_ACCESS_KEY_ID")?;
        let secret_access_key = required_environment("R2_SECRET_ACCESS_KEY")?;
        let bucket = required_environment("R2_BUCKET")?;
        let public_base_url = required_environment("R2_PUBLIC_BASE_URL")?;

        let endpoint = format!("https://{account_id}.r2.cloudflarestorage.com");

        let credentials = Credentials::new(
            access_key_id,
            secret_access_key,
            None,
            None,
            "cloudflare-r2",
        );

        let shared_config = aws_config::defaults(BehaviorVersion::v2026_01_12())
            .region(Region::new("auto"))
            .endpoint_url(endpoint)
            .credentials_provider(credentials)
            .retry_config(RetryConfig::standard().with_max_attempts(R2_MAX_ATTEMPTS))
            .load()
            .await;

        Ok(Self {
            backend: ObjectStorageBackend::R2 {
                client: Client::new(&shared_config),
                bucket,
            },
            public_base_url: public_base_url.trim_end_matches("/").to_owned(),
        })
    }

    /// Uploads an object and preserves its validated media type
    ///
    /// # Errors
    ///
    /// Returns an error after the configured SDK retries are exhausted
    pub async fn upload(
        &self,
        key: &str,
        content_type: &str,
        bytes: Bytes,
    ) -> Result<(), ObjectStorageError> {
        match &self.backend {
            ObjectStorageBackend::R2 { client, bucket } => {
                client
                    .put_object()
                    .bucket(bucket)
                    .key(key)
                    .content_type(content_type)
                    .cache_control("public, max-age=60")
                    .body(ByteStream::from(bytes))
                    .send()
                    .await
                    .inspect_err(|error| {
                        tracing::warn!(
                            key,
                            error = ?error,
                            max_attemps = R2_MAX_ATTEMPTS,
                                "R2 upload failed after SDK retries"
                        );
                    })?;
            }
            ObjectStorageBackend::InMemory { objects } => {
                objects.write().await.insert(
                    key.to_owned(),
                    InMemoryObject {
                        content_type: content_type.to_owned(),
                        bytes,
                    },
                );
            }
        }

        Ok(())
    }

    /// Deletes an object from R2
    ///
    /// # Errors
    ///
    /// Returns an error after the configured SDK retries are exhausted
    pub async fn delete(&self, key: &str) -> Result<(), ObjectStorageError> {
        match &self.backend {
            ObjectStorageBackend::R2 { client, bucket } => {
                client
                    .delete_object()
                    .bucket(bucket)
                    .key(key)
                    .send()
                    .await
                    .inspect_err(|error| {
                        tracing::warn!(
                            key,
                            error = ?error,
                            max_attempts = R2_MAX_ATTEMPTS,
                            "R2 deletion failed after SDK retries"
                        )
                    })?;
            }
            ObjectStorageBackend::InMemory { objects } => {
                objects.write().await.remove(key);
            }
        }

        Ok(())
    }

    /// Returns a public cache-busted URL for a stored object
    pub fn public_url(&self, key: &str, version: i128) -> String {
        format!("{}/{key}?v={version}", self.public_base_url,)
    }

    /// Creates a test instance that never connects to real S3/R2.
    /// Used by integration tests only.
    pub async fn new_test() -> Self {
        Self {
            backend: ObjectStorageBackend::InMemory {
                objects: Arc::new(RwLock::new(HashMap::new())),
            },
            public_base_url: "https://cdn.example.test".to_owned(),
        }
    }

    #[doc(hidden)]
    pub async fn test_object(&self, key: &str) -> Option<(String, Bytes)> {
        let ObjectStorageBackend::InMemory { objects } = &self.backend else {
            return None;
        };

        objects
            .read()
            .await
            .get(key)
            .map(|object| (object.content_type.clone(), object.bytes.clone()))
    }
}

fn required_environment(name: &'static str) -> Result<String, ObjectStorageInitError> {
    let value = env::var(name)
        .map_err(|source| ObjectStorageInitError::MissingEnvironment { name, source })?;

    if value.is_empty() {
        return Err(ObjectStorageInitError::EmptyEnvironment { name });
    }

    Ok(value)
}
