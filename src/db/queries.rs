use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{ApiKey, EmailJob, EmailJobStatus, EmailJobStatusHistory};

pub async fn create_email_job(
    pool: &PgPool,
    recipient: &str,
    template_key: &str,
    reference_id: &str,
) -> Result<EmailJob> {
    let job = sqlx::query_as::<_, EmailJob>(
        r#"
        INSERT INTO email_jobs (recipient, template_key, reference_id, status, attempt_count)
        VALUES ($1, $2, $3, $4, 0)
        ON CONFLICT (reference_id, template_key) DO UPDATE
        SET recipient = EXCLUDED.recipient
        RETURNING *
        "#,
    )
    .bind(recipient)
    .bind(template_key)
    .bind(reference_id)
    .bind(EmailJobStatus::Pending.to_string())
    .fetch_one(pool)
    .await?;

    record_status_change(pool, job.id, &EmailJobStatus::Pending.to_string(), None).await?;

    Ok(job)
}

pub async fn get_email_job(pool: &PgPool, id: Uuid) -> Result<Option<EmailJob>> {
    let job = sqlx::query_as::<_, EmailJob>(
        r#"
        SELECT * FROM email_jobs WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(job)
}

pub async fn get_pending_jobs(pool: &PgPool, limit: i64) -> Result<Vec<EmailJob>> {
    let jobs = sqlx::query_as::<_, EmailJob>(
        r#"
        SELECT * FROM email_jobs
        WHERE status = $1
        ORDER BY created_at ASC
        LIMIT $2
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(EmailJobStatus::Pending.to_string())
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(jobs)
}

pub async fn update_job_submitted(
    pool: &PgPool,
    job_id: Uuid,
    postfix_queue_id: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE email_jobs
        SET status = $1, postfix_queue_id = $2, submitted_at = $3, attempt_count = attempt_count + 1
        WHERE id = $4
        "#,
    )
    .bind(EmailJobStatus::SubmittedToMta.to_string())
    .bind(postfix_queue_id)
    .bind(Utc::now())
    .bind(job_id)
    .execute(pool)
    .await?;

    record_status_change(
        pool,
        job_id,
        &EmailJobStatus::SubmittedToMta.to_string(),
        None,
    )
    .await?;

    Ok(())
}

pub async fn update_job_failed(pool: &PgPool, job_id: Uuid, error: &str) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE email_jobs
        SET status = $1, last_error = $2, attempt_count = attempt_count + 1
        WHERE id = $3
        "#,
    )
    .bind(EmailJobStatus::Failed.to_string())
    .bind(error)
    .bind(job_id)
    .execute(pool)
    .await?;

    record_status_change(pool, job_id, &EmailJobStatus::Failed.to_string(), Some(error)).await?;

    Ok(())
}

pub async fn update_job_status_by_queue_id(
    pool: &PgPool,
    queue_id: &str,
    status: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE email_jobs
        SET status = $1, completed_at = CASE WHEN $1 IN ('AcceptedByRecipientServer', 'Bounced', 'Failed') THEN $2 ELSE completed_at END
        WHERE postfix_queue_id = $3
        RETURNING id
        "#,
    )
    .bind(status)
    .bind(Utc::now())
    .bind(queue_id)
    .execute(pool)
    .await?;

    if let Some(job) = sqlx::query_as::<_, EmailJob>(
        "SELECT * FROM email_jobs WHERE postfix_queue_id = $1",
    )
    .bind(queue_id)
    .fetch_optional(pool)
    .await?
    {
        record_status_change(pool, job.id, status, None).await?;
    }

    Ok(())
}

async fn record_status_change(
    pool: &PgPool,
    job_id: Uuid,
    status: &str,
    error_message: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO email_job_status_history (job_id, status, error_message)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(job_id)
    .bind(status)
    .bind(error_message)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_status_history(pool: &PgPool, job_id: Uuid) -> Result<Vec<EmailJobStatusHistory>> {
    let history = sqlx::query_as::<_, EmailJobStatusHistory>(
        r#"
        SELECT * FROM email_job_status_history
        WHERE job_id = $1
        ORDER BY changed_at DESC
        "#,
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?;

    Ok(history)
}

pub async fn verify_api_key(pool: &PgPool, key_hash: &str) -> Result<Option<ApiKey>> {
    let api_key = sqlx::query_as::<_, ApiKey>(
        r#"
        SELECT * FROM api_keys
        WHERE key_hash = $1 AND is_active = true
        "#,
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await?;

    if api_key.is_some() {
        sqlx::query(
            r#"
            UPDATE api_keys
            SET last_used_at = $1
            WHERE key_hash = $2
            "#,
        )
        .bind(Utc::now())
        .bind(key_hash)
        .execute(pool)
        .await?;
    }

    Ok(api_key)
}
