use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum EmailJobStatus {
    Pending,
    SubmittedToMta,
    Deferred,
    AcceptedByRecipientServer,
    Bounced,
    Failed,
}

impl std::fmt::Display for EmailJobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmailJobStatus::Pending => write!(f, "Pending"),
            EmailJobStatus::SubmittedToMta => write!(f, "SubmittedToMta"),
            EmailJobStatus::Deferred => write!(f, "Deferred"),
            EmailJobStatus::AcceptedByRecipientServer => write!(f, "AcceptedByRecipientServer"),
            EmailJobStatus::Bounced => write!(f, "Bounced"),
            EmailJobStatus::Failed => write!(f, "Failed"),
        }
    }
}

impl std::str::FromStr for EmailJobStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(EmailJobStatus::Pending),
            "SubmittedToMta" => Ok(EmailJobStatus::SubmittedToMta),
            "Deferred" => Ok(EmailJobStatus::Deferred),
            "AcceptedByRecipientServer" => Ok(EmailJobStatus::AcceptedByRecipientServer),
            "Bounced" => Ok(EmailJobStatus::Bounced),
            "Failed" => Ok(EmailJobStatus::Failed),
            _ => Err(format!("Unknown status: {}", s)),
        }
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct EmailJob {
    pub id: Uuid,
    pub recipient: String,
    pub template_key: String,
    pub reference_id: String,
    pub status: String,
    pub postfix_queue_id: Option<String>,
    pub attempt_count: i32,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct EmailJobStatusHistory {
    pub id: i64,
    pub job_id: Uuid,
    pub status: String,
    pub changed_at: DateTime<Utc>,
    pub error_message: Option<String>,
    pub metadata: Option<sqlx::types::JsonValue>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ApiKey {
    pub id: Uuid,
    pub key_hash: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub is_active: bool,
}
