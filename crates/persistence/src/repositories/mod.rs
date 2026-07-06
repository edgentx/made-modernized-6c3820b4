//! PostgreSQL repository adapters, one module per bounded context.
//!
//! # Why these are async, owned-value adapters (and not `impl Repository<A>`)
//!
//! The domain kernel's [`shared::Repository`] port is deliberately *synchronous*
//! and hands back a borrow (`fn find_by_id(&self) -> Result<Option<&A>, _>`): it
//! is shaped for the in-memory [`mocks`] adapter that owns a `HashMap<String, A>`
//! and the WASM-safe domain core, which cannot depend on an async runtime. A
//! live PostgreSQL backend cannot satisfy that signature — it has no `&A` to
//! lend (rows are reconstructed on demand), and the aggregates expose no
//! hydration constructor to rebuild one at a non-zero version. Implementing the
//! port literally would therefore require *domain-layer changes*, which this
//! story forbids.
//!
//! So, exactly as the crate's existing `leaderboard` read model does, each
//! adapter here maps rows to a persistence-local record type and exposes async
//! methods over a [`sqlx::PgPool`]. Together they are the Postgres realization
//! of every durable aggregate's persistence:
//!
//! * **Round-tripping** — `insert` then `find_by_id` reconstructs the record's
//!   fields identically (verified per aggregate in the integration tests).
//! * **Optimistic concurrency** — every mutable aggregate carries a `version`
//!   column; `update` guards on the expected version and returns a typed
//!   [`RepositoryError::Conflict`] on a miss instead of overwriting.
//! * **Transactions** — multi-row invariants (the emission ledger, collection
//!   grants, pack opening, an order and its line items) run inside a single
//!   [`sqlx::Transaction`] that rolls back atomically on any error.
//!
//! All queries use sqlx's *runtime* API rather than the compile-time-checked
//! macros: the workspace builds in `SQLX_OFFLINE=true` mode with no database in
//! reach, so a new checked query (with no cached `.sqlx/` metadata) would break
//! the offline build. The queries are exercised for real against a Postgres
//! container by the `#[sqlx::test]` integration suite.

use sqlx::PgExecutor;

use crate::error::RepositoryError;

pub mod collection;
pub mod content;
pub mod match_play;
pub mod matchmaking;
pub mod shop;
pub mod solo_ai;
pub mod token;

/// A single card grant applied to a player's collection ledger: `quantity`
/// copies of `card_definition_id`, capped at `max_copies` (the Legendary cap is
/// 1). Shared by the collection-grant and pack-opening transactional paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantLine {
    /// The catalog card being granted.
    pub card_definition_id: String,
    /// How many copies to add (must keep the balance within `[0, max_copies]`).
    pub quantity: i32,
    /// The per-card copy cap enforced by the ledger `CHECK` constraint.
    pub max_copies: i32,
}

impl GrantLine {
    /// Convenience constructor for a grant line.
    pub fn new(card_definition_id: impl Into<String>, quantity: i32, max_copies: i32) -> Self {
        Self {
            card_definition_id: card_definition_id.into(),
            quantity,
            max_copies,
        }
    }
}

/// Decide, after a version-guarded write matched **zero** rows, whether the
/// aggregate is missing entirely ([`RepositoryError::NotFound`]) or was a
/// concurrency loss ([`RepositoryError::Conflict`]).
///
/// `table` is an internal, hard-coded identifier (never caller input), so the
/// formatted lookup carries no injection risk. The lookup runs on whatever
/// executor the caller passes — the pool for a plain `update`, or the open
/// transaction for a guarded step inside a multi-row mutation.
pub(crate) async fn conflict_or_missing<'e, E>(
    executor: E,
    aggregate: &'static str,
    table: &str,
    id: &str,
    expected_version: i64,
) -> RepositoryError
where
    E: PgExecutor<'e>,
{
    let existing: Result<Option<i64>, sqlx::Error> =
        sqlx::query_scalar(&format!("SELECT version FROM {table} WHERE id = $1"))
            .bind(id)
            .fetch_optional(executor)
            .await;

    match existing {
        Ok(Some(_)) => RepositoryError::Conflict {
            aggregate,
            id: id.to_string(),
            expected_version,
        },
        Ok(None) => RepositoryError::NotFound {
            aggregate,
            id: id.to_string(),
        },
        // If even the existence probe failed, surface that raw error.
        Err(err) => RepositoryError::classify(aggregate, err),
    }
}
