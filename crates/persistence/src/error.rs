//! Typed errors surfaced by the PostgreSQL repository adapters.
//!
//! The adapters never let a lost update pass silently: an optimistic-concurrency
//! miss becomes a [`RepositoryError::Conflict`] (AC — "concurrency conflicts
//! surface as a typed error rather than a silent overwrite"), and a database
//! invariant a transaction trips (a `CHECK`/`UNIQUE`/foreign-key violation — the
//! ledger non-negative balance, the Legendary copy cap, a duplicate serial, …)
//! is classified into [`RepositoryError::InvariantViolation`] rather than being
//! flattened into an opaque driver error. Everything else is a
//! [`RepositoryError::Database`].

use std::error::Error;
use std::fmt;

use sqlx::error::ErrorKind;

/// A failure raised by a PostgreSQL repository adapter.
#[derive(Debug)]
pub enum RepositoryError {
    /// Optimistic-concurrency miss: a write guarded on an expected version
    /// matched no row because the persisted version had already moved on (a
    /// concurrent writer won). The caller must reload and retry — the write was
    /// *not* applied, so nothing was silently overwritten.
    Conflict {
        /// The aggregate type whose write was rejected.
        aggregate: &'static str,
        /// Identity of the row that lost the race.
        id: String,
        /// The version the caller expected to still be current.
        expected_version: i64,
    },
    /// No row exists for the requested identity.
    NotFound {
        /// The aggregate type that was queried.
        aggregate: &'static str,
        /// The identity that produced no row.
        id: String,
    },
    /// A database invariant rejected the write: a `CHECK` (non-negative ledger
    /// balance, copy cap, pool solvency), a `UNIQUE` (duplicate serial / one
    /// live ticket per season), a foreign key, or a `NOT NULL`. In a
    /// transactional path this is what triggers the atomic rollback.
    InvariantViolation {
        /// The aggregate type whose invariant was violated.
        aggregate: &'static str,
        /// The named constraint that fired, if the driver reported one.
        constraint: Option<String>,
        /// The database's human-readable message.
        message: String,
    },
    /// Any other underlying `sqlx` / driver error (connection, protocol, …).
    Database(sqlx::Error),
}

impl RepositoryError {
    /// Classify a raw `sqlx` error for `aggregate`, promoting a Postgres
    /// integrity violation (`CHECK`/`UNIQUE`/foreign-key/`NOT NULL`) to
    /// [`RepositoryError::InvariantViolation`] and leaving everything else a
    /// [`RepositoryError::Database`]. This is how a rolled-back transaction
    /// reports *why* it rolled back.
    pub fn classify(aggregate: &'static str, err: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &err {
            let is_integrity = matches!(
                db.kind(),
                ErrorKind::UniqueViolation
                    | ErrorKind::ForeignKeyViolation
                    | ErrorKind::NotNullViolation
                    | ErrorKind::CheckViolation
            );
            if is_integrity {
                return RepositoryError::InvariantViolation {
                    aggregate,
                    constraint: db.constraint().map(str::to_string),
                    message: db.message().to_string(),
                };
            }
        }
        RepositoryError::Database(err)
    }

    /// Whether this is an optimistic-concurrency [`RepositoryError::Conflict`].
    pub fn is_conflict(&self) -> bool {
        matches!(self, RepositoryError::Conflict { .. })
    }

    /// Whether this is a database [`RepositoryError::InvariantViolation`].
    pub fn is_invariant_violation(&self) -> bool {
        matches!(self, RepositoryError::InvariantViolation { .. })
    }
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepositoryError::Conflict {
                aggregate,
                id,
                expected_version,
            } => write!(
                f,
                "optimistic-concurrency conflict on {aggregate} '{id}': expected version {expected_version} is stale"
            ),
            RepositoryError::NotFound { aggregate, id } => {
                write!(f, "no {aggregate} found for id '{id}'")
            }
            RepositoryError::InvariantViolation {
                aggregate,
                constraint,
                message,
            } => match constraint {
                Some(c) => write!(f, "{aggregate} invariant '{c}' violated: {message}"),
                None => write!(f, "{aggregate} invariant violated: {message}"),
            },
            RepositoryError::Database(err) => write!(f, "database error: {err}"),
        }
    }
}

impl Error for RepositoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            RepositoryError::Database(err) => Some(err),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for RepositoryError {
    fn from(err: sqlx::Error) -> Self {
        RepositoryError::Database(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_is_detected_and_rendered() {
        let err = RepositoryError::Conflict {
            aggregate: "Season",
            id: "2026-summer".to_string(),
            expected_version: 3,
        };
        assert!(err.is_conflict());
        assert!(!err.is_invariant_violation());
        assert_eq!(
            err.to_string(),
            "optimistic-concurrency conflict on Season '2026-summer': expected version 3 is stale"
        );
    }

    #[test]
    fn not_found_renders_aggregate_and_id() {
        let err = RepositoryError::NotFound {
            aggregate: "Order",
            id: "o-1".to_string(),
        };
        assert!(!err.is_conflict());
        assert_eq!(err.to_string(), "no Order found for id 'o-1'");
    }

    #[test]
    fn invariant_violation_reports_constraint() {
        let err = RepositoryError::InvariantViolation {
            aggregate: "EmissionPool",
            constraint: Some("emission_pools_solvent".to_string()),
            message: "new row violates check constraint".to_string(),
        };
        assert!(err.is_invariant_violation());
        assert!(err
            .to_string()
            .contains("EmissionPool invariant 'emission_pools_solvent' violated"));
    }
}
