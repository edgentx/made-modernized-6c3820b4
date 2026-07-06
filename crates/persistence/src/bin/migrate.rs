//! Local + CI migration runner: bring the database named by `DATABASE_URL` up
//! to the latest schema, then exit.
//!
//! This is the programmatic twin of `sqlx migrate run`. It exists so a local
//! developer (or a container entrypoint) can migrate with nothing installed but
//! the workspace binary:
//!
//! ```sh
//! DATABASE_URL=postgres://made:made@localhost:5432/made cargo run -p persistence --bin made-migrate
//! ```
//!
//! It applies the exact same embedded migration set as CI's `sqlx migrate run`.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("error: DATABASE_URL must be set (e.g. postgres://user:pass@host:5432/db)");
            return ExitCode::FAILURE;
        }
    };

    let pool = match persistence::connect(&database_url).await {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("error: could not connect to {database_url}: {err}");
            return ExitCode::FAILURE;
        }
    };

    match persistence::run_migrations(&pool).await {
        Ok(()) => {
            println!("migrations applied: database is at the latest schema");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: migration failed: {err}");
            ExitCode::FAILURE
        }
    }
}
