//! Season bounded context — a competitive season with a start/end and rewards.
//!
//! A [`Season`] is a single ranked competition window with a fixed shape. Five
//! invariants govern the end-of-season snapshot that freezes final standings:
//!
//! 1. **Cadence & singularity** — a season runs on a fixed
//!    [`SEASON_CADENCE_WEEKS`] (12) week cadence, and only one season is open at
//!    a time; a season with the wrong length, or one snapshotted while it is not
//!    the sole open season, is inconsistent.
//! 2. **Snapshot immutability** — the end-of-season snapshot is immutable once
//!    taken and is the basis for rewards; a season already
//!    [`SeasonStatus::Snapshotted`] may not be snapshotted a second time.
//! 3. **Reward-once** — season rewards are distributed exactly once per eligible
//!    player per season; since the snapshot is the *basis* for rewards, it must
//!    be taken before any distribution — snapshotting after rewards have gone out
//!    would risk a second, duplicate distribution.
//! 4. **Soft reset at open** — a soft reset is applied to standings at season
//!    open; a season whose open did not apply that reset is malformed.
//! 5. **Leaderboard cap** — the public leaderboard exposes at most the top
//!    [`MAX_LEADERBOARD_ENTRIES`] (1000) and requires no login; a leaderboard
//!    larger than the cap is inconsistent.
//!
//! The only command implemented so far is [`SnapshotSeason`]
//! (`SnapshotSeasonCmd`): it freezes the final standings at season end,
//! enforcing every invariant, and on success emits [`Event::SeasonSnapshotted`]
//! (`season.snapshotted`). This module is hand-written (it no longer uses
//! `shared::stub_aggregate!`) but preserves the same public surface — a
//! [`Season`] aggregate and a [`SeasonRepository`] port — so the persistence
//! adapters in `crates/mocks` keep compiling unchanged.

use serde::{Deserialize, Serialize};

use shared::{Aggregate, AggregateRoot, Command, DomainError, DomainEvent, Repository};

/// Stable aggregate type name, surfaced in [`DomainError::UnknownCommand`] and
/// used for command routing.
const AGGREGATE_TYPE: &str = "Season";

/// The command name [`Season::execute`] recognizes.
const SNAPSHOT_SEASON: &str = "SnapshotSeasonCmd";

/// A season runs on a fixed cadence of this many weeks. A season whose length
/// does not match is off-cadence and inconsistent.
pub const SEASON_CADENCE_WEEKS: u32 = 12;

/// The public leaderboard exposes at most this many standings (the top 1000)
/// and requires no login.
pub const MAX_LEADERBOARD_ENTRIES: u32 = 1000;

/// Lifecycle status of a season.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SeasonStatus {
    /// The season is running; standings are live and still mutable.
    Open,
    /// The end-of-season snapshot has been taken; final standings are frozen and
    /// immutable, and serve as the basis for rewards.
    Snapshotted,
}

/// The `SnapshotSeasonCmd` payload: the season to snapshot. The field name is
/// the ranked service's `camelCase` schema.
///
/// Build one directly and turn it into a [`Command`] with
/// [`SnapshotSeason::into_command`], or decode it from a command payload via
/// [`serde_json`] inside [`Season::execute`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSeason {
    /// Identity of the season being snapshotted; must name the season this
    /// aggregate records, and must be non-empty.
    pub season_id: String,
}

impl SnapshotSeason {
    /// The command name this maps to.
    pub const COMMAND: &'static str = SNAPSHOT_SEASON;

    /// Build a command snapshotting `season_id`.
    pub fn new(season_id: impl Into<String>) -> Self {
        Self {
            season_id: season_id.into(),
        }
    }

    /// Encode this command as a [`shared::Command`] carrying a JSON payload,
    /// ready to hand to [`Season::execute`].
    pub fn into_command(&self) -> Command {
        // Serialization of a plain data struct to a Vec cannot fail here.
        let payload = serde_json::to_vec(self).expect("SnapshotSeason is always serializable");
        Command::with_payload(Self::COMMAND, payload)
    }
}

/// The frozen final standings, carried by [`Event::SeasonSnapshotted`] and thus
/// by the emitted `season.snapshotted` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonSnapshotted {
    /// The season whose standings were frozen.
    pub season_id: String,
    /// The size of the frozen public leaderboard (≤ [`MAX_LEADERBOARD_ENTRIES`]).
    pub leaderboard_size: u32,
}

/// Domain events emitted by [`Season`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The final standings were frozen at season end: the snapshot is now
    /// immutable and is the basis for reward distribution.
    SeasonSnapshotted(SeasonSnapshotted),
}

impl DomainEvent for Event {
    fn event_type(&self) -> &'static str {
        match self {
            Event::SeasonSnapshotted(_) => "season.snapshotted",
        }
    }
}

/// The Season aggregate: one ranked competition window.
///
/// Mirrors the shape produced by [`shared::stub_aggregate!`] (identity plus an
/// embedded [`AggregateRoot`]) so the surrounding wiring — the in-memory
/// repository adapters, the server — is unchanged, while it now carries the
/// season's state: its lifecycle [`SeasonStatus`], its cadence and how many
/// seasons are concurrently open, whether rewards have been distributed, whether
/// the season-open soft reset was applied, and the public leaderboard size. Its
/// `execute` handles [`SnapshotSeasonCmd`].
///
/// A fresh season from [`Season::new`] is [`SeasonStatus::Open`] on the standard
/// [`SEASON_CADENCE_WEEKS`] cadence, is the sole open season, has not yet
/// distributed rewards, applied its soft reset at open, and exposes a
/// full-but-capped leaderboard — i.e. it is snapshot-ready. The configuration
/// methods below drive it to a state a command rejects, exactly as
/// [`RankedStanding`](crate::ranked_standing) is built up before a command
/// validates it.
#[derive(Debug)]
pub struct Season {
    id: String,
    root: AggregateRoot,
    /// Current lifecycle status.
    status: SeasonStatus,
    /// The season's length in weeks; must equal [`SEASON_CADENCE_WEEKS`].
    duration_weeks: u32,
    /// How many seasons are concurrently open (including this one); only one
    /// season may be open at a time, so this must be exactly 1.
    open_season_count: u32,
    /// Whether season rewards have already been distributed. The snapshot is the
    /// basis for rewards, so it must be taken *before* distribution.
    rewards_distributed: bool,
    /// Whether the soft reset to standings was applied when the season opened.
    soft_reset_applied: bool,
    /// The size of the public leaderboard; must be within
    /// [`MAX_LEADERBOARD_ENTRIES`].
    leaderboard_size: u32,
}

impl Season {
    /// Create a new season identified by `id`: [`SeasonStatus::Open`] on the
    /// standard [`SEASON_CADENCE_WEEKS`] cadence, the sole open season, rewards
    /// not yet distributed, soft reset applied at open, and a full-but-capped
    /// leaderboard. Use the configuration methods to drive it to the state a
    /// command validates.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root: AggregateRoot::new(),
            status: SeasonStatus::Open,
            duration_weeks: SEASON_CADENCE_WEEKS,
            open_season_count: 1,
            rewards_distributed: false,
            soft_reset_applied: true,
            leaderboard_size: MAX_LEADERBOARD_ENTRIES,
        }
    }

    /// This aggregate's identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Current lifecycle status.
    pub fn status(&self) -> SeasonStatus {
        self.status
    }

    /// The size of the public leaderboard.
    pub fn leaderboard_size(&self) -> u32 {
        self.leaderboard_size
    }

    /// Current version (delegates to the embedded [`AggregateRoot`]).
    pub fn version(&self) -> u64 {
        self.root.version()
    }

    /// Events produced but not yet persisted.
    pub fn uncommitted_events(&self) -> &[Box<dyn DomainEvent>] {
        self.root.uncommitted_events()
    }

    /// Set the lifecycle status (e.g. to a season already snapshotted).
    pub fn set_status(&mut self, status: SeasonStatus) {
        self.status = status;
    }

    /// Set the season cadence: its length in weeks and how many seasons are
    /// concurrently open (including this one).
    pub fn set_cadence(&mut self, duration_weeks: u32, open_season_count: u32) {
        self.duration_weeks = duration_weeks;
        self.open_season_count = open_season_count;
    }

    /// Record whether season rewards have already been distributed.
    pub fn set_rewards_distributed(&mut self, distributed: bool) {
        self.rewards_distributed = distributed;
    }

    /// Record whether the season-open soft reset was applied to standings.
    pub fn set_soft_reset_applied(&mut self, applied: bool) {
        self.soft_reset_applied = applied;
    }

    /// Set the public leaderboard size.
    pub fn set_leaderboard_size(&mut self, leaderboard_size: u32) {
        self.leaderboard_size = leaderboard_size;
    }

    /// Cadence-and-singularity invariant: a season runs on a fixed
    /// [`SEASON_CADENCE_WEEKS`] cadence and only one season is open at a time.
    fn ensure_cadence_and_single_open(&self) -> Result<(), DomainError> {
        if self.duration_weeks != SEASON_CADENCE_WEEKS {
            return Err(DomainError::InvariantViolation(format!(
                "season '{}' runs for {} weeks but a season runs on a fixed {}-week cadence",
                self.id, self.duration_weeks, SEASON_CADENCE_WEEKS
            )));
        }
        if self.open_season_count != 1 {
            return Err(DomainError::InvariantViolation(format!(
                "season '{}' is one of {} concurrently open seasons, but only one season is open \
                 at a time",
                self.id, self.open_season_count
            )));
        }
        Ok(())
    }

    /// Snapshot-immutability invariant: the end-of-season snapshot is immutable
    /// once taken, so a season already snapshotted may not be snapshotted again.
    fn ensure_snapshot_not_taken(&self) -> Result<(), DomainError> {
        if self.status == SeasonStatus::Snapshotted {
            return Err(DomainError::InvariantViolation(format!(
                "season '{}' is already snapshotted; the end-of-season snapshot is immutable once \
                 taken",
                self.id
            )));
        }
        Ok(())
    }

    /// Reward-once invariant: rewards are distributed exactly once per eligible
    /// player per season, and the snapshot is their basis — so it must be taken
    /// before any distribution. Snapshotting after rewards have gone out would
    /// risk a second, duplicate distribution.
    fn ensure_rewards_not_distributed(&self) -> Result<(), DomainError> {
        if self.rewards_distributed {
            return Err(DomainError::InvariantViolation(format!(
                "season '{}' has already distributed rewards; the snapshot is the basis for the \
                 exactly-once-per-player distribution and must be taken before it",
                self.id
            )));
        }
        Ok(())
    }

    /// Soft-reset invariant: a soft reset is applied to standings at season open,
    /// so a season whose open did not apply it is malformed.
    fn ensure_soft_reset_applied(&self) -> Result<(), DomainError> {
        if !self.soft_reset_applied {
            return Err(DomainError::InvariantViolation(format!(
                "season '{}' did not apply the soft reset to standings at season open",
                self.id
            )));
        }
        Ok(())
    }

    /// Leaderboard-cap invariant: the public leaderboard exposes at most the top
    /// [`MAX_LEADERBOARD_ENTRIES`].
    fn ensure_leaderboard_within_cap(&self) -> Result<(), DomainError> {
        if self.leaderboard_size > MAX_LEADERBOARD_ENTRIES {
            return Err(DomainError::InvariantViolation(format!(
                "season '{}' exposes a {}-entry public leaderboard but it may expose at most the \
                 top {}",
                self.id, self.leaderboard_size, MAX_LEADERBOARD_ENTRIES
            )));
        }
        Ok(())
    }

    /// Handle `SnapshotSeasonCmd`: verify the command targets this season with a
    /// valid identity, enforce every invariant (cadence & singularity, snapshot
    /// immutability, reward-once, soft reset at open, and leaderboard cap), freeze
    /// the final standings, and emit [`Event::SeasonSnapshotted`].
    fn snapshot_season(&mut self, cmd: SnapshotSeason) -> Result<Vec<Event>, DomainError> {
        // A valid seasonId must be supplied.
        if cmd.season_id.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "season '{}' requires a valid seasonId to snapshot",
                self.id
            )));
        }
        // The command must name the season this aggregate actually records.
        if cmd.season_id != self.id {
            return Err(DomainError::InvariantViolation(format!(
                "command targets season '{}' but this aggregate records '{}'",
                cmd.season_id, self.id
            )));
        }

        // Enforce every invariant before freezing the standings.
        self.ensure_cadence_and_single_open()?;
        self.ensure_snapshot_not_taken()?;
        self.ensure_rewards_not_distributed()?;
        self.ensure_soft_reset_applied()?;
        self.ensure_leaderboard_within_cap()?;

        let event = Event::SeasonSnapshotted(SeasonSnapshotted {
            season_id: cmd.season_id,
            leaderboard_size: self.leaderboard_size,
        });
        // Freeze the final standings: the snapshot is now immutable.
        self.status = SeasonStatus::Snapshotted;
        self.root.record(Box::new(event.clone()));
        Ok(vec![event])
    }
}

impl Aggregate for Season {
    type Event = Event;

    fn aggregate_type() -> &'static str {
        AGGREGATE_TYPE
    }

    fn execute(&mut self, command: Command) -> Result<Vec<Self::Event>, DomainError> {
        match command.name.as_str() {
            SNAPSHOT_SEASON => {
                let cmd: SnapshotSeason =
                    serde_json::from_slice(&command.payload).map_err(|e| {
                        DomainError::InvariantViolation(format!(
                            "malformed SnapshotSeasonCmd payload: {e}"
                        ))
                    })?;
                self.snapshot_season(cmd)
            }
            // Any other command is unknown to this aggregate.
            _ => Err(DomainError::unknown_command(
                <Self as Aggregate>::aggregate_type(),
                command.name,
            )),
        }
    }
}

/// Repository contract for the [`Season`] aggregate. Adapters implement
/// [`shared::Repository`] for `Season` and then this marker trait.
pub trait SeasonRepository: Repository<Season> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot-ready season `s-01`: open, on the 12-week cadence, the sole
    /// open season, rewards not yet distributed, soft reset applied at open, and
    /// a full-but-capped leaderboard. Tests mutate one aspect at a time to drive
    /// a specific rejection.
    fn ready_season() -> Season {
        let mut season = Season::new("s-01");
        season.set_status(SeasonStatus::Open);
        season.set_cadence(SEASON_CADENCE_WEEKS, 1);
        season.set_rewards_distributed(false);
        season.set_soft_reset_applied(true);
        season.set_leaderboard_size(MAX_LEADERBOARD_ENTRIES);
        season
    }

    /// A command snapshotting `s-01`.
    fn valid_cmd() -> SnapshotSeason {
        SnapshotSeason::new("s-01")
    }

    // Scenario: Successfully execute SnapshotSeasonCmd.
    #[test]
    fn snapshots_and_emits_season_snapshotted_event() {
        let mut season = ready_season();

        let events = season
            .execute(valid_cmd().into_command())
            .expect("valid snapshot should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type(), "season.snapshotted");
        match &events[0] {
            Event::SeasonSnapshotted(snapshot) => {
                assert_eq!(snapshot.season_id, "s-01");
                assert_eq!(snapshot.leaderboard_size, MAX_LEADERBOARD_ENTRIES);
            }
        }
        // The season recorded the event and is now frozen.
        assert_eq!(season.status(), SeasonStatus::Snapshotted);
        assert_eq!(season.version(), 1);
        assert_eq!(season.uncommitted_events().len(), 1);
        assert_eq!(
            season.uncommitted_events()[0].event_type(),
            "season.snapshotted"
        );
    }

    // Scenario: rejected — a season runs on a 12-week cadence; only one season is
    // open at a time (off-cadence length).
    #[test]
    fn rejects_when_off_cadence() {
        let mut season = ready_season();
        // An 8-week season is off the fixed 12-week cadence.
        season.set_cadence(SEASON_CADENCE_WEEKS - 4, 1);

        let err = season
            .execute(valid_cmd().into_command())
            .expect_err("an off-cadence season must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(season.version(), 0);
    }

    // Scenario: rejected — only one season is open at a time.
    #[test]
    fn rejects_when_multiple_seasons_open() {
        let mut season = ready_season();
        // Two seasons open at once breaks the singularity rule.
        season.set_cadence(SEASON_CADENCE_WEEKS, 2);

        let err = season
            .execute(valid_cmd().into_command())
            .expect_err("a non-sole open season must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(season.version(), 0);
    }

    // Scenario: rejected — the end-of-season snapshot is immutable once taken.
    #[test]
    fn rejects_when_already_snapshotted() {
        let mut season = ready_season();
        // A season already snapshotted cannot be snapshotted again.
        season.set_status(SeasonStatus::Snapshotted);

        let err = season
            .execute(valid_cmd().into_command())
            .expect_err("a re-snapshot must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(season.version(), 0);
    }

    // Scenario: rejected — season rewards are distributed exactly once per
    // eligible player per season.
    #[test]
    fn rejects_when_rewards_already_distributed() {
        let mut season = ready_season();
        // Rewards already went out; the snapshot is their basis and must precede.
        season.set_rewards_distributed(true);

        let err = season
            .execute(valid_cmd().into_command())
            .expect_err("snapshotting after rewards were distributed must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(season.version(), 0);
    }

    // Scenario: rejected — a soft reset is applied to standings at season open.
    #[test]
    fn rejects_when_soft_reset_not_applied() {
        let mut season = ready_season();
        // The season opened without applying the soft reset.
        season.set_soft_reset_applied(false);

        let err = season
            .execute(valid_cmd().into_command())
            .expect_err("a missing season-open soft reset must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(season.version(), 0);
    }

    // Scenario: rejected — the public leaderboard exposes at most the top 1000.
    #[test]
    fn rejects_when_leaderboard_exceeds_cap() {
        let mut season = ready_season();
        // One over the top-1000 cap is inconsistent.
        season.set_leaderboard_size(MAX_LEADERBOARD_ENTRIES + 1);

        let err = season
            .execute(valid_cmd().into_command())
            .expect_err("an over-cap leaderboard must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(season.version(), 0);
    }

    // A command naming a different season is rejected before any invariant runs.
    #[test]
    fn rejects_command_for_a_different_season() {
        let mut season = ready_season();
        let cmd = SnapshotSeason::new("s-99");

        let err = season
            .execute(cmd.into_command())
            .expect_err("a command for another season must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(season.version(), 0);
    }

    // A command with no seasonId is rejected.
    #[test]
    fn rejects_command_without_a_season_id() {
        let mut season = ready_season();
        let cmd = SnapshotSeason::new("   ");

        let err = season
            .execute(cmd.into_command())
            .expect_err("a missing seasonId must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(season.version(), 0);
    }

    // An unrecognized command is still an UnknownCommand for this aggregate,
    // preserving the contract the mock adapters rely on.
    #[test]
    fn rejects_unknown_command() {
        let mut season = Season::new("s-01");
        let err = season.execute(Command::new("NoSuchCommand")).unwrap_err();
        match err {
            DomainError::UnknownCommand { aggregate, command } => {
                assert_eq!(aggregate, "Season");
                assert_eq!(command, "NoSuchCommand");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn command_payload_round_trips() {
        let cmd = valid_cmd();
        let command = cmd.into_command();
        assert_eq!(command.name, SnapshotSeason::COMMAND);
        let decoded: SnapshotSeason = serde_json::from_slice(&command.payload).unwrap();
        assert_eq!(decoded, valid_cmd());
    }
}
