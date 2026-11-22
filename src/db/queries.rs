use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{ApiKey, EmailJob, EmailJobStatus, EmailJobStatusHistory};

/// Atomically insert a job, or return the existing one on idempotency conflict.
/// Same content → Ok(existing). Different content → Err conflict.
pub async fn create_email_job(
    pool: &PgPool,
    api_key_id: Uuid,
    recipient: &str,
    template_key: &str,
    reference_id: &str,
    idempotency_key: &str,
    template_data: &sqlx::types::JsonValue,
) -> Result<EmailJob> {
    let insert_result = sqlx::query_as::<_, EmailJob>(
        r#"
        INSERT INTO email_jobs (
            api_key_id, recipient, template_key, reference_id,
            idempotency_key, template_data, status, attempt_count
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, 0)
        ON CONFLICT (api_key_id, idempotency_key) DO NOTHING
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
    .fetch_optional(pool)
    .await?;

    if let Some(job) = insert_result {
        record_status_change(pool, job.id, &EmailJobStatus::Pending.to_string(), None).await?;
        return Ok(job);
    }

    let existing = sqlx::query_as::<_, EmailJob>(
        r#"
        SELECT * FROM email_jobs
        WHERE api_key_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(api_key_id)
    .bind(idempotency_key)
    .fetch_one(pool)
    .await?;

    if !jobs_content_equal(&existing, recipient, template_key, template_data) {
        anyhow::bail!("Idempotency key conflict: different content");
    }

    Ok(existing)
}

/// Compare job content for idempotency (unit-tested).
pub fn jobs_content_equal(
    existing: &EmailJob,
    recipient: &str,
    template_key: &str,
    template_data: &sqlx::types::JsonValue,
) -> bool {
    let existing_data = existing.template_data.as_ref();
    existing.recipient == recipient
        && existing.template_key == template_key
        && existing_data == Some(template_data)
}

pub async fn get_email_job(pool: &PgPool, id: Uuid) -> Result<Option<EmailJob>> {
    let job = sqlx::query_as::<_, EmailJob>(r#"SELECT * FROM email_jobs WHERE id = $1"#)
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
        r#"SELECT * FROM email_jobs WHERE id = $1 AND api_key_id = $2"#,
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
    let now = Utc::now();

    let jobs = sqlx::query_as::<_, EmailJob>(
        r#"
        UPDATE email_jobs
        SET status = $1, worker_id = $2, locked_until = $3
        WHERE id IN (
            SELECT id FROM email_jobs
            WHERE status = $4
              AND (next_attempt_at IS NULL OR next_attempt_at <= $5)
            ORDER BY created_at ASC
            LIMIT $6
            FOR UPDATE SKIP LOCKED
        )
        RETURNING *
        "#,
    )
    .bind(EmailJobStatus::Processing.to_string())
    .bind(worker_id)
    .bind(locked_until)
    .bind(EmailJobStatus::Pending.to_string())
    .bind(now)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;

    for job in &jobs {
        sqlx::query(r#"INSERT INTO email_job_status_history (job_id, status) VALUES ($1, $2)"#)
            .bind(job.id)
            .bind(EmailJobStatus::Processing.to_string())
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(jobs)
}

/// Recover expired leases.
/// - Processing → Pending (SMTP not started yet; safe to retry)
/// - Submitting → Ambiguous (may have been accepted by Postfix; do NOT auto-resend)
pub async fn recover_expired_leases(pool: &PgPool) -> Result<(usize, usize)> {
    let now = Utc::now();

    let processing = sqlx::query(
        r#"
        UPDATE email_jobs
        SET status = $1, worker_id = NULL, locked_until = NULL
        WHERE status = $2 AND locked_until < $3
        "#,
    )
    .bind(EmailJobStatus::Pending.to_string())
    .bind(EmailJobStatus::Processing.to_string())
    .bind(now)
    .execute(pool)
    .await?;

    let submitting = sqlx::query(
        r#"
        UPDATE email_jobs
        SET status = $1, worker_id = NULL, locked_until = NULL,
            last_error = 'Lease expired during SMTP handoff; do not auto-resend'
        WHERE status = $2 AND locked_until < $3
        "#,
    )
    .bind(EmailJobStatus::Ambiguous.to_string())
    .bind(EmailJobStatus::Submitting.to_string())
    .bind(now)
    .execute(pool)
    .await?;

    Ok((
        processing.rows_affected() as usize,
        submitting.rows_affected() as usize,
    ))
}

pub async fn mark_job_submitting(pool: &PgPool, job_id: Uuid) -> Result<()> {
    sqlx::query(r#"UPDATE email_jobs SET status = $1 WHERE id = $2 AND status = $3"#)
        .bind(EmailJobStatus::Submitting.to_string())
        .bind(job_id)
        .bind(EmailJobStatus::Processing.to_string())
        .execute(pool)
        .await?;

    record_status_change(pool, job_id, &EmailJobStatus::Submitting.to_string(), None).await?;
    Ok(())
}

pub async fn update_job_submitted(
    pool: &PgPool,
    job_id: Uuid,
    postfix_queue_id: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE email_jobs
        SET status = $1, postfix_queue_id = $2, submitted_at = $3,
            attempt_count = attempt_count + 1, worker_id = NULL, locked_until = NULL
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
        SET status = $1, last_error = $2, attempt_count = attempt_count + 1,
            worker_id = NULL, locked_until = NULL, completed_at = $3
        WHERE id = $4
        "#,
    )
    .bind(EmailJobStatus::Failed.to_string())
    .bind(error)
    .bind(Utc::now())
    .bind(job_id)
    .execute(pool)
    .await?;

    record_status_change(
        pool,
        job_id,
        &EmailJobStatus::Failed.to_string(),
        Some(error),
    )
    .await?;
    Ok(())
}

pub async fn schedule_retry(
    pool: &PgPool,
    job_id: Uuid,
    error: &str,
    next_attempt_at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE email_jobs
        SET status = $1, last_error = $2, attempt_count = attempt_count + 1,
            worker_id = NULL, locked_until = NULL, next_attempt_at = $3
        WHERE id = $4
        "#,
    )
    .bind(EmailJobStatus::Pending.to_string())
    .bind(error)
    .bind(next_attempt_at)
    .bind(job_id)
    .execute(pool)
    .await?;

    record_status_change(
        pool,
        job_id,
        &EmailJobStatus::Pending.to_string(),
        Some(error),
    )
    .await?;
    Ok(())
}

pub async fn update_job_status_by_queue_id(
    pool: &PgPool,
    queue_id: &str,
    status: &str,
) -> Result<()> {
    let result = sqlx::query(
        r#"
        UPDATE email_jobs
        SET status = $1,
            completed_at = CASE
                WHEN $1 IN ('AcceptedByRecipientServer', 'Bounced', 'Failed') THEN $2
                ELSE completed_at
            END
        WHERE postfix_queue_id = $3
        "#,
    )
    .bind(status)
    .bind(Utc::now())
    .bind(queue_id)
    .execute(pool)
    .await?;

    if result.rows_affected() > 0 {
        if let Some(job) =
            sqlx::query_as::<_, EmailJob>("SELECT * FROM email_jobs WHERE postfix_queue_id = $1")
                .bind(queue_id)
                .fetch_optional(pool)
                .await?
        {
            record_status_change(pool, job.id, status, None).await?;
        }
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
        WHERE status IN ($1, $2, $3)
        "#,
    )
    .bind(EmailJobStatus::Pending.to_string())
    .bind(EmailJobStatus::Processing.to_string())
    .bind(EmailJobStatus::Submitting.to_string())
    .fetch_one(pool)
    .await?;
    Ok(count.0)
}

pub async fn insert_api_key(pool: &PgPool, name: &str, key_hash: &str) -> Result<ApiKey> {
    let key = sqlx::query_as::<_, ApiKey>(
        r#"
        INSERT INTO api_keys (key_hash, name)
        VALUES ($1, $2)
        RETURNING *
        "#,
    )
    .bind(key_hash)
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(key)
}

pub async fn list_api_keys(pool: &PgPool) -> Result<Vec<ApiKey>> {
    let keys = sqlx::query_as::<_, ApiKey>(r#"SELECT * FROM api_keys ORDER BY created_at DESC"#)
        .fetch_all(pool)
        .await?;
    Ok(keys)
}

pub async fn revoke_api_key(pool: &PgPool, id: Uuid) -> Result<bool> {
    let result = sqlx::query(r#"UPDATE api_keys SET is_active = false WHERE id = $1"#)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn rotate_api_key(pool: &PgPool, id: Uuid, new_hash: &str) -> Result<bool> {
    let result =
        sqlx::query(r#"UPDATE api_keys SET key_hash = $1, is_active = true WHERE id = $2"#)
            .bind(new_hash)
            .bind(id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn pending_queue_stats(pool: &PgPool) -> Result<(i64, i64)> {
    let row: (i64, Option<i64>) = sqlx::query_as(
        r#"
        SELECT COUNT(*)::bigint,
               EXTRACT(EPOCH FROM (NOW() - MIN(created_at)))::bigint
        FROM email_jobs
        WHERE status = $1
        "#,
    )
    .bind(EmailJobStatus::Pending.to_string())
    .fetch_one(pool)
    .await?;
    Ok((row.0, row.1.unwrap_or(0)))
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
        sqlx::query(r#"UPDATE api_keys SET last_used_at = $1 WHERE key_hash = $2"#)
            .bind(Utc::now())
            .bind(key_hash)
            .execute(pool)
            .await?;
    }

    Ok(api_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_job(recipient: &str, template_key: &str, data: serde_json::Value) -> EmailJob {
        EmailJob {
            id: Uuid::new_v4(),
            recipient: recipient.to_string(),
            template_key: template_key.to_string(),
            reference_id: "ref".to_string(),
            status: "Pending".to_string(),
            postfix_queue_id: None,
            attempt_count: 0,
            created_at: Utc::now(),
            submitted_at: None,
            completed_at: None,
            last_error: None,
            worker_id: None,
            locked_until: None,
            api_key_id: Some(Uuid::new_v4()),
            idempotency_key: Some("key-1".to_string()),
            template_data: Some(data),
            next_attempt_at: None,
        }
    }

    #[test]
    fn test_jobs_content_equal_same() {
        let data = json!({ "verification_code": "ABC" });
        let job = sample_job("a@b.com", "EmailVerification", data.clone());
        assert!(jobs_content_equal(
            &job,
            "a@b.com",
            "EmailVerification",
            &data
        ));
    }

    #[test]
    fn test_jobs_content_equal_different_recipient() {
        let data = json!({ "verification_code": "ABC" });
        let job = sample_job("a@b.com", "EmailVerification", data.clone());
        assert!(!jobs_content_equal(
            &job,
            "other@b.com",
            "EmailVerification",
            &data
        ));
    }

    #[test]
    fn test_jobs_content_equal_different_data() {
        let data = json!({ "verification_code": "ABC" });
        let other = json!({ "verification_code": "XYZ" });
        let job = sample_job("a@b.com", "EmailVerification", data);
        assert!(!jobs_content_equal(
            &job,
            "a@b.com",
            "EmailVerification",
            &other
        ));
    }
}
