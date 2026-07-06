# syntax=docker/dockerfile:1.7
#
# Multi-stage production build for the MADE backend (`made-server`).
#
#   1. builder — cargo release build of the `server` crate against the committed
#      sqlx offline metadata, so no live database is needed at image-build time.
#   2. runtime — slim, non-root Debian image carrying just the static-ish binary,
#      listening on 0.0.0.0:8080 and exposing /health and /metrics.
#
# The MADE modernization consolidated the game-server, matchmaking, leaderboard,
# collection-deck and shop-payments bounded contexts into this single
# authoritative binary (REST /v1 + the /ws match channel over one Postgres pool),
# so there is one backend image rather than one per context.
#
# Build context is the REPO ROOT — the build needs the whole Cargo workspace, the
# committed `.sqlx/` query metadata, and `migrations/` (embedded at compile time
# by `sqlx::migrate!`). Build with:
#   docker build -f crates/server/Dockerfile -t made-server .

# ── Stage 1: cargo release build ──────────────────────────────────────────────
FROM rust:1-bookworm AS builder
WORKDIR /src

# Compile against the committed offline metadata — belt-and-suspenders alongside
# the workspace's .cargo/config.toml default, so the build never reaches for a DB.
ENV SQLX_OFFLINE=true

# Copy the workspace. Manifests + lockfile first so the dependency graph is
# resolved deterministically from the committed Cargo.lock; then the cargo config
# (the SQLX_OFFLINE default), the offline query metadata, the crates, and the
# migrations the persistence crate embeds at compile time.
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY .sqlx ./.sqlx
COPY crates ./crates
COPY migrations ./migrations

# Release build of just the server binary. Cache mounts on the cargo registry and
# the target dir keep rebuilds fast in CI without affecting reproducibility (the
# lockfile pins the inputs); the binary is copied out of the cached target dir
# within the same layer so it survives into the image.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p server --bin made-server \
    && cp /src/target/release/made-server /usr/local/bin/made-server

# ── Stage 2: slim runtime ─────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# curl backs the container HEALTHCHECK; ca-certificates is standard hygiene for a
# service that may open TLS connections to its backing stores. --no-install-
# recommends and cleaning the apt lists keep the image slim.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Provenance labels; VERSION is stamped by CI (see .github/workflows/ci.yml).
ARG VERSION=dev
LABEL org.opencontainers.image.title="made-server" \
      org.opencontainers.image.description="MADE authoritative game server (actix-web REST /v1 + actix-ws /ws)" \
      org.opencontainers.image.source="https://github.com/edgentx/made-modernized-6c3820b4" \
      org.opencontainers.image.version="${VERSION}"

# Run as a dedicated non-root user — no shell, no home, a fixed high uid so the
# filesystem owner is stable across rebuilds.
RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin made
USER made

COPY --from=builder /usr/local/bin/made-server /usr/local/bin/made-server

# Listen on all interfaces so the service is reachable outside the container
# (the binary defaults to 127.0.0.1 for local runs). DATABASE_URL / MADE_REDIS_URL
# are injected by the deploy environment; both connect lazily / fail soft, so the
# process still binds and serves its probes without them.
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080

# Liveness/smoke: /health returns a cheap 200 with no backing services required.
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/made-server"]
