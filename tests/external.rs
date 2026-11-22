//! Integration tests. They run only when TEST_DATABASE_URL is set
//! (Postgres you provide). SMTP is a local mock, not a real MTA.
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wifikir_mail::api::create_router;
use wifikir_mail::auth::hash_api_key;
use wifikir_mail::bounce::apply_bounce_text;
use wifikir_mail::config::Config;
use wifikir_mail::db::{create_email_job, get_email_job, insert_api_key};
use wifikir_mail::tracker::StatusTracker;
use wifikir_mail::worker::EmailWorker;

static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_db_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .ok()
        .filter(|s| !s.is_empty())
}

async fn pool() -> Option<sqlx::PgPool> {
    let url = test_db_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect test db");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE email_job_status_history, email_jobs, api_keys CASCADE")
        .execute(&pool)
        .await
        .expect("truncate");
    Some(pool)
}

async fn spawn_smtp(reply_after_data: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let Ok((sock, _)) = listener.accept().await else {
            return;
        };
        let (read, mut write) = sock.into_split();
        let mut lines = tokio::io::BufReader::new(read).lines();
        let _ = write.write_all(b"220 test ESMTP\r\n").await;
        while let Ok(Some(line)) = lines.next_line().await {
            let up = line.to_ascii_uppercase();
            if up.starts_with("QUIT") {
                let _ = write.write_all(b"221 bye\r\n").await;
                break;
            } else if up.starts_with("DATA") {
                let _ = write.write_all(b"354 Go ahead\r\n").await;
                while let Ok(Some(body)) = lines.next_line().await {
                    if body == "." {
                        break;
                    }
                }
                let _ = write
                    .write_all(format!("{reply_after_data}\r\n").as_bytes())
                    .await;
            } else {
                let _ = write.write_all(b"250 ok\r\n").await;
            }
        }
    });
    port
}

async fn refused_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn cfg(smtp_port: u16) -> Config {
    Config {
        database_url: test_db_url().unwrap_or_default(),
        smtp_host: "127.0.0.1".into(),
        smtp_port,
        server_host: "127.0.0.1".into(),
        server_port: 0,
        mail_from: "noreply@wifikir.com".into(),
        worker_interval_secs: 60,
        tracker_poll_interval_secs: 60,
        max_attempts: 3,
        rate_limit_per_minute: 1000,
        rate_limit_global: 10000,
        job_lease_duration_secs: 30,
        queue_capacity_limit: 10000,
        template_dir: "templates".into(),
        metrics_host: "127.0.0.1".into(),
        metrics_port: 0,
        mail_log_path: "/tmp/wifikir-mail-test.log".into(),
        cursor_path: "/tmp/wifikir-mail-test.cursor".into(),
        bounce_domain: String::new(),
        bounce_maildir: String::new(),
        bounce_poll_secs: 30,
    }
}

#[tokio::test]
async fn api_idempotency_and_scope() {
    let _guard = DB_LOCK.lock().await;
    let Some(pool) = pool().await else {
        eprintln!("TEST_DATABASE_URL unset; skipping api_idempotency_and_scope");
        return;
    };
    let key = insert_api_key(&pool, "test", &hash_api_key("secret-key"))
        .await
        .unwrap();
    let other = insert_api_key(&pool, "other", &hash_api_key("other-key"))
        .await
        .unwrap();
    assert_ne!(key.id, other.id);

    let app = create_router(pool.clone(), &cfg(2525));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/emails");
    let body = json!({
        "recipient": "user@example.com",
        "templateKey": "EmailVerification",
        "referenceId": "user-1",
        "idempotencyKey": "idem-1",
        "templateData": { "verification_code": "ABC123" }
    });
    let res = client
        .post(&url)
        .header("Authorization", "Bearer secret-key")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);
    let created: serde_json::Value = res.json().await.unwrap();
    let job_id = created["jobId"].as_str().unwrap().to_string();

    let again = client
        .post(&url)
        .header("Authorization", "Bearer secret-key")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 202);
    let again_body: serde_json::Value = again.json().await.unwrap();
    assert_eq!(again_body["jobId"].as_str().unwrap(), job_id);

    let mut changed = body.clone();
    changed["templateData"]["verification_code"] = json!("OTHER");
    let conflict = client
        .post(&url)
        .header("Authorization", "Bearer secret-key")
        .json(&changed)
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);

    let hidden = client
        .get(format!("http://{addr}/v1/emails/{job_id}"))
        .header("Authorization", "Bearer other-key")
        .send()
        .await
        .unwrap();
    assert_eq!(hidden.status(), 404);

    let ready = client
        .get(format!("http://{addr}/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), 200);
}

#[tokio::test]
async fn smtp_accept_reject_and_retry() {
    let _guard = DB_LOCK.lock().await;
    let Some(pool) = pool().await else {
        eprintln!("TEST_DATABASE_URL unset; skipping smtp_accept_reject_and_retry");
        return;
    };

    let key = insert_api_key(&pool, "w", &hash_api_key("k"))
        .await
        .unwrap();
    let data = json!({ "verification_code": "ABC123" });

    let ok_port = spawn_smtp("250 2.0.0 Ok: queued as DEADBEEF01").await;
    let job = create_email_job(
        &pool,
        key.id,
        "user@example.com",
        "EmailVerification",
        "ref-ok",
        "idem-ok",
        &data,
    )
    .await
    .unwrap();

    let worker = EmailWorker::new(pool.clone(), cfg(ok_port));
    tokio::time::sleep(Duration::from_millis(50)).await;
    worker.process_once().await.unwrap();
    let stored = get_email_job(&pool, job.id).await.unwrap().unwrap();
    assert_eq!(stored.status, "SubmittedToMta");
    assert_eq!(stored.postfix_queue_id.as_deref(), Some("DEADBEEF01"));

    let bad_port = spawn_smtp("550 5.1.1 User unknown").await;
    let job2 = create_email_job(
        &pool,
        key.id,
        "bad@example.com",
        "EmailVerification",
        "ref-bad",
        "idem-bad",
        &data,
    )
    .await
    .unwrap();
    let worker = EmailWorker::new(pool.clone(), cfg(bad_port));
    tokio::time::sleep(Duration::from_millis(50)).await;
    worker.process_once().await.unwrap();
    let stored = get_email_job(&pool, job2.id).await.unwrap().unwrap();
    assert_eq!(stored.status, "Failed");

    let job3 = create_email_job(
        &pool,
        key.id,
        "later@example.com",
        "EmailVerification",
        "ref-later",
        "idem-later",
        &data,
    )
    .await
    .unwrap();
    let closed = refused_port().await;
    let worker = EmailWorker::new(pool.clone(), cfg(closed));
    worker.process_once().await.unwrap();
    let stored = get_email_job(&pool, job3.id).await.unwrap().unwrap();
    assert_eq!(stored.status, "Pending");
    assert!(stored.attempt_count >= 1);
    assert!(stored.next_attempt_at.is_some());
}

#[tokio::test]
async fn tracker_updates_job_from_log_line() {
    let _guard = DB_LOCK.lock().await;
    let Some(pool) = pool().await else {
        eprintln!("TEST_DATABASE_URL unset; skipping tracker_updates_job_from_log_line");
        return;
    };
    let key = insert_api_key(&pool, "w", &hash_api_key("k"))
        .await
        .unwrap();
    let data = json!({ "reset_code": "XYZ" });
    let job = create_email_job(
        &pool,
        key.id,
        "user@example.com",
        "PasswordReset",
        "ref",
        "idem-track",
        &data,
    )
    .await
    .unwrap();
    sqlx::query(
        "UPDATE email_jobs SET status = 'SubmittedToMta', postfix_queue_id = $1 WHERE id = $2",
    )
    .bind("3F4A5B6C7D")
    .bind(job.id)
    .execute(&pool)
    .await
    .unwrap();

    let tracker = StatusTracker::new(pool.clone(), cfg(25));
    let line = "Oct 23 10:30:45 mail postfix/smtp[12345]: 3F4A5B6C7D: to=<user@example.com>, relay=mail.example.com[1.2.3.4]:25, delay=0.5, status=sent (250 OK)";
    assert!(tracker.apply_line(line).await.unwrap());
    let stored = get_email_job(&pool, job.id).await.unwrap().unwrap();
    assert_eq!(stored.status, "AcceptedByRecipientServer");
}

#[tokio::test]
async fn verp_bounce_message_marks_job() {
    let _guard = DB_LOCK.lock().await;
    let Some(pool) = pool().await else {
        eprintln!("TEST_DATABASE_URL unset; skipping verp_bounce_message_marks_job");
        return;
    };
    let key = insert_api_key(&pool, "w", &hash_api_key("k-verp"))
        .await
        .unwrap();
    let data = json!({ "verification_code": "ABC123" });
    let job = create_email_job(
        &pool,
        key.id,
        "user@example.com",
        "EmailVerification",
        "ref-verp",
        "idem-verp",
        &data,
    )
    .await
    .unwrap();
    let raw = format!(
        "Delivered-To: bounce+{}@bounce.wifikir.com\r\nDiagnostic-Code: smtp; 550 user unknown\r\n\r\n",
        job.id
    );
    assert!(apply_bounce_text(&pool, &raw).await.unwrap());
    let stored = get_email_job(&pool, job.id).await.unwrap().unwrap();
    assert_eq!(stored.status, "Bounced");
}
