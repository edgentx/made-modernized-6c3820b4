//! Prometheus metrics surface.
//!
//! A third operational surface alongside `/health` and the domain routes: a
//! `/metrics` endpoint rendering the process's counters in the Prometheus text
//! exposition format (v0.0.4) so the VForce360 Prometheus can scrape it. Like
//! the liveness probe it is intentionally dependency-free — no Postgres, no
//! Redis — so scraping never blocks on a downstream and the metric surface stays
//! up even when the database is unreachable.
//!
//! The registry is deliberately hand-rolled rather than pulling the `prometheus`
//! crate: the server core keeps to the workspace's zero-extra-deps ethos, and the
//! only instrument the container smoke check needs is a request counter plus
//! build/uptime gauges. Wiring is two calls from `main`: register [`Metrics`] as
//! app data and count each request with a `wrap_fn` middleware, then mount
//! [`metrics`] as a top-level service.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use actix_web::{get, web, HttpResponse, Responder};

/// Process-wide metrics registry.
///
/// Cheap to share: a monotonic start instant plus a single relaxed atomic
/// counter, so incrementing on the hot request path is uncontended and the
/// `web::Data<Metrics>` handle is just an `Arc` clone.
pub struct Metrics {
    /// When the process started, for the uptime gauge.
    started: Instant,
    /// Every HTTP request the server has accepted (all paths, including probes).
    http_requests_total: AtomicU64,
}

impl Metrics {
    /// A fresh registry with the uptime clock started now and a zeroed counter.
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            http_requests_total: AtomicU64::new(0),
        }
    }

    /// Count one accepted request. Relaxed ordering is sufficient — the value is
    /// only ever read for scraping, never to synchronize other state.
    pub fn incr_request(&self) {
        self.http_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Render the current values as a Prometheus text exposition body.
    fn render(&self) -> String {
        let total = self.http_requests_total.load(Ordering::Relaxed);
        let uptime = self.started.elapsed().as_secs_f64();
        format!(
            "# HELP made_build_info Build information for the MADE game server.\n\
             # TYPE made_build_info gauge\n\
             made_build_info{{version=\"{version}\"}} 1\n\
             # HELP made_http_requests_total Total HTTP requests accepted by the server.\n\
             # TYPE made_http_requests_total counter\n\
             made_http_requests_total {total}\n\
             # HELP made_process_uptime_seconds Seconds since the server process started.\n\
             # TYPE made_process_uptime_seconds gauge\n\
             made_process_uptime_seconds {uptime}\n",
            version = env!("CARGO_PKG_VERSION"),
        )
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// `GET /metrics` — the Prometheus scrape target.
///
/// The content type carries the exposition-format version so a scraper parses it
/// as text metrics rather than opaque `text/plain`.
#[get("/metrics")]
pub async fn metrics(registry: web::Data<Metrics>) -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4; charset=utf-8")
        .body(registry.render())
}
