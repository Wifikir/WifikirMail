use anyhow::{Context, Result};
use lettre::transport::smtp::response::Response;
use lettre::{Message, SmtpTransport, Transport};
use regex::Regex;
use sqlx::PgPool;
use std::time::Duration;
use tokio::time;
use tracing::{error, info};

use crate::config::Config;
use crate::db::{get_pending_jobs, update_job_failed, update_job_submitted};
use crate::templates::render_template;

pub struct EmailWorker {
    pool: PgPool,
    config: Config,
}

impl EmailWorker {
    pub fn new(pool: PgPool, config: Config) -> Self {
        Self { pool, config }
    }

    pub async fn run(&self) {
        info!("Email worker started");
        let mut interval = time::interval(Duration::from_secs(self.config.worker_interval_secs));

        loop {
            interval.tick().await;
            if let Err(e) = self.process_pending_jobs().await {
                error!("Error processing pending jobs: {}", e);
            }
        }
    }

    async fn process_pending_jobs(&self) -> Result<()> {
        let jobs = get_pending_jobs(&self.pool, 10).await?;

        if jobs.is_empty() {
            return Ok(());
        }

        info!("Processing {} pending jobs", jobs.len());

        for job in jobs {
            if let Err(e) = self.send_email(&job).await {
                error!("Failed to send email for job {}: {}", job.id, e);
                update_job_failed(&self.pool, job.id, &e.to_string())
                    .await
                    .context("Failed to update job status")?;
            }
        }

        Ok(())
    }

    async fn send_email(&self, job: &crate::db::EmailJob) -> Result<()> {
        let template = render_template(&job.template_key, &job.reference_id)?;

        let email = Message::builder()
            .from("noreply@wifikir.com".parse()?)
            .to(job.recipient.parse()?)
            .subject(&template.subject)
            .body(template.body)?;

        let mailer = SmtpTransport::builder_dangerous(&self.config.smtp_host)
            .port(self.config.smtp_port)
            .build();

        let response = mailer.send(&email)?;

        let queue_id = extract_queue_id(&response)?;
        info!(
            "Email sent for job {}, Postfix queue ID: {}",
            job.id, queue_id
        );

        update_job_submitted(&self.pool, job.id, &queue_id).await?;

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
        let queue_id = extract_queue_id(&response).unwrap();
        assert_eq!(queue_id, "3F4A5B6C7D");
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
        let queue_id = extract_queue_id(&response).unwrap();
        assert_eq!(queue_id, "ABC123DEF456");
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
        let result = extract_queue_id(&response);
        assert!(result.is_err());
    }
}
