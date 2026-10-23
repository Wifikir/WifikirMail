use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::auth::hash_api_key;
use crate::db::{count_pending_jobs, verify_api_key};

use super::{AppState, ErrorResponse};

pub async fn auth_middleware(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Missing Authorization header".to_string(),
                }),
            )
        })?;

    if !auth_header.starts_with("Bearer ") {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid Authorization header format".to_string(),
            }),
        ));
    }

    let api_key = &auth_header[7..];
    let key_hash = hash_api_key(api_key);

    match verify_api_key(&state.pool, &key_hash).await {
        Ok(Some(api_key_record)) => {
            request.extensions_mut().insert(api_key_record.id);
            Ok(next.run(request).await)
        }
        Ok(None) => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid API key".to_string(),
            }),
        )),
        Err(e) => {
            tracing::error!("Failed to verify API key: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Authentication error".to_string(),
                }),
            ))
        }
    }
}

#[derive(Clone)]
pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window,
        }
    }

    pub async fn check(&self, key: &str) -> bool {
        let mut requests = self.requests.lock().await;
        let now = Instant::now();
        
        let entry = requests.entry(key.to_string()).or_insert_with(Vec::new);
        
        entry.retain(|&t| now.duration_since(t) < self.window);
        
        if entry.len() >= self.max_requests {
            return false;
        }
        
        entry.push(now);
        true
    }
}

pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let api_key_id = request
        .extensions()
        .get::<Uuid>()
        .copied()
        .unwrap_or_else(Uuid::nil);

    if !state.rate_limiter.check(&api_key_id.to_string()).await {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse {
                error: "Per-key rate limit exceeded".to_string(),
            }),
        ));
    }

    if !state.global_rate_limiter.check("global").await {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse {
                error: "Global rate limit exceeded".to_string(),
            }),
        ));
    }

    match count_pending_jobs(&state.pool).await {
        Ok(count) if count >= state.queue_capacity_limit as i64 => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Queue capacity exceeded, please try again later".to_string(),
                }),
            ));
        }
        Err(e) => {
            tracing::warn!("Failed to check queue capacity: {}", e);
        }
        _ => {}
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        
        assert!(limiter.check("user1").await);
        assert!(limiter.check("user1").await);
        assert!(limiter.check("user1").await);
        assert!(!limiter.check("user1").await);
        
        assert!(limiter.check("user2").await);
    }

    #[tokio::test]
    async fn test_rate_limiter_expiry() {
        let limiter = RateLimiter::new(2, Duration::from_millis(100));
        
        assert!(limiter.check("user1").await);
        assert!(limiter.check("user1").await);
        assert!(!limiter.check("user1").await);
        
        tokio::time::sleep(Duration::from_millis(150)).await;
        
        assert!(limiter.check("user1").await);
    }
}
