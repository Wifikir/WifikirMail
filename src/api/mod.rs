pub mod handlers;
pub mod middleware;

use axum::{
    middleware as axum_middleware,
    routing::{get, post},
    Router,
};
use sqlx::PgPool;
use std::time::Duration;
use tower_http::trace::TraceLayer;

use crate::config::Config;

pub use handlers::*;
pub use middleware::*;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub rate_limiter: RateLimiter,
    pub global_rate_limiter: RateLimiter,
    pub queue_capacity_limit: usize,
}

pub fn create_router(pool: PgPool, config: &Config) -> Router {
    let rate_limiter = RateLimiter::new(
        config.rate_limit_per_minute as usize,
        Duration::from_secs(60),
    );
    
    let global_rate_limiter = RateLimiter::new(
        config.rate_limit_global as usize,
        Duration::from_secs(60),
    );

    let state = AppState {
        pool,
        rate_limiter,
        global_rate_limiter,
        queue_capacity_limit: config.queue_capacity_limit,
    };

    let protected_routes = Router::new()
        .route("/v1/emails", post(handlers::create_email))
        .route("/v1/emails/:id", get(handlers::get_email_status))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::rate_limit_middleware,
        ));

    Router::new()
        .route("/health", get(handlers::health))
        .route("/ready", get(handlers::ready))
        .merge(protected_routes)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
