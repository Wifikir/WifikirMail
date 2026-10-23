use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use tokio::signal;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use wifikir_mail::{
    api::create_router,
    config::Config,
    tracker::StatusTracker,
    worker::EmailWorker,
};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wifikir_mail=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env()?;
    info!("Starting WifikirMail with config: {:?}", config);

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;

    info!("Connected to database");

    sqlx::migrate!("./migrations").run(&pool).await?;
    info!("Database migrations completed");

    let worker = EmailWorker::new(pool.clone(), config.clone());
    let tracker = StatusTracker::new(pool.clone(), config.clone());

    tokio::spawn(async move {
        worker.run().await;
    });

    tokio::spawn(async move {
        tracker.run().await;
    });

    let app = create_router(pool, &config);

    let addr = format!("{}:{}", config.server_host, config.server_port);
    info!("Starting HTTP server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received, starting graceful shutdown");
}
