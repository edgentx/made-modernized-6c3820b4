# Persistence: PostgreSQL schema & sqlx migrations

PostgreSQL is the **non-substitutable** durable store for MADE (no MongoDB, no
other backend). This document describes the physical schema, how to run the
migrations, and the EXPLAIN evidence for the leaderboard covering index.

## Layout

- `migrations/` (repo root) — versioned `sqlx` SQL files, one per bounded
  context, applied in forward order:

  | File | Bounded context | Aggregates / tables |
  |------|------------------|---------------------|
  | `0001_content.sql` | content | `expansion_sets`, `card_definitions`, `boss_definitions` |
  | `0002_match_play.sql` | match-play | `game_sessions`, `match_replays` |
  | `0003_matchmaking_ranked.sql` | matchmaking-and-ranked | `seasons`, `matchmaking_tickets`, `ranked_standings` |
  | `0004_collection.sql` | collection-and-deckbuilding | `player_collections`, `player_collection_cards`, `outfits`, `outfit_cards` |
  | `0005_shop_payments.sql` | shop-and-payments | `orders`, `order_line_items`, `card_packs`, `battle_passes` |
  | `0006_solo_ai.sql` | solo/AI | `ai_profiles`, `mission_attempts` |
  | `0007_token_marketplace.sql` | token-and-marketplace | `card_tokens`, `marketplace_listings`, `emission_pools`, `emission_ledger` |

- `crates/persistence` — the adapter: the `made-migrate` runner binary, the
  embedded `MIGRATOR`, and the compile-time-checked `leaderboard` read model.
- `.sqlx/` — committed offline query metadata (`cargo sqlx prepare` output).

Files migrate in dependency order: reference tables (`expansion_sets`,
`seasons`) exist before the tables whose foreign keys point at them.

## Running the migrations

Local (either path applies the same files):

```sh
export DATABASE_URL=postgres://made:made@localhost:5432/made
make migrate                       # via the made-migrate binary
# or, with sqlx-cli installed:
sqlx migrate run --source migrations
```

CI applies them to a fresh Postgres service container in the `migrations` job
(`.github/workflows/ci.yml`) and then runs `cargo sqlx prepare --workspace
--check` to fail the build if the committed `.sqlx/` metadata is stale.

Builds default to `SQLX_OFFLINE=true` via `.cargo/config.toml`, so ordinary
`cargo build` / `make build` never need a database.

## Integrity constraints

Ledger and collection tables enforce their invariants in the schema itself:

- **Non-negative balances** — `player_collection_cards.quantity >= 0`,
  `emission_pools.remaining_balance >= 0`, all money columns `>= 0`.
- **Copy caps (e.g. Legendary)** — `player_collection_cards` has a composite
  primary key `(collection_id, card_definition_id)` (one balance row per card)
  plus `CHECK (quantity <= max_copies)`; the Legendary cap is `max_copies = 1`.
- **Solvency ceiling** — `emission_pools CHECK (remaining_balance <= starting_balance)`.
- **Unique serials** — `card_tokens.serial_number` is `UNIQUE`.
- **One standing per player/season** — `UNIQUE (season_id, player_id)`.

## Leaderboard covering index (EXPLAIN)

The hot read path is "top standings in a season, ordered by rating":

```sql
SELECT player_id, rating, tier, stars
FROM ranked_standings
WHERE season_id = $1
ORDER BY rating DESC
LIMIT $2;
```

served by:

```sql
CREATE INDEX idx_ranked_standings_leaderboard
    ON ranked_standings (season_id, rating DESC)
    INCLUDE (player_id, tier, stars);
```

`EXPLAIN ANALYZE` against 5,000 seeded standings confirms an **index-only scan**
with **zero heap fetches** — the index alone satisfies the query:

```
 Limit  (cost=0.28..2.96 rows=50 width=21) (actual time=0.021..0.025 rows=50 loops=1)
   ->  Index Only Scan using idx_ranked_standings_leaderboard on ranked_standings
         (cost=0.28..267.78 rows=5000 width=21) (actual time=0.020..0.023 rows=50 loops=1)
         Index Cond: (season_id = 's1'::text)
         Heap Fetches: 0
```
