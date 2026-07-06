# made-modernized-6c3820b4

Made — Card Game modernization: taylormade7978/made → Rust/Actix + React PWA (VForce360 Track B)

## Architecture

A Cargo workspace following **DDD hexagonal** conventions. The domain core is
framework-free and WASM-safe; adapters live at the edges.

| Crate | Role | Notes |
|-------|------|-------|
| `crates/shared` | Domain kernel (hexagon core) | `Aggregate` contract with `execute(cmd)`, `AggregateRoot` base type (version + uncommitted events), `DomainEvent`, `DomainError::UnknownCommand`, `Repository<A>` port, and the `stub_aggregate!` macro. Zero external deps. |
| `crates/game-session` | GameSession aggregate / rules | Compiles natively **and** to WASM via `wasm-pack` (`--features wasm`) for shared server/client execution. |
| `crates/domain` | The other 7 bounded contexts | MatchReplay, CardDefinition, BossDefinition, ExpansionSet, MatchmakingTicket, RankedStanding, Season — each a stub aggregate + repository contract. |
| `crates/mocks` | In-memory repository adapters | One `InMemory*Repository` per aggregate, each implementing its domain repository contract. Home of the scaffold test suite. |
| `crates/persistence` | PostgreSQL adapter | Owns the versioned `migrations/`, the `made-migrate` runner (local + CI twin of `sqlx migrate run`), and the first compile-time-checked read models. PostgreSQL is non-substitutable. |
| `crates/ephemeral` | Redis adapter | Ephemeral state on the shared VForce360 Redis: live match snapshots (TTL'd), session/presence keys, the dual-axis matchmaking queue, and match-event pub/sub. Namespaced keys, a configurable pool that fails fast on unreachable Redis. See `docs/ephemeral.md`. |
| `crates/server` | Authoritative WebSocket server | `actix-web` + `actix-ws` driving adapter, wiring aggregates to the mock repositories. |

### Persistence & migrations

The physical schema lives in the repo-root `migrations/` directory as versioned
`sqlx` SQL files, one per bounded context, applied in forward order. Every
durable aggregate has a table with a primary key and appropriate foreign keys;
ledger/collection tables enforce non-negative balances and copy caps (e.g. the
Legendary cap) via `CHECK`/`UNIQUE` constraints, and the ranked-standings
leaderboard is served by a covering index (`Index Only Scan`, verified via
`EXPLAIN` — see `docs/persistence.md`).

Builds default to `SQLX_OFFLINE=true` (via `.cargo/config.toml`), so `cargo
build` and CI's build-and-test job compile the checked queries against the
committed `.sqlx/` metadata with no database in reach. A dedicated CI job
applies the migrations to a fresh Postgres service container and checks that the
offline metadata is current.

### Ephemeral state (Redis)

Where PostgreSQL is the durable record of truth, the shared **VForce360 Redis**
holds the *ephemeral* state a live match needs and can safely lose on restart.
`crates/ephemeral` is that adapter: live `GameSession` match snapshots written
and read back with a configurable TTL, session/presence heartbeat keys, the
dual-axis (MMR + secondary) matchmaking queue the matchmaker consumes, and the
pub/sub fan-out of match events. `connect(&RedisConfig)` opens a pool sized from
the environment and **fails fast** if Redis is unreachable, and every key is
namespaced (`made:…`) so MADE never collides with a neighbouring tenant on the
shared instance. Live-Redis integration tests self-skip unless `MADE_REDIS_URL`
is set, keeping CI green with no Redis in reach — see `docs/ephemeral.md`.

### The Aggregate `Execute(cmd)` pattern

Every aggregate embeds `AggregateRoot` and implements `Aggregate::execute`, the
single write entrypoint. A command is a named message; an aggregate routes on
the name and, for anything it does not recognize, returns
`DomainError::UnknownCommand`. Stubs recognize *no* commands yet, so `execute`
currently always returns `UnknownCommand` — behavior is added by growing the
set of recognized command names.

## Commands

```sh
make build        # cargo build --workspace --all-targets
make test         # cargo test --workspace --no-run && cargo test --workspace
make wasm         # wasm-pack build crates/game-session -- --features wasm
make run          # run the actix-web + actix-ws game server on 127.0.0.1:8080
make migrate      # DATABASE_URL=... apply the sqlx migrations (local + CI)
make server-image # build the backend container image (made-server)
make server-run   # run it on :8080 (health at /health, metrics at /metrics)
make web-image    # build the PWA container image (made-pwa)
```

### Container images

Two production images, both built from the repo root as build context:

| Image | Dockerfile | Notes |
|-------|------------|-------|
| `made-server` | `crates/server/Dockerfile` | Multi-stage cargo release build of the authoritative game server into a slim, non-root Debian runtime. Compiles against the committed `.sqlx/` offline metadata (no DB at build time); embeds `migrations/` via `sqlx::migrate!`. Listens on `0.0.0.0:8080` (`BIND_ADDR`), serves `/health` (liveness) and `/metrics` (Prometheus). The seven bounded contexts are one binary, so this is one backend image. |
| `made-pwa` | `web/Dockerfile` | The React PWA: compiles the shared rules crate to WASM with `wasm-pack`, runs the Vite build, serves the static bundle via rootless NGINX. |

Both are built and (on a push to `main`) pushed by CI; the backend image additionally boots into a container health/smoke check on every run.
