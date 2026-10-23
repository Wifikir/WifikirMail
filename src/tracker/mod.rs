use anyhow::Result;
use regex::Regex;
use sqlx::PgPool;
use std::path::Path;
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::{update_job_status_by_queue_id, EmailJobStatus};

const CURSOR_FILE: &str = "/var/lib/wifikir-mail/mail.log.cursor";
const MAX_BACKOFF_SECS: u64 = 300;

pub struct StatusTracker {
    pool: PgPool,
    backoff_secs: u64,
}

impl StatusTracker {
    pub fn new(pool: PgPool, _config: Config) -> Self {
        Self {
            pool,
            backoff_secs: 1,
        }
    }

    pub async fn run(mut self) {
        info!("Status tracker started, monitoring /var/log/mail.log");

        loop {
            match self.monitor_postfix_logs().await {
                Ok(_) => {
                    warn!("Log monitoring ended normally, restarting");
                    self.backoff_secs = 1;
                }
                Err(e) => {
                    error!(
                        "Error monitoring Postfix logs: {}, retrying in {} seconds",
                        e, self.backoff_secs
                    );
                    time::sleep(Duration::from_secs(self.backoff_secs)).await;
                    self.backoff_secs = (self.backoff_secs * 2).min(MAX_BACKOFF_SECS);
                }
            }
        }
    }

    async fn monitor_postfix_logs(&mut self) -> Result<()> {
        let log_path = Path::new("/var/log/mail.log");
        
        let last_position = self.read_cursor().await.unwrap_or(0);
        info!("Starting from position: {}", last_position);

        let file = fs::File::open(log_path).await?;
        let mut file = file;
        
        if last_position > 0 {
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(last_position)).await?;
        }

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut current_position = last_position;

        loop {
            match lines.next_line().await? {
                Some(line) => {
                    current_position += line.len() as u64 + 1;
                    
                    if let Err(e) = self.process_log_line(&line).await {
                        warn!("Error processing log line: {}", e);
                    }
                    
                    if let Err(e) = self.save_cursor(current_position).await {
                        warn!("Failed to save cursor position: {}", e);
                    }
                }
                None => {
                    time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    async fn read_cursor(&self) -> Result<u64> {
        let content = fs::read_to_string(CURSOR_FILE).await?;
        Ok(content.trim().parse()?)
    }

    async fn save_cursor(&self, position: u64) -> Result<()> {
        if let Some(parent) = Path::new(CURSOR_FILE).parent() {
            fs::create_dir_all(parent).await.ok();
        }
        
        let tmp_file = format!("{}.tmp", CURSOR_FILE);
        let mut file = fs::File::create(&tmp_file).await?;
        file.write_all(position.to_string().as_bytes()).await?;
        file.sync_all().await?;
        drop(file);
        
        fs::rename(&tmp_file, CURSOR_FILE).await?;
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
        r"postfix/(?:smtp|local|virtual)\[\d+\]:\s+([A-F0-9]+):\s+.*\s+status=(sent|deferred|bounced)"
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
        let line = "Oct 23 10:30:45 mail postfix/smtp[12345]: 3F4A5B6C7D: to=<user@example.com>, relay=mail.example.com[1.2.3.4]:25, delay=0.5, delays=0.01/0.02/0.03/0.44, dsn=2.0.0, status=sent (250 2.0.0 OK)";
        let result = parse_postfix_log_line(line);
        assert!(result.is_some());
        let (queue_id, status) = result.unwrap();
        assert_eq!(queue_id, "3F4A5B6C7D");
        assert_eq!(status, "AcceptedByRecipientServer");
    }

    #[test]
    fn test_parse_postfix_log_line_deferred() {
        let line = "Oct 23 10:30:45 mail postfix/smtp[12345]: ABC123DEF456: to=<user@example.com>, relay=mail.example.com[1.2.3.4]:25, delay=0.5, delays=0.01/0.02/0.03/0.44, dsn=4.4.1, status=deferred (connect to mail.example.com[1.2.3.4]:25: Connection refused)";
        let result = parse_postfix_log_line(line);
        assert!(result.is_some());
        let (queue_id, status) = result.unwrap();
        assert_eq!(queue_id, "ABC123DEF456");
        assert_eq!(status, "Deferred");
    }

    #[test]
    fn test_parse_postfix_log_line_bounced() {
        let line = "Oct 23 10:30:45 mail postfix/smtp[12345]: 9876543210AB: to=<user@example.com>, relay=mail.example.com[1.2.3.4]:25, delay=0.5, delays=0.01/0.02/0.03/0.44, dsn=5.1.1, status=bounced (host mail.example.com[1.2.3.4] said: 550 5.1.1 User unknown)";
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
        let line = "Oct 23 10:30:45 mail postfix/local[54321]: DEADBEEF123: to=<local@localhost>, relay=local, delay=0.02, delays=0.01/0/0/0.01, dsn=2.0.0, status=sent (delivered to mailbox)";
        let result = parse_postfix_log_line(line);
        assert!(result.is_some());
        let (queue_id, status) = result.unwrap();
        assert_eq!(queue_id, "DEADBEEF123");
        assert_eq!(status, "AcceptedByRecipientServer");
    }

    #[test]
    fn test_parse_postfix_log_line_virtual_delivery() {
        let line = "Oct 23 10:30:45 mail postfix/virtual[98765]: ABCDEF12345: to=<user@virtual.example.com>, relay=virtual, delay=0.03, delays=0.01/0/0/0.02, dsn=2.0.0, status=sent (delivered to maildir)";
        let result = parse_postfix_log_line(line);
        assert!(result.is_some());
        let (queue_id, status) = result.unwrap();
        assert_eq!(queue_id, "ABCDEF12345");
        assert_eq!(status, "AcceptedByRecipientServer");
    }

    #[test]
    fn test_parse_postfix_log_line_without_status() {
        let line = "Oct 23 10:30:45 mail postfix/smtp[12345]: 3F4A5B6C7D: to=<user@example.com>, relay=mail.example.com[1.2.3.4]:25, delay=0.5";
        let result = parse_postfix_log_line(line);
        assert!(result.is_none());
    }
}
