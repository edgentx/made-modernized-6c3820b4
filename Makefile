.PHONY: build test wasm run fmt clippy check migrate sqlx-prepare

# Build every crate and target (including test targets).
build:
	cargo build --workspace --all-targets

# Compile test targets without running, then run the full suite.
test:
	cargo test --workspace --no-run
	cargo test --workspace

# Compile the GameSession rules crate to WASM for the browser client.
# Requires: rustup target add wasm32-unknown-unknown (and wasm-pack for pkg output).
wasm:
	wasm-pack build crates/game-session -- --features wasm

# Run the authoritative WebSocket game server.
run:
	cargo run -p server --bin made-server

# Apply the sqlx migrations to $DATABASE_URL (local dev + CI share these files).
# e.g. DATABASE_URL=postgres://made:made@localhost:5432/made make migrate
migrate:
	cargo run -p persistence --bin made-migrate

# Regenerate the committed offline query metadata (.sqlx/) after changing a
# compile-time-checked query. Requires a live DATABASE_URL at the target schema.
sqlx-prepare:
	cargo sqlx prepare --workspace

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace --all-targets

# The local compile gate: build all targets, then compile + run tests.
check: build test
