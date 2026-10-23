use anyhow::Result;
use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub server_host: String,
    pub server_port: u16,
    pub worker_interval_secs: u64,
    pub tracker_poll_interval_secs: u64,
    pub max_attempts: u32,
    pub rate_limit_per_minute: u32,
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
        })
    }
}
