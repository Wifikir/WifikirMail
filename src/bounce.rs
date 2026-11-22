//! VERP bounce mailbox reader.
//! Postfix delivers DSNs for `bounce+<job_id>@BOUNCE_DOMAIN` into a Maildir.
//! This component reads `new/` and marks the matching job Bounced.
use anyhow::Result;
use regex::Regex;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use tokio::time;
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::db::mark_job_bounced;

pub struct BounceReader {
    pool: PgPool,
    maildir: String,
    poll_secs: u64,
}

impl BounceReader {
    pub fn new(pool: PgPool, config: Config) -> Self {
        Self {
            pool,
            maildir: config.bounce_maildir,
            poll_secs: config.bounce_poll_secs.max(5),
        }
    }

    pub async fn run(self) {
        if self.maildir.is_empty() {
            info!("BOUNCE_MAILDIR is empty; asynchronous bounce reader is off");
            return;
        }
        info!("Bounce reader watching {}", self.maildir);
        let mut interval = time::interval(Duration::from_secs(self.poll_secs));
        loop {
            interval.tick().await;
            if let Err(e) = self.scan_once().await {
                warn!("Bounce mailbox scan failed: {}", e);
            }
        }
    }

    pub async fn scan_once(&self) -> Result<usize> {
        let new_dir = Path::new(&self.maildir).join("new");
        let cur_dir = Path::new(&self.maildir).join("cur");
        fs::create_dir_all(&new_dir).await?;
        fs::create_dir_all(&cur_dir).await?;

        let mut applied = 0;
        let mut entries = fs::read_dir(&new_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let text = fs::read_to_string(&path).await.unwrap_or_default();
            match apply_bounce_text(&self.pool, &text).await {
                Ok(true) => applied += 1,
                Ok(false) => {}
                Err(e) => {
                    warn!("Leaving bounce file in place after DB error: {}", e);
                    continue;
                }
            }
            if let Err(e) = archive_message(&path, &cur_dir).await {
                warn!("Could not archive bounce file: {}", e);
            }
        }
        Ok(applied)
    }
}

pub async fn apply_bounce_text(pool: &PgPool, raw: &str) -> Result<bool> {
    let Some(job_id) = verp_job_id(raw) else {
        return Ok(false);
    };
    let detail = diagnostic_summary(raw);
    let updated = mark_job_bounced(pool, job_id, &detail).await?;
    if updated {
        info!("Asynchronous bounce applied to job {}", job_id);
    }
    Ok(updated)
}

/// Extract the job id from a VERP recipient in the message headers.
pub fn verp_job_id(raw: &str) -> Option<Uuid> {
    let headers = raw.split("\n\n").next().unwrap_or(raw);
    let window = &headers[..headers.len().min(16_384)];
    let re =
        Regex::new(r"(?i)bounce\+([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})")
            .ok()?;
    let id = re.captures(window)?.get(1)?.as_str();
    Uuid::parse_str(id).ok()
}

fn diagnostic_summary(raw: &str) -> String {
    let line = raw
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("diagnostic-code:"))
        .unwrap_or("asynchronous bounce");
    let trimmed = line.trim();
    if trimmed.len() > 500 {
        trimmed[..500].to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn envelope_sender(job_id: Uuid, bounce_domain: &str, mail_from: &str) -> String {
    let domain = bounce_domain.trim();
    if domain.is_empty() {
        mail_from.to_string()
    } else {
        format!("bounce+{job_id}@{domain}")
    }
}

async fn archive_message(path: &Path, cur_dir: &Path) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("bounce");
    let dest: PathBuf = cur_dir.join(format!("{name}:2,S"));
    fs::rename(path, dest).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_job_id_from_delivered_to() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let raw = format!(
            "Delivered-To: bounce+{id}@bounce.wifikir.com\r\nSubject: Undelivered\r\n\r\nbody\r\n"
        );
        assert_eq!(verp_job_id(&raw).unwrap().to_string(), id);
    }

    #[test]
    fn ignores_body_when_headers_have_no_verp() {
        let raw = "To: user@example.com\n\nbounce+550e8400-e29b-41d4-a716-446655440000@x\n";
        assert!(verp_job_id(raw).is_none());
    }

    #[test]
    fn envelope_sender_uses_domain_when_set() {
        let id = Uuid::nil();
        assert_eq!(
            envelope_sender(id, "bounce.wifikir.com", "noreply@wifikir.com"),
            format!("bounce+{id}@bounce.wifikir.com")
        );
        assert_eq!(
            envelope_sender(id, "", "noreply@wifikir.com"),
            "noreply@wifikir.com"
        );
    }
}
