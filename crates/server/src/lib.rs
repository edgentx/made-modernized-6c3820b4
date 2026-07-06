//! The MADE game server as a library.
//!
//! The `made-server` binary ([`main`](../main.rs)) is a thin `HttpServer`
//! wrapper around this crate: it builds the Postgres pool and Redis handle, then
//! mounts the two driving adapters this library exposes through [`configure`].
//!
//! Factoring the app-building surface into a library (rather than leaving it
//! private to `main.rs`) is what lets the API-level integration suite in
//! `tests/` stand the *exact* production wiring up in-process and drive it —
//! injecting the trusted-gateway identity headers the Kong/OPA sidecars would
//! set — instead of re-deriving a parallel copy of the route table that could
//! drift from what actually ships.
//!
//! * [`http`] — the versioned `/v1` REST surface over the Postgres repository
//!   adapters (collection/deck, shop, leaderboard, catalog).
//! * [`ws`] — the authoritative `/ws` WebSocket match channel over the pure
//!   [`ws::hub::MatchHub`], mirroring live state to Redis and sealing a
//!   `MatchReplay` to Postgres on completion.

pub mod http;
pub mod ws;

use actix_web::{get, web, HttpResponse, Responder};

/// Liveness probe. Binds and answers immediately, independent of Postgres,
/// Redis, or the upstream sidecars being reachable.
#[get("/health")]
pub async fn health() -> impl Responder {
    HttpResponse::Ok().body("ok")
}

/// Mount every surface the server exposes onto an actix [`App`](actix_web::App)
/// (or a nested [`web::ServiceConfig`]): the liveness probe, the structured
/// malformed-body handler, the `/ws` match channel, and the `/v1` REST scope.
///
/// The caller supplies the two [`web::Data`] states (they hold the Postgres pool
/// and Redis handle) via `App::app_data` before calling this. Both `main` and
/// the integration tests wire the app through this one function, so a test can
/// never exercise a route table that differs from production's.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.app_data(http::json_config())
        .service(health)
        .route("/ws", web::get().to(ws::game_ws))
        .configure(http::configure);
}
