//! AIProfile bounded context.
//!
//! An [`AIProfile`] captures the tunable AI configuration for a difficulty
//! profile — the strategy kind bound to a difficulty tier and the budget/weights
//! that shape move selection. Three invariants are re-checked whenever the
//! profile's difficulty is tuned:
//!
//! 1. A difficulty tier maps to exactly one strategy kind (scripted for
//!    prologue; MCTS for Standard/Brutal/Legendary).
//! 2. MCTS move selection must stay within its configured search budget.
//! 3. Scripted profiles are deterministic for a given mission and state.
//!
//! [`TuneDifficulty`] (`TuneDifficultyCmd`) adjusts the strategy budget/weights
//! for balance. On success the aggregate applies and records the resulting
//! `difficulty.tuned` event.

use serde::{Deserialize, Serialize};

use shared::{Aggregate, AggregateRoot, Command, DomainError, DomainEvent, Repository};

/// Stable aggregate type name, surfaced in [`DomainError::UnknownCommand`].
const AGGREGATE_TYPE: &str = "AIProfile";

/// The command name that tunes a profile's difficulty budget/weights.
const TUNE_DIFFICULTY: &str = "TuneDifficultyCmd";

/// The `TuneDifficultyCmd` payload. Field names use the service's `camelCase`
/// schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TuneDifficulty {
    /// The AIProfile being tuned; must name this aggregate and must be
    /// non-empty.
    pub profile_id: String,
    /// The opaque tuning parameters (strategy budget/weights) to apply; must be
    /// non-empty.
    pub tuning_params: String,
}

impl TuneDifficulty {
    /// The command name this maps to.
    pub const COMMAND: &'static str = TUNE_DIFFICULTY;

    /// Build a command tuning `profile_id` with `tuning_params`.
    pub fn new(profile_id: impl Into<String>, tuning_params: impl Into<String>) -> Self {
        Self {
            profile_id: profile_id.into(),
            tuning_params: tuning_params.into(),
        }
    }

    /// Encode this command as a [`shared::Command`] carrying a JSON payload.
    pub fn into_command(&self) -> Command {
        let payload = serde_json::to_vec(self).expect("TuneDifficulty is always serializable");
        Command::with_payload(Self::COMMAND, payload)
    }
}

/// Story-facing alias for the command payload type.
pub type TuneDifficultyCmd = TuneDifficulty;

/// The applied difficulty tuning carried by [`Event::DifficultyTuned`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DifficultyTuned {
    /// The AIProfile whose difficulty was tuned.
    pub profile_id: String,
    /// The tuning parameters (strategy budget/weights) that were applied.
    pub tuning_params: String,
}

/// Domain events emitted by [`AIProfile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The profile's difficulty budget/weights were tuned for balance.
    DifficultyTuned(DifficultyTuned),
}

impl DomainEvent for Event {
    fn event_type(&self) -> &'static str {
        match self {
            Event::DifficultyTuned(_) => "difficulty.tuned",
        }
    }
}

/// An AI difficulty profile aggregate.
#[derive(Debug)]
pub struct AIProfile {
    id: String,
    root: AggregateRoot,
    /// Whether the profile's difficulty tier maps to exactly one strategy kind
    /// (scripted for prologue; MCTS for Standard/Brutal/Legendary).
    tier_strategy_mapping_valid: bool,
    /// Whether MCTS move selection stays within its configured search budget.
    mcts_within_search_budget: bool,
    /// Whether scripted profiles are deterministic for a given mission and state.
    scripted_deterministic: bool,
}

impl AIProfile {
    /// Create a new, tunable AI profile identified by `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root: AggregateRoot::new(),
            tier_strategy_mapping_valid: true,
            mcts_within_search_budget: true,
            scripted_deterministic: true,
        }
    }

    /// This aggregate's identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Current version (delegates to the embedded [`AggregateRoot`]).
    pub fn version(&self) -> u64 {
        self.root.version()
    }

    /// Events produced but not yet persisted.
    pub fn uncommitted_events(&self) -> &[Box<dyn DomainEvent>] {
        self.root.uncommitted_events()
    }

    /// Model whether the difficulty tier maps to exactly one strategy kind.
    pub fn set_tier_strategy_mapping_valid(&mut self, valid: bool) {
        self.tier_strategy_mapping_valid = valid;
    }

    /// Model whether MCTS move selection stays within its search budget.
    pub fn set_mcts_within_search_budget(&mut self, within: bool) {
        self.mcts_within_search_budget = within;
    }

    /// Model whether scripted profiles are deterministic for a mission and state.
    pub fn set_scripted_deterministic(&mut self, deterministic: bool) {
        self.scripted_deterministic = deterministic;
    }

    /// Tier-mapping invariant: a difficulty tier maps to exactly one strategy
    /// kind (scripted for prologue; MCTS for Standard/Brutal/Legendary).
    fn ensure_tier_strategy_mapping_valid(&self) -> Result<(), DomainError> {
        if !self.tier_strategy_mapping_valid {
            return Err(DomainError::InvariantViolation(format!(
                "AI profile '{}' binds a difficulty tier to the wrong strategy kind; a difficulty \
                 tier maps to exactly one strategy kind (scripted for prologue; MCTS for \
                 Standard/Brutal/Legendary)",
                self.id
            )));
        }
        Ok(())
    }

    /// MCTS-budget invariant: MCTS move selection must stay within its
    /// configured search budget.
    fn ensure_mcts_within_search_budget(&self) -> Result<(), DomainError> {
        if !self.mcts_within_search_budget {
            return Err(DomainError::InvariantViolation(format!(
                "AI profile '{}' exceeds its search budget; MCTS move selection must stay within \
                 its configured search budget",
                self.id
            )));
        }
        Ok(())
    }

    /// Determinism invariant: scripted profiles are deterministic for a given
    /// mission and state.
    fn ensure_scripted_deterministic(&self) -> Result<(), DomainError> {
        if !self.scripted_deterministic {
            return Err(DomainError::InvariantViolation(format!(
                "AI profile '{}' is not reproducible; scripted profiles are deterministic for a \
                 given mission and state",
                self.id
            )));
        }
        Ok(())
    }

    /// Apply an event to aggregate state.
    fn apply(&mut self, event: &Event) {
        match event {
            // Tuning adjusts strategy budget/weights; the invariant flags above
            // continue to hold for the newly tuned configuration.
            Event::DifficultyTuned(_) => {}
        }
    }

    /// Handle `TuneDifficultyCmd`.
    fn tune_difficulty(&mut self, cmd: TuneDifficulty) -> Result<Vec<Event>, DomainError> {
        if cmd.profile_id.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "AI profile '{}' requires a valid profileId to tune difficulty",
                self.id
            )));
        }
        if cmd.tuning_params.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "AI profile '{}' requires valid tuningParams to tune difficulty",
                self.id
            )));
        }
        if cmd.profile_id != self.id {
            return Err(DomainError::InvariantViolation(format!(
                "command targets AI profile '{}' but this aggregate is AI profile '{}'",
                cmd.profile_id, self.id
            )));
        }

        self.ensure_tier_strategy_mapping_valid()?;
        self.ensure_mcts_within_search_budget()?;
        self.ensure_scripted_deterministic()?;

        let event = Event::DifficultyTuned(DifficultyTuned {
            profile_id: cmd.profile_id,
            tuning_params: cmd.tuning_params,
        });
        self.apply(&event);
        self.root.record(Box::new(event.clone()));
        Ok(vec![event])
    }
}

impl Aggregate for AIProfile {
    type Event = Event;

    fn aggregate_type() -> &'static str {
        AGGREGATE_TYPE
    }

    fn execute(&mut self, command: Command) -> Result<Vec<Self::Event>, DomainError> {
        match command.name.as_str() {
            TUNE_DIFFICULTY => {
                let cmd: TuneDifficulty =
                    serde_json::from_slice(&command.payload).map_err(|e| {
                        DomainError::InvariantViolation(format!(
                            "malformed TuneDifficultyCmd payload: {e}"
                        ))
                    })?;
                self.tune_difficulty(cmd)
            }
            _ => Err(DomainError::unknown_command(
                <Self as Aggregate>::aggregate_type(),
                command.name,
            )),
        }
    }
}

/// Repository contract for the [`AIProfile`] aggregate.
pub trait AIProfileRepository: Repository<AIProfile> {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_profile() -> AIProfile {
        let mut profile = AIProfile::new("profile-01");
        profile.set_tier_strategy_mapping_valid(true);
        profile.set_mcts_within_search_budget(true);
        profile.set_scripted_deterministic(true);
        profile
    }

    fn valid_cmd() -> TuneDifficulty {
        TuneDifficulty::new("profile-01", "budget=1500;aggression=0.6")
    }

    // Scenario: Successfully execute TuneDifficultyCmd.
    #[test]
    fn tunes_difficulty_and_emits_event() {
        let mut profile = ready_profile();

        let events = profile
            .execute(valid_cmd().into_command())
            .expect("valid difficulty tuning should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type(), "difficulty.tuned");
        match &events[0] {
            Event::DifficultyTuned(tuned) => {
                assert_eq!(tuned.profile_id, "profile-01");
                assert_eq!(tuned.tuning_params, "budget=1500;aggression=0.6");
            }
        }
        assert_eq!(profile.version(), 1);
        assert_eq!(profile.uncommitted_events().len(), 1);
        assert_eq!(
            profile.uncommitted_events()[0].event_type(),
            "difficulty.tuned"
        );
    }

    // Scenario: TuneDifficultyCmd rejected - A difficulty tier maps to exactly
    // one strategy kind (scripted for prologue; MCTS for Standard/Brutal/Legendary).
    #[test]
    fn rejects_when_tier_strategy_mapping_is_invalid() {
        let mut profile = ready_profile();
        profile.set_tier_strategy_mapping_valid(false);

        let err = profile
            .execute(valid_cmd().into_command())
            .expect_err("a tier mapped to the wrong strategy kind must be rejected");

        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(profile.version(), 0);
    }

    // Scenario: TuneDifficultyCmd rejected - MCTS move selection must stay within
    // its configured search budget.
    #[test]
    fn rejects_when_mcts_exceeds_search_budget() {
        let mut profile = ready_profile();
        profile.set_mcts_within_search_budget(false);

        let err = profile
            .execute(valid_cmd().into_command())
            .expect_err("MCTS selection beyond its search budget must be rejected");

        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(profile.version(), 0);
    }

    // Scenario: TuneDifficultyCmd rejected - Scripted profiles are deterministic
    // for a given mission and state.
    #[test]
    fn rejects_when_scripted_profile_is_not_deterministic() {
        let mut profile = ready_profile();
        profile.set_scripted_deterministic(false);

        let err = profile
            .execute(valid_cmd().into_command())
            .expect_err("a non-deterministic scripted profile must be rejected");

        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(profile.version(), 0);
    }

    #[test]
    fn rejects_command_for_a_different_profile() {
        let mut profile = ready_profile();

        let err = profile
            .execute(TuneDifficulty::new("profile-99", "budget=1500").into_command())
            .expect_err("a tune command for another profile must be rejected");

        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(profile.version(), 0);
    }

    #[test]
    fn rejects_missing_profile_id() {
        let mut profile = ready_profile();

        let err = profile
            .execute(TuneDifficulty::new("   ", "budget=1500").into_command())
            .expect_err("missing profileId must be rejected");

        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(profile.version(), 0);
    }

    #[test]
    fn rejects_missing_tuning_params() {
        let mut profile = ready_profile();

        let err = profile
            .execute(TuneDifficulty::new("profile-01", "   ").into_command())
            .expect_err("missing tuningParams must be rejected");

        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(profile.version(), 0);
    }

    #[test]
    fn rejects_unknown_command() {
        let mut profile = AIProfile::new("profile-01");
        let err = profile.execute(Command::new("NoSuchCommand")).unwrap_err();

        match err {
            DomainError::UnknownCommand { aggregate, command } => {
                assert_eq!(aggregate, "AIProfile");
                assert_eq!(command, "NoSuchCommand");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn command_payload_round_trips() {
        let cmd = valid_cmd();
        let command = cmd.into_command();

        assert_eq!(command.name, TuneDifficulty::COMMAND);
        let decoded: TuneDifficulty = serde_json::from_slice(&command.payload).unwrap();
        assert_eq!(decoded, valid_cmd());
    }
}
