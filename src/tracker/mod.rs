use anyhow::Result;
use regex::Regex;
use sqlx::PgPool;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::{update_job_status_by_queue_id, EmailJobStatus};

pub struct StatusTracker {
    pool: PgPool,
    config: Config,
}

impl StatusTracker {
    pub fn new(pool: PgPool, config: Config) -> Self {
        Self { pool, config }
    }

    pub async fn run(&self) {
        info!("Status tracker started");
        
        loop {
            if let Err(e) = self.monitor_postfix_logs().await {
                error!("Error monitoring Postfix logs: {}", e);
                time::sleep(Duration::from_secs(self.config.tracker_poll_interval_secs)).await;
            }
        }
    }

    async fn monitor_postfix_logs(&self) -> Result<()> {
        let mut child = Command::new("journalctl")
            .args([
                "-u", "postfix@-",
                "-f",
                "--output=short-iso",
                "-n", "0",
            ])
            .stdout(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().expect("Failed to capture stdout");
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await? {
            if let Err(e) = self.process_log_line(&line).await {
                warn!("Error processing log line: {}", e);
            }
        }

        Ok(())
    }

    async fn process_log_line(&self, line: &str) -> Result<()> {
        if let Some((queue_id, status)) = parse_postfix_log_line(line) {
            info!("Detected status change: queue_id={}, status={}", queue_id, status);
            update_job_status_by_queue_id(&self.pool, &queue_id, &status).await?;
        }
        Ok(())
    }
}

pub fn parse_postfix_log_line(line: &str) -> Option<(String, String)> {
    let re = Regex::new(
        r"postfix/(?:smtp|local|virtual)/[^:]+: ([A-F0-9]+): .* status=(sent|deferred|bounced)"
    ).ok()?;

    if let Some(captures) = re.captures(line) {
        let queue_id = captures.get(1)?.as_str().to_string();
        let status_str = captures.get(2)?.as_str();

        let status = match status_str {
            "sent" => EmailJobStatus::AcceptedByRecipientServer.to_string(),
            "deferred" => EmailJobStatus::Deferred.to_string(),
            "bounced" => EmailJobStatus::Bounced.to_string(),
            _ => return None,
        };

        return Some((queue_id, status));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_postfix_log_line_sent() {
        let line = "2024-10-23T10:30:45+0000 mail postfix/smtp/smtp[12345]: 3F4A5B6C7D: to=<user@example.com>, relay=mail.example.com[1.2.3.4]:25, delay=0.5, status=sent (250 OK)";
        let result = parse_postfix_log_line(line);
        assert!(result.is_some());
        let (queue_id, status) = result.unwrap();
        assert_eq!(queue_id, "3F4A5B6C7D");
        assert_eq!(status, "AcceptedByRecipientServer");
    }

    #[test]
    fn test_parse_postfix_log_line_deferred() {
        let line = "2024-10-23T10:30:45+0000 mail postfix/smtp/smtp[12345]: ABC123DEF456: to=<user@example.com>, relay=mail.example.com[1.2.3.4]:25, delay=0.5, status=deferred (temporary failure)";
        let result = parse_postfix_log_line(line);
        assert!(result.is_some());
        let (queue_id, status) = result.unwrap();
        assert_eq!(queue_id, "ABC123DEF456");
        assert_eq!(status, "Deferred");
    }

    #[test]
    fn test_parse_postfix_log_line_bounced() {
        let line = "2024-10-23T10:30:45+0000 mail postfix/smtp/smtp[12345]: 9876543210AB: to=<user@example.com>, relay=none, status=bounced (Host not found)";
        let result = parse_postfix_log_line(line);
        assert!(result.is_some());
        let (queue_id, status) = result.unwrap();
        assert_eq!(queue_id, "9876543210AB");
        assert_eq!(status, "Bounced");
    }

    #[test]
    fn test_parse_postfix_log_line_invalid() {
        let line = "Some random log line without postfix status";
        let result = parse_postfix_log_line(line);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_postfix_log_line_local_delivery() {
        let line = "2024-10-23T10:30:45+0000 mail postfix/local/local[12345]: DEADBEEF123: to=<local@localhost>, status=sent (delivered to mailbox)";
        let result = parse_postfix_log_line(line);
        assert!(result.is_some());
        let (queue_id, status) = result.unwrap();
        assert_eq!(queue_id, "DEADBEEF123");
        assert_eq!(status, "AcceptedByRecipientServer");
    }
}
