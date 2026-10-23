use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{ApiKey, EmailJob, EmailJobStatus, EmailJobStatusHistory};

pub async fn create_email_job(
    pool: &PgPool,
    api_key_id: Uuid,
    recipient: &str,
    template_key: &str,
    reference_id: &str,
    idempotency_key: Option<&str>,
    template_data: Option<&sqlx::types::JsonValue>,
) -> Result<EmailJob> {
    if let Some(idem_key) = idempotency_key {
        if let Some(existing) = sqlx::query_as::<_, EmailJob>(
            r#"
            SELECT * FROM email_jobs
            WHERE api_key_id = $1 AND idempotency_key = $2
            "#,
        )
        .bind(api_key_id)
        .bind(idem_key)
        .fetch_optional(pool)
        .await?
        {
            let existing_data_str = existing.template_data.as_ref()
                .map(|v| v.to_string())
                .unwrap_or_default();
            let new_data_str = template_data
                .map(|v| v.to_string())
                .unwrap_or_default();
            
            if existing.recipient != recipient 
                || existing.template_key != template_key 
                || existing_data_str != new_data_str
            {
                anyhow::bail!("Idempotency key conflict: different content");
            }
            
            return Ok(existing);
        }
    }

    let job = sqlx::query_as::<_, EmailJob>(
        r#"
        INSERT INTO email_jobs (
            api_key_id, recipient, template_key, reference_id, 
            idempotency_key, template_data, status, attempt_count
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, 0)
        RETURNING *
        "#,
    )
    .bind(api_key_id)
    .bind(recipient)
    .bind(template_key)
    .bind(reference_id)
    .bind(idempotency_key)
    .bind(template_data)
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

pub async fn get_email_job_by_api_key(
    pool: &PgPool,
    api_key_id: Uuid,
    job_id: Uuid,
) -> Result<Option<EmailJob>> {
    let job = sqlx::query_as::<_, EmailJob>(
        r#"
        SELECT * FROM email_jobs WHERE id = $1 AND api_key_id = $2
        "#,
    )
    .bind(job_id)
    .bind(api_key_id)
    .fetch_optional(pool)
    .await?;

    Ok(job)
}

pub async fn claim_pending_jobs(
    pool: &PgPool,
    worker_id: &str,
    limit: i64,
    lease_duration_secs: i64,
) -> Result<Vec<EmailJob>> {
    let mut tx = pool.begin().await?;
    
    let locked_until = Utc::now() + chrono::Duration::seconds(lease_duration_secs);
    
    let jobs = sqlx::query_as::<_, EmailJob>(
        r#"
        UPDATE email_jobs
        SET status = $1, worker_id = $2, locked_until = $3
        WHERE id IN (
            SELECT id FROM email_jobs
            WHERE status = $4
            ORDER BY created_at ASC
            LIMIT $5
            FOR UPDATE SKIP LOCKED
        )
        RETURNING *
        "#,
    )
    .bind(EmailJobStatus::Processing.to_string())
    .bind(worker_id)
    .bind(locked_until)
    .bind(EmailJobStatus::Pending.to_string())
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;

    for job in &jobs {
        sqlx::query(
            r#"
            INSERT INTO email_job_status_history (job_id, status)
            VALUES ($1, $2)
            "#,
        )
        .bind(job.id)
        .bind(EmailJobStatus::Processing.to_string())
        .execute(&mut *tx)
        .await?;
    }
    
    tx.commit().await?;
    Ok(jobs)
}

pub async fn recover_expired_leases(pool: &PgPool) -> Result<usize> {
    let result = sqlx::query(
        r#"
        UPDATE email_jobs
        SET status = $1, worker_id = NULL, locked_until = NULL
        WHERE status = $2 AND locked_until < $3
        "#,
    )
    .bind(EmailJobStatus::Pending.to_string())
    .bind(EmailJobStatus::Processing.to_string())
    .bind(Utc::now())
    .execute(pool)
    .await?;

    Ok(result.rows_affected() as usize)
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

pub async fn count_pending_jobs(pool: &PgPool) -> Result<i64> {
    let count: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM email_jobs
        WHERE status IN ($1, $2)
        "#,
    )
    .bind(EmailJobStatus::Pending.to_string())
    .bind(EmailJobStatus::Processing.to_string())
    .fetch_one(pool)
    .await?;

    Ok(count.0)
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
