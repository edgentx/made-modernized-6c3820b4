//! Shared helpers for the API-level integration suite.
//!
//! The suite drives the *production* app (assembled through [`server::configure`])
//! in-process against a live Postgres — provisioned per test by `#[sqlx::test]` —
//! and, for the WebSocket match, a live Redis. Two things every test needs live
//! here: seeding valid catalog/ranked rows through the real repository adapters,
//! and injecting the trusted-gateway identity headers the Kong/OPA sidecars would
//! set upstream (there is no auth in this service — the handlers only read those
//! headers, see `server::http::identity`).
//!
//! Each integration test crate pulls this module in via `mod common;` and uses a
//! subset of its helpers, so unused items per-crate are expected.
#![allow(dead_code)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::test::TestRequest;

use ephemeral::{connect, RedisConfig, RedisHandle};
use persistence::repositories::content::{
    CardDefinitionRepository, CardDefinitionRow, ExpansionSetRepository, ExpansionSetRow,
};
use persistence::repositories::matchmaking::{
    RankedStandingRepository, RankedStandingRow, SeasonRepository, SeasonRow,
};
use persistence::repositories::shop::{
    BattlePassRepository, BattlePassRow, CardPackRepository, CardPackRow,
};
use persistence::PgPool;

/// The tenant every injected identity acts within. A real request would carry
/// whatever tenant the gateway resolved; the suite pins a fixed one.
pub const TENANT: &str = "t-integration";

/// The trusted-gateway header names, mirroring `server::http::identity`'s
/// contract with the Kong/OPA sidecar.
const TENANT_HEADER: &str = "X-Tenant-Id";
const PLAYER_HEADER: &str = "X-Player-Id";
const ROLES_HEADER: &str = "X-Roles";

// ---------------------------------------------------------------------------
// Trusted-gateway identity injection (the sidecar contract)
// ---------------------------------------------------------------------------

/// Stamp `req` with the trusted-gateway headers for an *ordinary player*
/// (`X-Tenant-Id` + `X-Player-Id`, no roles) — exactly what the Kong/OPA sidecar
/// injects once it has authenticated the caller. No token is ever presented; the
/// service trusts these headers by contract.
pub fn as_player(req: TestRequest, player_id: &str) -> TestRequest {
    req.insert_header((TENANT_HEADER, TENANT))
        .insert_header((PLAYER_HEADER, player_id))
}

/// Stamp `req` with the headers for an internal *service account* — the
/// `service` role the gateway attaches to fulfillment/rewards callers, which
/// gates the privileged grant/pack-open operations.
pub fn as_service(req: TestRequest, player_id: &str) -> TestRequest {
    as_player(req, player_id).insert_header((ROLES_HEADER, "service"))
}

// ---------------------------------------------------------------------------
// Live-Redis handle (optional; the WS match test asserts more when present)
// ---------------------------------------------------------------------------

/// A namespaced live-Redis handle, or `None` when `MADE_REDIS_URL`/`REDIS_URL`
/// is unset. The `api-integration` CI job provisions Redis and sets the URL, so
/// the WS test exercises the real live-state mirror there; a developer without
/// Redis still runs the Postgres half.
pub async fn redis_handle(suffix: &str) -> Option<RedisHandle> {
    let url = std::env::var("MADE_REDIS_URL")
        .or_else(|_| std::env::var("REDIS_URL"))
        .ok()?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let cfg = RedisConfig::new(url)
        .with_namespace(format!("made-it-{suffix}-{nanos}"))
        .with_connect_timeout(Duration::from_millis(2_000));
    Some(connect(&cfg).await.expect("live redis should connect"))
}

// ---------------------------------------------------------------------------
// Seed helpers — valid rows through the real repository adapters
// ---------------------------------------------------------------------------

/// Seed an expansion set and one card in it, returning the card id. Card packs,
/// bosses, and the collection grant ledger all reference a catalog card, so this
/// is the shared root fixture.
pub async fn seed_card(pool: &PgPool, key: &str, rarity: &str) -> String {
    let exp_id = format!("exp-{key}");
    ExpansionSetRepository::new(pool.clone())
        .insert(&ExpansionSetRow {
            id: exp_id.clone(),
            code: format!("C-{key}"),
            name: format!("Expansion {key}"),
            version: 0,
        })
        .await
        .expect("seed expansion");
    let card_id = format!("card-{key}");
    CardDefinitionRepository::new(pool.clone())
        .insert(&CardDefinitionRow {
            id: card_id.clone(),
            expansion_set_id: exp_id,
            name: format!("Card {key}"),
            rarity: rarity.to_string(),
            cost: 1,
            effect_ref: None,
            version: 0,
        })
        .await
        .expect("seed card");
    card_id
}

/// Seed a purchasable card pack (with its backing expansion), returning its id.
pub async fn seed_card_pack(pool: &PgPool, key: &str, price: i64, count: i32) -> String {
    let exp_id = format!("exp-pack-{key}");
    ExpansionSetRepository::new(pool.clone())
        .insert(&ExpansionSetRow {
            id: exp_id.clone(),
            code: format!("P-{key}"),
            name: format!("Pack expansion {key}"),
            version: 0,
        })
        .await
        .expect("seed pack expansion");
    let pack_id = format!("pack-{key}");
    CardPackRepository::new(pool.clone())
        .insert(&CardPackRow {
            id: pack_id.clone(),
            expansion_set_id: exp_id,
            name: format!("Pack {key}"),
            price_amount: price,
            card_count: count,
            version: 0,
        })
        .await
        .expect("seed card pack");
    pack_id
}

/// Seed a season, returning its id.
pub async fn seed_season(pool: &PgPool, key: &str, number: i32) -> String {
    let id = format!("season-{key}");
    SeasonRepository::new(pool.clone())
        .insert(&SeasonRow {
            id: id.clone(),
            number,
            name: format!("Season {key}"),
            version: 0,
        })
        .await
        .expect("seed season");
    id
}

/// Seed a battle pass for `season_id`, returning its id.
pub async fn seed_battle_pass(pool: &PgPool, key: &str, season_id: &str) -> String {
    let id = format!("bp-{key}");
    BattlePassRepository::new(pool.clone())
        .insert(&BattlePassRow {
            id: id.clone(),
            season_id: season_id.to_string(),
            name: format!("Battle Pass {key}"),
            tier_count: 100,
            price_amount: 950,
            version: 0,
        })
        .await
        .expect("seed battle pass");
    id
}

/// Seed one ranked standing for a season at a given rating, returning its id.
pub async fn seed_standing(
    pool: &PgPool,
    key: &str,
    season_id: &str,
    player_id: &str,
    rating: f64,
    tier: &str,
) -> String {
    let id = format!("rs-{key}");
    RankedStandingRepository::new(pool.clone())
        .insert(&RankedStandingRow {
            id: id.clone(),
            player_id: player_id.to_string(),
            season_id: season_id.to_string(),
            rating,
            rating_dev: 60.0,
            volatility: 0.06,
            tier: tier.to_string(),
            stars: 2,
            floor_tier: "Block".to_string(),
            matches_played: 20,
            version: 0,
        })
        .await
        .expect("seed ranked standing");
    id
}
