use anyhow::Result;
use regex::Regex;
use sqlx::PgPool;
use std::fs::Metadata;
use std::path::Path;
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::time;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::{update_job_status_by_queue_id, EmailJobStatus};

const MAX_BACKOFF_SECS: u64 = 300;

pub struct StatusTracker {
    pool: PgPool,
    backoff_secs: u64,
    log_path: String,
    cursor_path: String,
}

impl StatusTracker {
    pub fn new(pool: PgPool, config: Config) -> Self {
        Self {
            pool,
            backoff_secs: 1,
            log_path: config.mail_log_path,
            cursor_path: config.cursor_path,
        }
    }

    /// Apply one Postfix log line. Public for integration tests.
    pub async fn apply_line(&self, line: &str) -> Result<bool> {
        self.process_log_line(line).await
    }

    pub async fn run(mut self) {
        crate::metrics::METRICS.tracker_up.set(1);
        info!("Status tracker started, monitoring {}", self.log_path);

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
        let log_path = Path::new(&self.log_path);

        let (last_position, last_inode) = self.read_cursor_with_inode().await.unwrap_or((0, 0));
        info!(
            "Starting from position: {}, inode: {}",
            last_position, last_inode
        );

        let mut file = fs::File::open(log_path).await?;
        let metadata = file.metadata().await?;
        let current_inode = Self::get_inode(&metadata);

        let mut current_position = if current_inode == last_inode && last_position <= metadata.len()
        {
            file.seek(std::io::SeekFrom::Start(last_position)).await?;
            last_position
        } else {
            info!("Log file rotated or truncated, starting from beginning");
            0
        };

        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let line_len = (line.len() + 1) as u64;

                    // Advance cursor ONLY after a successful DB update.
                    // Non-matching lines still advance; matching lines that fail DB do not.
                    match self.process_log_line(&line).await {
                        Ok(matched) => {
                            current_position += line_len;
                            if matched {
                                if let Err(e) = self
                                    .save_cursor_with_inode(current_position, current_inode)
                                    .await
                                {
                                    error!("Failed to save cursor position: {}", e);
                                }
                            } else if current_position % 4096 < line_len {
                                // Periodically persist progress for non-matching lines too.
                                let _ = self
                                    .save_cursor_with_inode(current_position, current_inode)
                                    .await;
                            }
                        }
                        Err(e) => {
                            // Do NOT advance cursor — status would be lost.
                            warn!(
                                "DB update failed for log line at offset {}; cursor not advanced: {}",
                                current_position, e
                            );
                            time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
                Ok(None) => {
                    // Persist cursor at EOF so we don't re-scan on clean restart.
                    let _ = self
                        .save_cursor_with_inode(current_position, current_inode)
                        .await;

                    time::sleep(Duration::from_millis(100)).await;

                    match fs::metadata(log_path).await {
                        Ok(new_metadata) => {
                            let new_inode = Self::get_inode(&new_metadata);
                            if new_inode != current_inode {
                                info!("Log file rotated (inode changed), reopening");
                                return Ok(());
                            }
                            if new_metadata.len() < current_position {
                                info!("Log file truncated, reopening");
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            warn!("Failed to check log file metadata: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Error reading log line: {}", e);
                    return Err(e.into());
                }
            }
        }
    }

    #[cfg(unix)]
    fn get_inode(metadata: &Metadata) -> u64 {
        use std::os::unix::fs::MetadataExt;
        metadata.ino()
    }

    #[cfg(not(unix))]
    fn get_inode(_metadata: &Metadata) -> u64 {
        0
    }

    async fn read_cursor_with_inode(&self) -> Result<(u64, u64)> {
        let content = fs::read_to_string(&self.cursor_path).await?;
        let parts: Vec<&str> = content.trim().split(',').collect();
        if parts.len() == 2 {
            Ok((parts[0].parse()?, parts[1].parse()?))
        } else {
            Ok((content.trim().parse()?, 0))
        }
    }

    async fn save_cursor_with_inode(&self, position: u64, inode: u64) -> Result<()> {
        if let Some(parent) = Path::new(&self.cursor_path).parent() {
            fs::create_dir_all(parent).await.ok();
        }

        let tmp_file = format!("{}.tmp", self.cursor_path);
        let mut file = fs::File::create(&tmp_file).await?;
        let content = format!("{},{}", position, inode);
        file.write_all(content.as_bytes()).await?;
        file.sync_all().await?;
        drop(file);

        fs::rename(&tmp_file, &self.cursor_path).await?;
        Ok(())
    }

    /// Returns Ok(true) if a matching status line was applied to the DB.
    async fn process_log_line(&self, line: &str) -> Result<bool> {
        if let Some((queue_id, status)) = parse_postfix_log_line(line) {
            info!(
                "Detected status change: queue_id={}, status={}",
                queue_id, status
            );
            update_job_status_by_queue_id(&self.pool, &queue_id, &status).await?;
            return Ok(true);
        }
        Ok(false)
    }
}

pub fn parse_postfix_log_line(line: &str) -> Option<(String, String)> {
    let re = Regex::new(
        r"postfix/(?:smtp|local|virtual)\[\d+\]:\s+([A-F0-9]+):\s+.*\s+status=(sent|deferred|bounced)",
    )
    .ok()?;

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
        assert!(parse_postfix_log_line(line).is_none());
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
        assert!(parse_postfix_log_line(line).is_none());
    }
}
