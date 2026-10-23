use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{validate_email, validate_template_key};
use crate::db::{create_email_job, get_email_job};

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateEmailRequest {
    pub recipient: String,
    pub template_key: String,
    pub reference_id: String,
}

#[derive(Debug, Serialize)]
pub struct CreateEmailResponse {
    pub job_id: Uuid,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct EmailJobResponse {
    pub id: Uuid,
    pub recipient: String,
    pub template_key: String,
    pub reference_id: String,
    pub status: String,
    pub postfix_queue_id: Option<String>,
    pub created_at: String,
    pub submitted_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

pub async fn create_email(
    State(state): State<AppState>,
    Json(payload): Json<CreateEmailRequest>,
) -> Result<(StatusCode, Json<CreateEmailResponse>), (StatusCode, Json<ErrorResponse>)> {
    if !validate_email(&payload.recipient) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid email address".to_string(),
            }),
        ));
    }

    if !validate_template_key(&payload.template_key) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid template key".to_string(),
            }),
        ));
    }

    if payload.reference_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Reference ID cannot be empty".to_string(),
            }),
        ));
    }

    match create_email_job(
        &state.pool,
        &payload.recipient,
        &payload.template_key,
        &payload.reference_id,
    )
    .await
    {
        Ok(job) => Ok((
            StatusCode::ACCEPTED,
            Json(CreateEmailResponse {
                job_id: job.id,
                status: job.status,
            }),
        )),
        Err(e) => {
            tracing::error!("Failed to create email job: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to create email job".to_string(),
                }),
            ))
        }
    }
}

pub async fn get_email_status(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<EmailJobResponse>, (StatusCode, Json<ErrorResponse>)> {
    match get_email_job(&state.pool, job_id).await {
        Ok(Some(job)) => Ok(Json(EmailJobResponse {
            id: job.id,
            recipient: job.recipient,
            template_key: job.template_key,
            reference_id: job.reference_id,
            status: job.status,
            postfix_queue_id: job.postfix_queue_id,
            created_at: job.created_at.to_rfc3339(),
            submitted_at: job.submitted_at.map(|t| t.to_rfc3339()),
            completed_at: job.completed_at.map(|t| t.to_rfc3339()),
        })),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Job not found".to_string(),
            }),
        )),
        Err(e) => {
            tracing::error!("Failed to get email job: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to retrieve email job".to_string(),
                }),
            ))
        }
    }
}
