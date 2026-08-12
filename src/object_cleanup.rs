use std::time::Duration;

use sqlx::{PgPool, prelude::FromRow};
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{ObjectStorage, storage::ObjectStorageError};

const CLEANUP_INTERVAL: Duration = Duration::from_secs(5);
const JOB_LEASE_SECONDS: i64 = 120;
const MAX_JOBS_PER_TICK: usize = 20;
const MAX_DELETE_ATTEMPTS: i32 = 10;
const MAX_ERROR_MESSAGE_CHARS: usize = 2_000;

#[derive(FromRow)]
struct ObjectDeletionJob {
    id: Uuid,
    object_key: String,
    attempts: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JobFailureDisposition {
    RetryAfterSeconds(i64),
    PermantentlyFailed,
}

/// Deletes queued R2 objects until cancellation is requested
pub async fn run_object_cleanup_worker(
    database: PgPool,
    storage: ObjectStorage,
    cancellation: CancellationToken,
) {
    let mut ticker = interval(CLEANUP_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;

            () = cancellation.cancelled() => {
                tracing::info!("object cleanup worker stopped");
                return;
            }

            _ = ticker.tick() => {
                if let Err(error) = process_available_jobs(&database, &storage).await {
                    tracing::error!(error = ?error, "object cleanup batch failed");
                }
            }
        }
    }
}

async fn process_available_jobs(
    database: &PgPool,
    storage: &ObjectStorage,
) -> Result<(), sqlx::Error> {
    for _ in 0..MAX_JOBS_PER_TICK {
        let Some(job) = claim_next_job(database).await? else {
            break;
        };

        process_job(database, storage, job).await?;
    }

    Ok(())
}

async fn claim_next_job(database: &PgPool) -> Result<Option<ObjectDeletionJob>, sqlx::Error> {
    sqlx::query_as::<_, ObjectDeletionJob>(
        r#"
        WITH next_job AS (
            SELECT id
            FROM object_deletion_jobs
            WHERE failed_at IS NULL
                AND available_at <= now()
            ORDER BY available_at, id
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        )
        UPDATE object_deletion_jobs AS job
        SET attempts = job.attempts + 1,
            available_at =
                now() + ($1::bigint * interval '1 seconds')
        FROM next_job
        WHERE job.id = next_job.id
        RETURNING
            job.id,
            job.object_key,
            job.attempts
        "#,
    )
    .bind(JOB_LEASE_SECONDS)
    .fetch_optional(database)
    .await
}

async fn process_job(
    database: &PgPool,
    storage: &ObjectStorage,
    job: ObjectDeletionJob,
) -> Result<(), sqlx::Error> {
    match storage.delete(&job.object_key).await {
        Ok(()) => {
            complete_job(database, job.id).await?;

            tracing::info!(
                job_id = %job.id,
                object_key = job.object_key,
                "R2 object deleted"
            );
        }

        Err(error) => {
            let disposition = record_job_failure(database, &job, &error).await?;

            match disposition {
                JobFailureDisposition::RetryAfterSeconds(delay_seconds) => {
                    tracing::warn!(
                        job_id = %job.id,
                        object_key = job.object_key,
                        attempts = job.attempts,
                        delay_seconds,
                        error = ?error,
                        "R2 object deletion will be retried"
                    );
                }
                JobFailureDisposition::PermantentlyFailed => {
                    tracing::error!(
                        job_id = %job.id,
                        object_key = job.object_key,
                        attempts = job.attempts,
                        error = ?error,
                        "R2 object deletion permanently failed"
                    );
                }
            }
        }
    }

    Ok(())
}

async fn complete_job(database: &PgPool, job_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM object_deletion_jobs WHERE id = $1")
        .bind(job_id)
        .execute(database)
        .await?;

    Ok(())
}

async fn record_job_failure(
    database: &PgPool,
    job: &ObjectDeletionJob,
    error: &ObjectStorageError,
) -> Result<JobFailureDisposition, sqlx::Error> {
    let error_message: String = format!("{error:?}")
        .chars()
        .take(MAX_ERROR_MESSAGE_CHARS)
        .collect();

    let disposition = job_failure_disposition(job.attempts);

    match disposition {
        JobFailureDisposition::RetryAfterSeconds(delay_seconds) => {
            reschedule_job(database, job.id, delay_seconds, &error_message).await?;
        }
        JobFailureDisposition::PermantentlyFailed => {
            mark_job_failed(database, job.id, &error_message).await?;
        }
    }

    Ok(disposition)
}

async fn mark_job_failed(
    database: &PgPool,
    job_id: Uuid,
    error_message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE object_deletion_jobs
        SET failed_at = now(),
            last_error = $2
        WHERE id = $1
        "#,
    )
    .bind(job_id)
    .bind(error_message)
    .execute(database)
    .await?;

    Ok(())
}

async fn reschedule_job(
    database: &PgPool,
    job_id: Uuid,
    delay_seconds: i64,
    error_message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE object_deletion_jobs
        SET available_at =
                now() + ($2::bigint * interval '1 second'),
            last_error = $3
        WHERE id = $1
        "#,
    )
    .bind(job_id)
    .bind(delay_seconds)
    .bind(error_message)
    .execute(database)
    .await?;

    Ok(())
}

fn retry_delay_seconds(attempts: i32) -> i64 {
    let exponent = attempts.clamp(1, 6) - 1;

    5 * 2_i64.pow(exponent.cast_unsigned())
}

fn job_failure_disposition(attempts: i32) -> JobFailureDisposition {
    if attempts >= MAX_DELETE_ATTEMPTS {
        return JobFailureDisposition::PermantentlyFailed;
    }

    JobFailureDisposition::RetryAfterSeconds(retry_delay_seconds(attempts))
}

#[cfg(test)]
mod tests {
    use super::{
        JobFailureDisposition, MAX_DELETE_ATTEMPTS, job_failure_disposition, retry_delay_seconds,
    };

    #[test]
    fn retry_delay_should_start_at_five_seconds() {
        assert_eq!(retry_delay_seconds(1), 5);
    }

    #[test]
    fn retry_delay_should_cap_at_one_hundred_sixty_seconds() {
        assert_eq!(retry_delay_seconds(10), 160);
    }

    #[test]
    fn job_failure_should_schedule_retry_before_maximum_attempts() {
        assert_eq!(
            job_failure_disposition(MAX_DELETE_ATTEMPTS - 1),
            JobFailureDisposition::RetryAfterSeconds(160)
        );
    }

    #[test]
    fn job_failure_should_stop_retrying_at_maximum_attempts() {
        assert_eq!(
            job_failure_disposition(MAX_DELETE_ATTEMPTS),
            JobFailureDisposition::PermantentlyFailed
        );
    }
}
