//! The trusted-gateway identity extractor.
//!
//! Authentication is **not** this service's job: the Kong gateway and its OPA
//! sidecar verify the caller's JWT, evaluate policy, and then inject the
//! resolved principal into the upstream request as plain headers. By the time a
//! request reaches an actix handler the auth decision is already made, so this
//! extractor only *reads* those headers — it never parses a token, checks a
//! signature, or re-derives a claim. That is the whole point of the sidecar
//! topology: keep crypto/authz in one audited place, not smeared across every
//! service.
//!
//! Two headers are consumed:
//!
//! * `X-Tenant-Id` — the tenant the caller is acting within.
//! * `X-Player-Id` — the authenticated principal (the player subject).
//!
//! A missing header is treated as an [`ApiError::Unauthenticated`] (401): in a
//! correctly-wired deployment the gateway always sets them, so their absence
//! means the request bypassed the gateway (or it is misconfigured) — either way
//! the handler must not proceed as some ambiguous identity.

use std::future::{ready, Ready};

use actix_web::{dev::Payload, FromRequest, HttpRequest};

use super::envelope::ApiError;

/// Header the gateway sets to the caller's tenant.
const TENANT_HEADER: &str = "X-Tenant-Id";
/// Header the gateway sets to the authenticated player subject.
const PLAYER_HEADER: &str = "X-Player-Id";

/// The caller's trusted identity, lifted from gateway-set headers.
///
/// Handlers take this as an argument to scope work to the right tenant/player.
/// Crucially, *creates* draw their owner from [`Identity::player_id`] — never
/// from the request body — so a client cannot forge ownership of a resource for
/// another player.
#[derive(Debug, Clone)]
pub struct Identity {
    /// The tenant the caller is acting within (`X-Tenant-Id`).
    pub tenant_id: String,
    /// The authenticated player subject (`X-Player-Id`).
    pub player_id: String,
}

impl Identity {
    /// Emit a structured audit line for a state-changing `action`.
    ///
    /// Authentication and authorization are enforced upstream by the gateway;
    /// this only records, after the fact, *which player* acted in *which
    /// tenant* — the traceability a service behind a trusted gateway still owes
    /// even though it does not make the auth decision itself.
    pub fn audit(&self, action: &str) {
        println!(
            "audit tenant={} player={} action={action}",
            self.tenant_id, self.player_id
        );
    }
}

/// Read a required header as an owned `String`, or `None` if it is absent or not
/// valid UTF-8.
fn header(req: &HttpRequest, name: &str) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

impl FromRequest for Identity {
    type Error = ApiError;
    type Future = Ready<Result<Self, ApiError>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let tenant_id = header(req, TENANT_HEADER);
        let player_id = header(req, PLAYER_HEADER);

        ready(match (tenant_id, player_id) {
            (Some(tenant_id), Some(player_id)) => Ok(Identity {
                tenant_id,
                player_id,
            }),
            _ => Err(ApiError::Unauthenticated(format!(
                "missing trusted gateway identity headers ({TENANT_HEADER} / {PLAYER_HEADER})"
            ))),
        })
    }
}
