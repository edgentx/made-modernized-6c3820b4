//! API-level integration tests for the `/v1` REST surface.
//!
//! Each test provisions its own ephemeral Postgres database via `#[sqlx::test]`
//! (running the embedded migrations first), stands the *production* app up
//! in-process through [`server::configure`], and drives it with
//! `actix_web::test` — injecting the trusted-gateway identity headers a real
//! request would arrive with. The collection/deck, shop, and leaderboard
//! contexts are each covered with both success and error paths, asserting the
//! HTTP status and the shared success/error envelope the handlers render.
//!
//! Gated behind the `integration-tests` feature so the DB-free `cargo test
//! --workspace` job never tries to run them without a database.
#![cfg(feature = "integration-tests")]

mod common;

use actix_web::http::header::ContentType;
use actix_web::{test, web, App};
use serde_json::{json, Value};

use persistence::PgPool;
use server::http::ApiState;
use server::ws::WsState;

/// Build the production app over `pool` and initialize it for in-process
/// testing. WS state is present (the `/ws` route mounts through the same
/// `configure`) but unused by these REST cases.
macro_rules! rest_app {
    ($pool:expr) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new(ApiState::new($pool.clone())))
                .app_data(web::Data::new(WsState::new($pool.clone(), None)))
                .configure(server::configure),
        )
        .await
    };
}

// ===========================================================================
// Collection / deck
// ===========================================================================

#[sqlx::test(migrator = "persistence::MIGRATOR")]
async fn collection_create_read_and_object_ownership(pool: PgPool) {
    let app = rest_app!(pool);

    // Create — the owner is taken from the identity header, never the body.
    let req = common::as_player(test::TestRequest::post().uri("/v1/collections"), "p-alice")
        .set_json(json!({ "id": "col-alice" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 201, "collection create is 201");
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["player_id"], "p-alice");
    assert_eq!(body["data"]["version"], 0);

    // Owner reads it back.
    let req = common::as_player(
        test::TestRequest::get().uri("/v1/collections/col-alice"),
        "p-alice",
    )
    .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);

    // A different player must not learn it exists — ownership mismatch is a 404,
    // not a 403 (so an id cannot be probed).
    let req = common::as_player(
        test::TestRequest::get().uri("/v1/collections/col-alice"),
        "p-mallory",
    )
    .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 404, "non-owner read is 404");
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "not_found");

    // A truly missing collection is also a 404.
    let req = common::as_player(
        test::TestRequest::get().uri("/v1/collections/ghost"),
        "p-alice",
    )
    .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 404);
}

#[sqlx::test(migrator = "persistence::MIGRATOR")]
async fn granting_cards_is_service_role_only(pool: PgPool) {
    let card = common::seed_card(&pool, "grant", "Common").await;
    let app = rest_app!(pool);

    // Open a collection for the player.
    let req = common::as_player(test::TestRequest::post().uri("/v1/collections"), "p-bob")
        .set_json(json!({ "id": "col-bob" }))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status().as_u16(), 201);

    let grant_body = json!({
        "expected_version": 0,
        "grants": [ { "card_definition_id": card, "quantity": 2, "max_copies": 3 } ],
    });

    // An ordinary player may not mint cards — 403.
    let req = common::as_player(
        test::TestRequest::post().uri("/v1/collections/col-bob/grants"),
        "p-bob",
    )
    .set_json(&grant_body)
    .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 403, "player grant is forbidden");
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "forbidden");

    // The fulfillment service (service role) may — 200, and the ledger reflects it.
    let req = common::as_service(
        test::TestRequest::post().uri("/v1/collections/col-bob/grants"),
        "svc-fulfillment",
    )
    .set_json(&grant_body)
    .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "service grant succeeds");
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["version"], 1, "grant bumped the version");
    assert_eq!(body["data"]["cards"][0]["quantity"], 2);

    // An empty grant batch fails shape validation — 400 with field details.
    let req = common::as_service(
        test::TestRequest::post().uri("/v1/collections/col-bob/grants"),
        "svc-fulfillment",
    )
    .set_json(json!({ "expected_version": 1, "grants": [] }))
    .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "validation_error");
}

#[sqlx::test(migrator = "persistence::MIGRATOR")]
async fn missing_gateway_identity_is_unauthenticated(pool: PgPool) {
    let app = rest_app!(pool);

    // No trusted-gateway headers at all: the identity extractor rejects with 401
    // (the request bypassed the gateway).
    let req = test::TestRequest::get()
        .uri("/v1/collections/anything")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "unauthenticated");
}

// ===========================================================================
// Shop / orders / packs
// ===========================================================================

#[sqlx::test(migrator = "persistence::MIGRATOR")]
async fn order_totals_are_computed_and_scoped_to_owner(pool: PgPool) {
    let app = rest_app!(pool);

    // Two line items: totals are derived server-side, not trusted from the body.
    let req = common::as_player(test::TestRequest::post().uri("/v1/orders"), "p-carol")
        .set_json(json!({
            "id": "order-1",
            "currency": "USD",
            "items": [
                { "id": "li-1", "sku": "pack.core", "unit_amount": 499, "quantity": 2 },
                { "id": "li-2", "sku": "pass.s1", "unit_amount": 950, "quantity": 1 }
            ]
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 201);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["player_id"], "p-carol");
    assert_eq!(body["data"]["total_amount"], 499 * 2 + 950);
    assert_eq!(body["data"]["status"], "Created");
    assert_eq!(body["data"]["items"][0]["line_amount"], 499 * 2);

    // The purchaser reads their order.
    let req = common::as_player(
        test::TestRequest::get().uri("/v1/orders/order-1"),
        "p-carol",
    )
    .to_request();
    assert_eq!(test::call_service(&app, req).await.status().as_u16(), 200);

    // Another player cannot — 404 (ownership mismatch is not disclosed).
    let req = common::as_player(test::TestRequest::get().uri("/v1/orders/order-1"), "p-dave")
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status().as_u16(), 404);

    // An order with no line items fails validation — 400.
    let req = common::as_player(test::TestRequest::post().uri("/v1/orders"), "p-carol")
        .set_json(json!({ "id": "order-empty", "currency": "USD", "items": [] }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
    assert_eq!(
        test::read_body_json::<Value, _>(resp).await["error"]["code"],
        "validation_error"
    );
}

#[sqlx::test(migrator = "persistence::MIGRATOR")]
async fn card_pack_read_hit_and_miss(pool: PgPool) {
    let pack = common::seed_card_pack(&pool, "core", 499, 5).await;
    let app = rest_app!(pool);

    let req = common::as_player(
        test::TestRequest::get().uri(&format!("/v1/card-packs/{pack}")),
        "p-erin",
    )
    .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["price_amount"], 499);
    assert_eq!(body["data"]["card_count"], 5);

    // A malformed JSON body on a write endpoint renders the same 400 envelope
    // (the JsonConfig error handler), proving the uniform malformed-payload path.
    let req = common::as_player(test::TestRequest::post().uri("/v1/orders"), "p-erin")
        .insert_header(ContentType::json())
        .set_payload("{ this is not json ")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
    assert_eq!(
        test::read_body_json::<Value, _>(resp).await["error"]["code"],
        "validation_error"
    );

    // A missing pack is a 404.
    let req = common::as_player(
        test::TestRequest::get().uri("/v1/card-packs/nope"),
        "p-erin",
    )
    .to_request();
    assert_eq!(test::call_service(&app, req).await.status().as_u16(), 404);
}

// ===========================================================================
// Leaderboard / ranked
// ===========================================================================

#[sqlx::test(migrator = "persistence::MIGRATOR")]
async fn leaderboard_orders_by_rating_and_clamps_limit(pool: PgPool) {
    let season = common::seed_season(&pool, "ldr", 1).await;
    common::seed_standing(&pool, "low", &season, "p-low", 1100.0, "Block").await;
    common::seed_standing(&pool, "high", &season, "p-high", 1900.0, "Legend").await;
    common::seed_standing(&pool, "mid", &season, "p-mid", 1500.0, "Champion").await;
    let app = rest_app!(pool);

    // Page of 2, ordered by hidden rating descending.
    let req = common::as_player(
        test::TestRequest::get().uri(&format!("/v1/seasons/{season}/leaderboard?limit=2")),
        "p-viewer",
    )
    .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["limit"], 2);
    let entries = body["data"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "limit is honored");
    assert_eq!(entries[0]["player_id"], "p-high");
    assert_eq!(entries[1]["player_id"], "p-mid");

    // A single standing reads back with its full shape.
    let req = common::as_player(
        test::TestRequest::get().uri("/v1/ranked-standings/rs-high"),
        "p-viewer",
    )
    .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        test::read_body_json::<Value, _>(resp).await["data"]["tier"],
        "Legend"
    );

    // Missing season and missing standing are both 404.
    let req = common::as_player(
        test::TestRequest::get().uri("/v1/seasons/no-such-season"),
        "p-viewer",
    )
    .to_request();
    assert_eq!(test::call_service(&app, req).await.status().as_u16(), 404);

    let req = common::as_player(
        test::TestRequest::get().uri("/v1/ranked-standings/no-such-standing"),
        "p-viewer",
    )
    .to_request();
    assert_eq!(test::call_service(&app, req).await.status().as_u16(), 404);
}
