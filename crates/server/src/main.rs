//! Authoritative game server for MADE.
//!
//! This binary is a *driving adapter* on the outside of the hexagon. It exposes
//! two surfaces over the same domain core:
//!
//! * the authoritative WebSocket match channel ([`ws`]) on `/ws` — an `actix-ws`
//!   endpoint that drives the [`GameSession`](game_session::GameSession)
//!   aggregate server-side as the source of truth, validates client actions with
//!   the shared rules crate, broadcasts state deltas, keeps live state in Redis,
//!   and seals a `MatchReplay` to PostgreSQL when a match completes; and
//! * the versioned `/v1` REST API ([`http`]) — collection/deck, leaderboard/
//!   ranked, shop-payments, and catalog endpoints over the Postgres repository
//!   adapters.
//!
//! Both surfaces share one lazily-connected Postgres pool, so the process binds
//! and serves its liveness probe immediately without waiting on Postgres, Redis,
//! or the Kong/OPA sidecars. Auth is terminated by those sidecars upstream, so
//! there is no auth middleware here — the handlers only read the identity the
//! gateway injects.
//!
//! Two operational surfaces sit alongside the domain routes: `/health` (liveness)
//! and `/metrics` (the Prometheus scrape target, see [`metrics`]). The listen
//! address is taken from `BIND_ADDR` (default `127.0.0.1:8080` for local runs);
//! the container image sets `BIND_ADDR=0.0.0.0:8080` so the service is reachable
//! from outside the container.

// `/metrics` and the request counter are a process-level *operational* surface,
// not part of the domain route table the integration suite mounts, so they stay
// binary-local here while the domain wiring lives in `server::configure`.
mod metrics;

use actix_web::dev::Service;
use actix_web::{web, App, HttpServer};

// The REST/WS surfaces and the shared route assembly are exposed by the `server`
// library so this binary and the `tests/` integration suite mount the identical
// wiring.
use server::{configure, http, ws};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Build the Postgres pool both surfaces run over. It connects *lazily* — the
    // server binds and serves immediately, and the first request that touches the
    // database establishes the connection — so startup does not depend on
    // Postgres (or the Kong/OPA sidecars) already being reachable.
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://made:made@localhost:5432/made".to_string());
    let pool = persistence::connect_lazy(&database_url)
        .expect("DATABASE_URL must be a valid Postgres connection string");

    // Connect the ephemeral Redis handle for live match state. This fails *soft*:
    // an unreachable Redis disables live-state persistence (which is ephemeral and
    // safe to lose) rather than blocking startup.
    let redis = ws::connect_redis().await;
    if redis.is_some() {
        println!("live match state: Redis connected");
    } else {
        println!("live match state: Redis unavailable, running in-memory only");
    }

    let api_state = web::Data::new(http::ApiState::new(pool.clone()));
    let ws_state = web::Data::new(ws::WsState::new(pool, redis));
    // The metrics registry is shared across every worker so counts aggregate
    // process-wide, not per worker.
    let metrics = web::Data::new(metrics::Metrics::new());

    // Bind address is env-configurable so the same binary listens on loopback
    // locally and on 0.0.0.0 in a container (the image sets BIND_ADDR).
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    println!(
        "MADE game server listening on http://{} (REST /v1, ws /ws, /metrics)",
        bind_addr
    );

    HttpServer::new(move || {
        // Per-worker clone of the shared registry, captured by the request-
        // counting middleware below.
        let metrics_mw = metrics.clone();
        App::new()
            .app_data(api_state.clone())
            .app_data(ws_state.clone())
            .app_data(metrics.clone())
            // Count every accepted request before dispatch so /metrics reflects
            // total traffic across all surfaces.
            .wrap_fn(move |req, srv| {
                metrics_mw.incr_request();
                srv.call(req)
            })
            // Operational scrape target, kept binary-local (see module note above).
            .service(metrics::metrics)
            // The domain route table (health, malformed-body handler, `/ws`,
            // `/v1`) is assembled by `server::configure` so the integration
            // suite mounts the identical wiring this binary serves.
            .configure(configure)
    })
    .bind(bind_addr)?
    .run()
    .await
}
