use anyhow::Result;
use chrono::{DateTime, Utc};
use lettre::transport::smtp::response::Response;
use lettre::{Message, SmtpTransport, Transport};
use rand::Rng;
use regex::Regex;
use sqlx::PgPool;
use std::time::Duration;
use tokio::time;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::db::{
    claim_pending_jobs, mark_job_submitting, recover_expired_leases, schedule_retry,
    update_job_failed, update_job_submitted,
};
use crate::templates::render_template;

/// Classification of SMTP / handoff failures for the retry policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendErrorKind {
    /// Cannot reach local Postfix (connection refused, timeout, etc.)
    CannotConnect,
    /// SMTP 4xx transient response
    SmtpTransient,
    /// SMTP 5xx permanent response
    SmtpPermanent,
    /// Template / address validation — never retry
    Validation,
}

impl std::fmt::Display for SendErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendErrorKind::CannotConnect => write!(f, "CannotConnect"),
            SendErrorKind::SmtpTransient => write!(f, "SmtpTransient"),
            SendErrorKind::SmtpPermanent => write!(f, "SmtpPermanent"),
            SendErrorKind::Validation => write!(f, "Validation"),
        }
    }
}

pub struct EmailWorker {
    pool: PgPool,
    config: Config,
    worker_id: String,
}

impl EmailWorker {
    pub fn new(pool: PgPool, config: Config) -> Self {
        let worker_id = format!("worker-{}", Uuid::new_v4());
        info!("Email worker created with id: {}", worker_id);
        Self {
            pool,
            config,
            worker_id,
        }
    }

    pub async fn run(&self) {
        info!("Email worker started");

        match recover_expired_leases(&self.pool).await {
            Ok((processing, submitting)) => {
                info!(
                    "Lease recovery: {} Processing→Pending, {} Submitting→Ambiguous",
                    processing, submitting
                );
            }
            Err(e) => error!("Failed to recover expired leases on startup: {}", e),
        }

        let mut interval = time::interval(Duration::from_secs(self.config.worker_interval_secs));

        loop {
            interval.tick().await;

            if let Err(e) = recover_expired_leases(&self.pool).await {
                error!("Failed to recover expired leases: {}", e);
            }

            if let Err(e) = self.process_pending_jobs().await {
                error!("Error processing pending jobs: {}", e);
            }
        }
    }

    async fn process_pending_jobs(&self) -> Result<()> {
        let jobs = claim_pending_jobs(
            &self.pool,
            &self.worker_id,
            10,
            self.config.job_lease_duration_secs,
        )
        .await?;

        if jobs.is_empty() {
            return Ok(());
        }

        info!("Claimed and processing {} jobs", jobs.len());

        for job in jobs {
            match self.send_email(&job).await {
                Ok(()) => {}
                Err((kind, msg)) => {
                    error!("Failed to send email for job {}: {} ({})", job.id, msg, kind);
                    self.handle_send_error(&job, kind, &msg).await?;
                }
            }
        }

        Ok(())
    }

    async fn handle_send_error(
        &self,
        job: &crate::db::EmailJob,
        kind: SendErrorKind,
        msg: &str,
    ) -> Result<()> {
        let attempt_count = job.attempt_count + 1;

        match kind {
            SendErrorKind::Validation | SendErrorKind::SmtpPermanent => {
                update_job_failed(&self.pool, job.id, msg).await?;
            }
            SendErrorKind::CannotConnect | SendErrorKind::SmtpTransient => {
                if attempt_count >= self.config.max_attempts as i32 {
                    update_job_failed(
                        &self.pool,
                        job.id,
                        &format!("{} (max attempts reached)", msg),
                    )
                    .await?;
                } else {
                    let next_attempt = calculate_next_attempt(attempt_count as u32);
                    info!("Scheduling retry for job {} at {}", job.id, next_attempt);
                    schedule_retry(&self.pool, job.id, msg, next_attempt).await?;
                }
            }
        }

        Ok(())
    }

    async fn send_email(
        &self,
        job: &crate::db::EmailJob,
    ) -> Result<(), (SendErrorKind, String)> {
        let template = render_template(&job.template_key, job.template_data.as_ref())
            .map_err(|e| (SendErrorKind::Validation, e.to_string()))?;

        let email = Message::builder()
            .from(
                self.config
                    .mail_from
                    .parse()
                    .map_err(|e| (SendErrorKind::Validation, format!("Invalid MAIL_FROM: {}", e)))?,
            )
            .to(job.recipient.parse().map_err(|e| {
                (
                    SendErrorKind::Validation,
                    format!("Invalid recipient: {}", e),
                )
            })?)
            .subject(&template.subject)
            .body(template.body)
            .map_err(|e| (SendErrorKind::Validation, e.to_string()))?;

        // Persist Submitting marker BEFORE the SMTP handoff so a lease expiry
        // mid-handoff becomes Ambiguous instead of a blind resend.
        mark_job_submitting(&self.pool, job.id)
            .await
            .map_err(|e| (SendErrorKind::Validation, format!("Failed to mark submitting: {}", e)))?;

        let mailer = SmtpTransport::builder_dangerous(&self.config.smtp_host)
            .port(self.config.smtp_port)
            .build();

        let response = mailer.send(&email).map_err(|e| {
            let kind = classify_smtp_error_message(&e.to_string());
            (kind, e.to_string())
        })?;

        let queue_id = extract_queue_id(&response).map_err(|e| {
            (
                SendErrorKind::SmtpPermanent,
                format!("Could not parse queue id: {}", e),
            )
        })?;

        info!(
            "Email accepted by Postfix for job {}, queue ID: {}",
            job.id, queue_id
        );

        // Critical: Postfix accepted. Never treat DB failure as a retryable send.
        if let Err(e) = update_job_submitted(&self.pool, job.id, &queue_id).await {
            error!(
                "CRITICAL: Postfix accepted queue_id={} for job {} but DB update failed: {}. \
                 Job left in Submitting; lease recovery will mark Ambiguous — do not resend.",
                queue_id, job.id, e
            );
            // Last-ditch: try once more to persist the queue id.
            if let Err(e2) = update_job_submitted(&self.pool, job.id, &queue_id).await {
                error!(
                    "CRITICAL: second DB persist also failed for queue_id={}: {}",
                    queue_id, e2
                );
            }
            return Err((
                SendErrorKind::Validation,
                format!(
                    "Postfix accepted (queued as {}) but DB update failed",
                    queue_id
                ),
            ));
        }

        Ok(())
    }
}

pub fn extract_queue_id(response: &Response) -> Result<String> {
    let message = response
        .message()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(" ");

    let re = Regex::new(r"queued as ([A-F0-9]+)")?;
    if let Some(captures) = re.captures(&message) {
        if let Some(queue_id) = captures.get(1) {
            return Ok(queue_id.as_str().to_string());
        }
    }

    anyhow::bail!("Could not extract queue ID from response: {}", message)
}

/// Classify a lettre / SMTP error string into a retry category.
/// Public for unit testing without constructing lettre::Error variants.
pub fn classify_smtp_error_message(msg: &str) -> SendErrorKind {
    let lower = msg.to_lowercase();

    if lower.contains("connection")
        || lower.contains("connect")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("os error")
        || lower.contains("broken pipe")
        || lower.contains("network")
        || lower.contains("refused")
    {
        return SendErrorKind::CannotConnect;
    }

    // Look for SMTP reply codes like "421", "450", "550", or "4.x.x" / "5.x.x" DSN.
    if let Some(code) = extract_smtp_reply_code(msg) {
        if (400..500).contains(&code) {
            return SendErrorKind::SmtpTransient;
        }
        if (500..600).contains(&code) {
            return SendErrorKind::SmtpPermanent;
        }
    }

    if lower.contains("transient") || lower.contains("4.") {
        return SendErrorKind::SmtpTransient;
    }
    if lower.contains("permanent") || lower.contains("5.") {
        return SendErrorKind::SmtpPermanent;
    }

    // Unknown SMTP failure — treat as transient to avoid silent data loss,
    // but still bounded by MAX_ATTEMPTS.
    warn!("Unclassified SMTP error, treating as transient: {}", msg);
    SendErrorKind::SmtpTransient
}

fn extract_smtp_reply_code(msg: &str) -> Option<u16> {
    let re = Regex::new(r"\b([45]\d{2})\b").ok()?;
    re.captures(msg)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// Exponential backoff with jitter: base 60s * 2^(attempt-1), capped at 1h, +0..10% jitter.
pub fn calculate_next_attempt(attempt_count: u32) -> DateTime<Utc> {
    let base_delay_secs = 60u64;
    let max_delay_secs = 3600u64;

    let delay_secs = (base_delay_secs * 2u64.pow(attempt_count.saturating_sub(1))).min(max_delay_secs);

    let jitter = rand::thread_rng().gen_range(0..=(delay_secs / 10).max(1));
    let total_delay = delay_secs + jitter;

    Utc::now() + chrono::Duration::seconds(total_delay as i64)
}

/// Pure backoff delay (without jitter) for deterministic tests.
pub fn backoff_delay_secs(attempt_count: u32) -> u64 {
    let base_delay_secs = 60u64;
    let max_delay_secs = 3600u64;
    (base_delay_secs * 2u64.pow(attempt_count.saturating_sub(1))).min(max_delay_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lettre::transport::smtp::response::{Category, Code, Detail, Response, Severity};

    #[test]
    fn test_extract_queue_id() {
        let response = Response::new(
            Code::new(
                Severity::PositiveCompletion,
                Category::MailSystem,
                Detail::Zero,
            ),
            vec!["2.0.0 Ok: queued as 3F4A5B6C7D".to_string()],
        );
        assert_eq!(extract_queue_id(&response).unwrap(), "3F4A5B6C7D");
    }

    #[test]
    fn test_extract_queue_id_different_format() {
        let response = Response::new(
            Code::new(
                Severity::PositiveCompletion,
                Category::MailSystem,
                Detail::Zero,
            ),
            vec!["Message accepted for delivery queued as ABC123DEF456".to_string()],
        );
        assert_eq!(extract_queue_id(&response).unwrap(), "ABC123DEF456");
    }

    #[test]
    fn test_extract_queue_id_not_found() {
        let response = Response::new(
            Code::new(
                Severity::PositiveCompletion,
                Category::MailSystem,
                Detail::Zero,
            ),
            vec!["Ok".to_string()],
        );
        assert!(extract_queue_id(&response).is_err());
    }

    #[test]
    fn test_classify_cannot_connect() {
        assert_eq!(
            classify_smtp_error_message("Connection refused (os error 111)"),
            SendErrorKind::CannotConnect
        );
        assert_eq!(
            classify_smtp_error_message("failed to connect to 127.0.0.1:25"),
            SendErrorKind::CannotConnect
        );
    }

    #[test]
    fn test_classify_smtp_transient() {
        assert_eq!(
            classify_smtp_error_message("421 4.4.2 Service not available"),
            SendErrorKind::SmtpTransient
        );
        assert_eq!(
            classify_smtp_error_message("450 Temporary failure"),
            SendErrorKind::SmtpTransient
        );
    }

    #[test]
    fn test_classify_smtp_permanent() {
        assert_eq!(
            classify_smtp_error_message("550 5.1.1 User unknown"),
            SendErrorKind::SmtpPermanent
        );
        assert_eq!(
            classify_smtp_error_message("553 permanent rejection"),
            SendErrorKind::SmtpPermanent
        );
    }

    #[test]
    fn test_backoff_delay_exponential() {
        assert_eq!(backoff_delay_secs(1), 60);
        assert_eq!(backoff_delay_secs(2), 120);
        assert_eq!(backoff_delay_secs(3), 240);
        assert_eq!(backoff_delay_secs(4), 480);
    }

    #[test]
    fn test_backoff_delay_capped() {
        assert_eq!(backoff_delay_secs(20), 3600);
    }

    #[test]
    fn test_calculate_next_attempt_in_future() {
        let next = calculate_next_attempt(1);
        assert!(next > Utc::now());
        let delay = (next - Utc::now()).num_seconds();
        assert!((60..=72).contains(&delay));
    }
}
