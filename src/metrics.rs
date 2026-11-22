use once_cell::sync::Lazy;
use prometheus::{Encoder, IntCounterVec, IntGauge, Opts, Registry, TextEncoder};

pub static METRICS: Lazy<AppMetrics> = Lazy::new(AppMetrics::new);

pub struct AppMetrics {
    registry: Registry,
    pub jobs_pending: IntGauge,
    pub oldest_pending_age_seconds: IntGauge,
    pub worker_last_success_unix: IntGauge,
    pub tracker_up: IntGauge,
    send_outcomes: IntCounterVec,
}

impl AppMetrics {
    fn new() -> Self {
        let registry = Registry::new();
        let jobs_pending = IntGauge::with_opts(Opts::new(
            "email_jobs_pending",
            "Jobs in Pending, Processing or Submitting",
        ))
        .expect("metric");
        let oldest_pending_age_seconds = IntGauge::with_opts(Opts::new(
            "email_jobs_oldest_pending_age_seconds",
            "Age of the oldest Pending job in seconds",
        ))
        .expect("metric");
        let worker_last_success_unix = IntGauge::with_opts(Opts::new(
            "worker_last_success_unix",
            "Unix timestamp of the last successful Postfix handoff",
        ))
        .expect("metric");
        let tracker_up = IntGauge::with_opts(Opts::new(
            "status_tracker_up",
            "1 when the status tracker loop is running",
        ))
        .expect("metric");
        let send_outcomes = IntCounterVec::new(
            Opts::new(
                "email_send_outcomes_total",
                "SMTP handoff outcomes (no addresses or ids)",
            ),
            &["outcome"],
        )
        .expect("metric");

        registry.register(Box::new(jobs_pending.clone())).ok();
        registry
            .register(Box::new(oldest_pending_age_seconds.clone()))
            .ok();
        registry
            .register(Box::new(worker_last_success_unix.clone()))
            .ok();
        registry.register(Box::new(tracker_up.clone())).ok();
        registry.register(Box::new(send_outcomes.clone())).ok();

        Self {
            registry,
            jobs_pending,
            oldest_pending_age_seconds,
            worker_last_success_unix,
            tracker_up,
            send_outcomes,
        }
    }

    pub fn record_outcome(&self, outcome: &str) {
        self.send_outcomes.with_label_values(&[outcome]).inc();
    }

    pub fn render(&self) -> String {
        let mut buf = Vec::new();
        let encoder = TextEncoder::new();
        let families = self.registry.gather();
        encoder.encode(&families, &mut buf).ok();
        String::from_utf8(buf).unwrap_or_default()
    }
}
