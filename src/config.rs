use anyhow::Result;
use std::env;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub server_host: String,
    pub server_port: u16,
    pub mail_from: String,
    pub worker_interval_secs: u64,
    pub tracker_poll_interval_secs: u64,
    pub max_attempts: u32,
    pub rate_limit_per_minute: u32,
    pub rate_limit_global: u32,
    pub job_lease_duration_secs: i64,
    pub queue_capacity_limit: usize,
    pub template_dir: String,
    pub metrics_host: String,
    pub metrics_port: u16,
    pub mail_log_path: String,
    pub cursor_path: String,
    /// Domain for VERP envelope sender: bounce+<job_id>@bounce_domain.
    /// Empty disables VERP and uses MAIL_FROM as the envelope sender.
    pub bounce_domain: String,
    /// Maildir (with new/ and cur/) where Postfix delivers DSNs. Empty disables the reader.
    pub bounce_maildir: String,
    pub bounce_poll_secs: u64,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("database_url", &"***REDACTED***")
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("server_host", &self.server_host)
            .field("server_port", &self.server_port)
            .field("mail_from", &self.mail_from)
            .field("worker_interval_secs", &self.worker_interval_secs)
            .field(
                "tracker_poll_interval_secs",
                &self.tracker_poll_interval_secs,
            )
            .field("max_attempts", &self.max_attempts)
            .field("rate_limit_per_minute", &self.rate_limit_per_minute)
            .field("rate_limit_global", &self.rate_limit_global)
            .field("job_lease_duration_secs", &self.job_lease_duration_secs)
            .field("queue_capacity_limit", &self.queue_capacity_limit)
            .field("template_dir", &self.template_dir)
            .field("metrics_host", &self.metrics_host)
            .field("metrics_port", &self.metrics_port)
            .field("mail_log_path", &self.mail_log_path)
            .field("cursor_path", &self.cursor_path)
            .field("bounce_domain", &self.bounce_domain)
            .field("bounce_maildir", &self.bounce_maildir)
            .field("bounce_poll_secs", &self.bounce_poll_secs)
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Config {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://localhost/wifikir_mail".to_string()),
            smtp_host: env::var("SMTP_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            smtp_port: env::var("SMTP_PORT")
                .unwrap_or_else(|_| "25".to_string())
                .parse()?,
            server_host: env::var("SERVER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            server_port: env::var("SERVER_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()?,
            mail_from: env::var("MAIL_FROM").unwrap_or_else(|_| "noreply@wifikir.com".to_string()),
            worker_interval_secs: env::var("WORKER_INTERVAL_SECS")
                .unwrap_or_else(|_| "10".to_string())
                .parse()?,
            tracker_poll_interval_secs: env::var("TRACKER_POLL_INTERVAL_SECS")
                .unwrap_or_else(|_| "30".to_string())
                .parse()?,
            max_attempts: env::var("MAX_ATTEMPTS")
                .unwrap_or_else(|_| "3".to_string())
                .parse()?,
            rate_limit_per_minute: env::var("RATE_LIMIT_PER_MINUTE")
                .unwrap_or_else(|_| "60".to_string())
                .parse()?,
            rate_limit_global: env::var("RATE_LIMIT_GLOBAL")
                .unwrap_or_else(|_| "1000".to_string())
                .parse()?,
            job_lease_duration_secs: env::var("JOB_LEASE_DURATION_SECS")
                .unwrap_or_else(|_| "300".to_string())
                .parse()?,
            queue_capacity_limit: env::var("QUEUE_CAPACITY_LIMIT")
                .unwrap_or_else(|_| "10000".to_string())
                .parse()?,
            template_dir: env::var("TEMPLATE_DIR").unwrap_or_else(|_| "templates".to_string()),
            metrics_host: env::var("METRICS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            metrics_port: env::var("METRICS_PORT")
                .unwrap_or_else(|_| "9090".to_string())
                .parse()?,
            mail_log_path: env::var("MAIL_LOG_PATH")
                .unwrap_or_else(|_| "/var/log/mail.log".to_string()),
            cursor_path: env::var("CURSOR_PATH")
                .unwrap_or_else(|_| "/var/lib/wifikir-mail/mail.log.cursor".to_string()),
            bounce_domain: env::var("BOUNCE_DOMAIN").unwrap_or_default(),
            bounce_maildir: env::var("BOUNCE_MAILDIR").unwrap_or_default(),
            bounce_poll_secs: env::var("BOUNCE_POLL_SECS")
                .unwrap_or_else(|_| "30".to_string())
                .parse()?,
        })
    }
}
