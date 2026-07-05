//! MissionAttempt bounded context - a player's PvE mission attempt.
//!
//! A [`MissionAttempt`] models a player beginning an authored mission and later
//! claiming its fixed first-clear reward. The same four mission invariants are
//! checked before either command records an event:
//!
//! 1. The fixed $MADE reward for a mission is granted only on the player's first
//!    clear, ever.
//! 2. Prologue missions are gated in sequence; a mission unlocks only after its
//!    predecessor is cleared.
//! 3. Only missions in today's Reprise rotation are eligible for repeat rewards.
//! 4. Per-mission special rules and boss HP-threshold barks fire exactly at
//!    their scripted points.
//!
//! [`StartMission`] (`StartMissionCmd`) begins the attempt and emits
//! [`Event::MissionStarted`] (`mission.started`). [`ClaimFirstClearReward`]
//! (`ClaimFirstClearRewardCmd`) claims the fixed first-clear reward and emits
//! [`Event::FirstClearRewardClaimed`] (`first.clear.reward.claimed`).

use serde::{Deserialize, Serialize};

use shared::{Aggregate, AggregateRoot, Command, DomainError, DomainEvent, Repository};

/// Stable aggregate type name, surfaced in [`DomainError::UnknownCommand`] and
/// used for command routing.
const AGGREGATE_TYPE: &str = "MissionAttempt";

/// The command name [`MissionAttempt::execute`] recognizes to begin an attempt.
const START_MISSION: &str = "StartMissionCmd";

/// The command name that claims a mission's first-clear reward bundle.
const CLAIM_FIRST_CLEAR_REWARD: &str = "ClaimFirstClearRewardCmd";

/// The `StartMissionCmd` payload: which player is starting which mission. Field
/// names use the story service's `camelCase` schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartMission {
    /// The player starting the mission; must be non-empty.
    pub player_id: String,
    /// The mission being started; must be non-empty.
    pub mission_id: String,
}

impl StartMission {
    /// The command name this maps to.
    pub const COMMAND: &'static str = START_MISSION;

    /// Build a command starting `mission_id` for `player_id`.
    pub fn new(player_id: impl Into<String>, mission_id: impl Into<String>) -> Self {
        Self {
            player_id: player_id.into(),
            mission_id: mission_id.into(),
        }
    }

    /// Encode this command as a [`shared::Command`] carrying a JSON payload,
    /// ready to hand to [`MissionAttempt::execute`].
    pub fn into_command(&self) -> Command {
        let payload = serde_json::to_vec(self).expect("StartMission is always serializable");
        Command::with_payload(Self::COMMAND, payload)
    }
}

/// Story-facing alias for the start command payload type.
pub type StartMissionCmd = StartMission;

/// The `ClaimFirstClearRewardCmd` payload. Field names use the service's
/// `camelCase` schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimFirstClearReward {
    /// The player claiming the reward; must be non-empty.
    pub player_id: String,
    /// The mission whose first-clear reward is claimed; must be non-empty.
    pub mission_id: String,
}

impl ClaimFirstClearReward {
    /// The command name this maps to.
    pub const COMMAND: &'static str = CLAIM_FIRST_CLEAR_REWARD;

    /// Build a command payload for `player_id` claiming `mission_id`.
    pub fn new(player_id: impl Into<String>, mission_id: impl Into<String>) -> Self {
        Self {
            player_id: player_id.into(),
            mission_id: mission_id.into(),
        }
    }

    /// Encode this command as a [`shared::Command`] carrying a JSON payload.
    pub fn into_command(&self) -> Command {
        let payload =
            serde_json::to_vec(self).expect("ClaimFirstClearReward is always serializable");
        Command::with_payload(Self::COMMAND, payload)
    }
}

/// Story-facing alias for the reward claim command payload type.
pub type ClaimFirstClearRewardCmd = ClaimFirstClearReward;

/// The mission attempt that began, carried by [`Event::MissionStarted`] and thus
/// by the emitted `mission.started` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionStarted {
    /// The attempt aggregate that started.
    pub attempt_id: String,
    /// The player starting the mission.
    pub player_id: String,
    /// The mission being started.
    pub mission_id: String,
    /// Intro panels to show as the attempt begins.
    pub intro_panel_ids: Vec<String>,
}

/// The first-clear reward claim carried by [`Event::FirstClearRewardClaimed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstClearRewardClaimed {
    /// The player that claimed the reward.
    pub player_id: String,
    /// The mission whose first-clear reward was claimed.
    pub mission_id: String,
}

/// Domain events emitted by [`MissionAttempt`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A player began a mission attempt and should see its intro panels.
    MissionStarted(MissionStarted),
    /// The fixed first-clear reward bundle was claimed for a player and mission.
    FirstClearRewardClaimed(FirstClearRewardClaimed),
}

impl DomainEvent for Event {
    fn event_type(&self) -> &'static str {
        match self {
            Event::MissionStarted(_) => "mission.started",
            Event::FirstClearRewardClaimed(_) => "first.clear.reward.claimed",
        }
    }
}

/// The MissionAttempt aggregate.
///
/// Mirrors the shape produced by [`shared::stub_aggregate!`] (identity plus an
/// embedded [`AggregateRoot`]) so repository wiring stays consistent with the
/// rest of the domain crate, while carrying the state required by the start and
/// reward-claim commands.
#[derive(Debug)]
pub struct MissionAttempt {
    id: String,
    root: AggregateRoot,
    /// The authored mission this attempt targets. Empty means the mission is
    /// established by the first valid start command, preserving older callers
    /// that constructed an attempt with only an aggregate id.
    mission_id: String,
    /// Intro panels to show when the attempt starts.
    intro_panel_ids: Vec<String>,
    /// Whether this player's first-clear reward has already been claimed.
    first_clear_reward_claimed: bool,
    /// Whether the fixed $MADE reward rule is currently satisfied for starts.
    fixed_made_reward_first_clear_only: bool,
    /// Whether this Prologue mission's predecessor has been cleared.
    prologue_predecessor_cleared: bool,
    /// Whether this mission is eligible for repeat rewards in today's Reprise
    /// rotation.
    reprise_rotation_eligible_today: bool,
    /// Whether per-mission special rules and boss HP-threshold barks are wired to
    /// their scripted trigger points.
    scripted_rules_and_barks_at_scripted_points: bool,
    /// Whether this attempt has already started.
    started: bool,
}

impl MissionAttempt {
    /// Create a new mission attempt identified by `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root: AggregateRoot::new(),
            mission_id: String::new(),
            intro_panel_ids: vec!["intro-panel-1".to_string()],
            first_clear_reward_claimed: false,
            fixed_made_reward_first_clear_only: true,
            prologue_predecessor_cleared: true,
            reprise_rotation_eligible_today: true,
            scripted_rules_and_barks_at_scripted_points: true,
            started: false,
        }
    }

    /// Create a new, startable MissionAttempt identified by `id` and targeting
    /// `mission_id`.
    pub fn for_mission(id: impl Into<String>, mission_id: impl Into<String>) -> Self {
        let mut attempt = Self::new(id);
        attempt.mission_id = mission_id.into();
        attempt
    }

    /// This aggregate's identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The mission this attempt targets, if it has already been established.
    pub fn mission_id(&self) -> &str {
        &self.mission_id
    }

    /// Current version (delegates to the embedded [`AggregateRoot`]).
    pub fn version(&self) -> u64 {
        self.root.version()
    }

    /// Events produced but not yet persisted.
    pub fn uncommitted_events(&self) -> &[Box<dyn DomainEvent>] {
        self.root.uncommitted_events()
    }

    /// Whether this attempt has started.
    pub fn has_started(&self) -> bool {
        self.started
    }

    /// Whether the first-clear reward has been claimed on this attempt.
    pub fn first_clear_reward_claimed(&self) -> bool {
        self.first_clear_reward_claimed
    }

    /// Set the mission this attempt targets.
    pub fn set_mission_id(&mut self, mission_id: impl Into<String>) {
        self.mission_id = mission_id.into();
    }

    /// Set the intro panels shown when the attempt starts.
    pub fn set_intro_panel_ids(&mut self, intro_panel_ids: Vec<String>) {
        self.intro_panel_ids = intro_panel_ids;
    }

    /// Model whether the fixed first-clear reward has already been claimed.
    pub fn set_first_clear_reward_claimed(&mut self, claimed: bool) {
        self.first_clear_reward_claimed = claimed;
    }

    /// Record whether the fixed $MADE reward is restricted to the player's first
    /// clear ever.
    pub fn set_fixed_made_reward_first_clear_only(&mut self, ok: bool) {
        self.fixed_made_reward_first_clear_only = ok;
    }

    /// Model whether a prologue mission's predecessor has been cleared.
    pub fn set_prologue_predecessor_cleared(&mut self, cleared: bool) {
        self.prologue_predecessor_cleared = cleared;
    }

    /// Model whether the mission is in today's Reprise rotation.
    pub fn set_reprise_rotation_eligible(&mut self, eligible: bool) {
        self.reprise_rotation_eligible_today = eligible;
    }

    /// Record whether this mission is in today's Reprise rotation for repeat
    /// rewards.
    pub fn set_reprise_rotation_eligible_today(&mut self, eligible: bool) {
        self.reprise_rotation_eligible_today = eligible;
    }

    /// Model whether special rules and boss HP-threshold barks fired exactly at
    /// their scripted points.
    pub fn set_scripted_points_satisfied(&mut self, satisfied: bool) {
        self.scripted_rules_and_barks_at_scripted_points = satisfied;
    }

    /// Record whether special rules and boss HP-threshold barks are wired to
    /// their scripted trigger points.
    pub fn set_scripted_rules_and_barks_at_scripted_points(&mut self, ok: bool) {
        self.scripted_rules_and_barks_at_scripted_points = ok;
    }

    /// First-clear invariant: the fixed $MADE reward is granted only once.
    fn ensure_first_clear_reward_available(&self) -> Result<(), DomainError> {
        if self.first_clear_reward_claimed {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' has already claimed the fixed $MADE first-clear reward; the \
                 fixed $MADE reward for a mission is granted only on the player's first clear, ever",
                self.id
            )));
        }
        Ok(())
    }

    /// First-clear reward invariant for mission starts.
    fn ensure_fixed_made_reward_first_clear_only(&self) -> Result<(), DomainError> {
        if self.first_clear_reward_claimed || !self.fixed_made_reward_first_clear_only {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' violates the fixed $MADE reward rule; the fixed $MADE reward \
                 for a mission is granted only on the player's first clear, ever",
                self.id
            )));
        }
        Ok(())
    }

    /// Prologue gate invariant: a Prologue mission unlocks only after its
    /// predecessor is cleared.
    fn ensure_prologue_predecessor_cleared(&self) -> Result<(), DomainError> {
        if !self.prologue_predecessor_cleared {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' is locked behind an uncleared predecessor; Prologue missions \
                 are gated in sequence, and a mission unlocks only after its predecessor is cleared",
                self.id
            )));
        }
        Ok(())
    }

    /// Reprise invariant: only missions in today's Reprise rotation are eligible
    /// for repeat rewards.
    fn ensure_reprise_rotation_eligible_today(&self) -> Result<(), DomainError> {
        if !self.reprise_rotation_eligible_today {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' is not in today's Reprise rotation; only missions in today's \
                 Reprise rotation are eligible for repeat rewards",
                self.id
            )));
        }
        Ok(())
    }

    /// Script invariant: special rules and boss HP-threshold barks fire exactly
    /// at their scripted points.
    fn ensure_scripted_rules_and_barks_at_scripted_points(&self) -> Result<(), DomainError> {
        if !self.scripted_rules_and_barks_at_scripted_points {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' did not satisfy scripted mission points; per-mission special \
                 rules and boss HP-threshold barks fire exactly at their scripted points",
                self.id
            )));
        }
        Ok(())
    }

    /// Intro panels must be present before the mission can begin.
    fn ensure_intro_panels_present(&self) -> Result<(), DomainError> {
        if self
            .intro_panel_ids
            .iter()
            .all(|panel_id| panel_id.trim().is_empty())
        {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' requires at least one intro panel before a mission can start",
                self.id
            )));
        }
        Ok(())
    }

    /// Apply an event to aggregate state.
    fn apply(&mut self, event: &Event) {
        match event {
            Event::MissionStarted(started) => {
                if self.mission_id.is_empty() {
                    self.mission_id = started.mission_id.clone();
                }
                self.started = true;
            }
            Event::FirstClearRewardClaimed(_) => {
                self.first_clear_reward_claimed = true;
            }
        }
    }

    /// Handle `StartMissionCmd`.
    fn start_mission(&mut self, cmd: StartMission) -> Result<Vec<Event>, DomainError> {
        if cmd.player_id.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' requires a valid playerId to start",
                self.id
            )));
        }
        if cmd.mission_id.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' requires a valid missionId to start",
                self.id
            )));
        }
        if !self.mission_id.is_empty() && cmd.mission_id != self.mission_id {
            return Err(DomainError::InvariantViolation(format!(
                "command targets mission '{}' but mission attempt '{}' targets mission '{}'",
                cmd.mission_id, self.id, self.mission_id
            )));
        }
        if self.started {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' for mission '{}' has already started",
                self.id, self.mission_id
            )));
        }

        self.ensure_fixed_made_reward_first_clear_only()?;
        self.ensure_prologue_predecessor_cleared()?;
        self.ensure_reprise_rotation_eligible_today()?;
        self.ensure_scripted_rules_and_barks_at_scripted_points()?;
        self.ensure_intro_panels_present()?;

        let event = Event::MissionStarted(MissionStarted {
            attempt_id: self.id.clone(),
            player_id: cmd.player_id,
            mission_id: cmd.mission_id,
            intro_panel_ids: self.intro_panel_ids.clone(),
        });
        self.apply(&event);
        self.root.record(Box::new(event.clone()));
        Ok(vec![event])
    }

    /// Handle `ClaimFirstClearRewardCmd`.
    fn claim_first_clear_reward(
        &mut self,
        cmd: ClaimFirstClearReward,
    ) -> Result<Vec<Event>, DomainError> {
        if cmd.player_id.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' requires a valid playerId to claim the first-clear reward",
                self.id
            )));
        }
        if cmd.mission_id.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "mission attempt '{}' requires a valid missionId to claim the first-clear reward",
                self.id
            )));
        }

        self.ensure_first_clear_reward_available()?;
        self.ensure_prologue_predecessor_cleared()?;
        self.ensure_reprise_rotation_eligible_today()?;
        self.ensure_scripted_rules_and_barks_at_scripted_points()?;

        let event = Event::FirstClearRewardClaimed(FirstClearRewardClaimed {
            player_id: cmd.player_id,
            mission_id: cmd.mission_id,
        });
        self.apply(&event);
        self.root.record(Box::new(event.clone()));
        Ok(vec![event])
    }
}

impl Aggregate for MissionAttempt {
    type Event = Event;

    fn aggregate_type() -> &'static str {
        AGGREGATE_TYPE
    }

    fn execute(&mut self, command: Command) -> Result<Vec<Self::Event>, DomainError> {
        match command.name.as_str() {
            START_MISSION => {
                let cmd: StartMission = serde_json::from_slice(&command.payload).map_err(|e| {
                    DomainError::InvariantViolation(format!(
                        "malformed StartMissionCmd payload: {e}"
                    ))
                })?;
                self.start_mission(cmd)
            }
            CLAIM_FIRST_CLEAR_REWARD => {
                let cmd: ClaimFirstClearReward =
                    serde_json::from_slice(&command.payload).map_err(|e| {
                        DomainError::InvariantViolation(format!(
                            "malformed ClaimFirstClearRewardCmd payload: {e}"
                        ))
                    })?;
                self.claim_first_clear_reward(cmd)
            }
            _ => Err(DomainError::unknown_command(
                <Self as Aggregate>::aggregate_type(),
                command.name,
            )),
        }
    }
}

/// Repository contract for the [`MissionAttempt`] aggregate. Adapters implement
/// [`shared::Repository`] for `MissionAttempt` and then this marker trait.
pub trait MissionAttemptRepository: Repository<MissionAttempt> {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_attempt() -> MissionAttempt {
        let mut attempt = MissionAttempt::for_mission("attempt-01", "mission-01");
        attempt.set_intro_panel_ids(vec!["intro-a".to_string(), "intro-b".to_string()]);
        attempt.set_first_clear_reward_claimed(false);
        attempt.set_fixed_made_reward_first_clear_only(true);
        attempt.set_prologue_predecessor_cleared(true);
        attempt.set_reprise_rotation_eligible_today(true);
        attempt.set_scripted_rules_and_barks_at_scripted_points(true);
        attempt
    }

    fn valid_start_cmd() -> StartMission {
        StartMission::new("player-01", "mission-01")
    }

    fn valid_claim_cmd() -> ClaimFirstClearReward {
        ClaimFirstClearReward::new("player-01", "mission-01")
    }

    // Scenario: Successfully execute StartMissionCmd.
    #[test]
    fn starts_mission_and_emits_mission_started_event() {
        let mut attempt = ready_attempt();

        let events = attempt
            .execute(valid_start_cmd().into_command())
            .expect("valid mission start should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type(), "mission.started");
        match &events[0] {
            Event::MissionStarted(started) => {
                assert_eq!(started.attempt_id, "attempt-01");
                assert_eq!(started.player_id, "player-01");
                assert_eq!(started.mission_id, "mission-01");
                assert_eq!(
                    started.intro_panel_ids,
                    vec!["intro-a".to_string(), "intro-b".to_string()]
                );
            }
            other => panic!("expected MissionStarted, got {other:?}"),
        }
        assert!(attempt.has_started());
        assert_eq!(attempt.version(), 1);
        assert_eq!(attempt.uncommitted_events().len(), 1);
        assert_eq!(
            attempt.uncommitted_events()[0].event_type(),
            "mission.started"
        );
    }

    // Scenario: Successfully execute ClaimFirstClearRewardCmd.
    #[test]
    fn claims_first_clear_reward_and_emits_event() {
        let mut attempt = ready_attempt();

        let events = attempt
            .execute(valid_claim_cmd().into_command())
            .expect("valid first-clear reward claim should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type(), "first.clear.reward.claimed");
        match &events[0] {
            Event::FirstClearRewardClaimed(claimed) => {
                assert_eq!(claimed.player_id, "player-01");
                assert_eq!(claimed.mission_id, "mission-01");
            }
            other => panic!("expected FirstClearRewardClaimed, got {other:?}"),
        }
        assert!(attempt.first_clear_reward_claimed());
        assert_eq!(attempt.version(), 1);
        assert_eq!(attempt.uncommitted_events().len(), 1);
        assert_eq!(
            attempt.uncommitted_events()[0].event_type(),
            "first.clear.reward.claimed"
        );
    }

    // Scenario: StartMissionCmd rejected - The fixed $MADE reward for a mission
    // is granted only on the player's first clear, ever.
    #[test]
    fn rejects_start_when_fixed_made_reward_first_clear_rule_is_violated() {
        let mut attempt = ready_attempt();
        attempt.set_fixed_made_reward_first_clear_only(false);

        let err = attempt
            .execute(valid_start_cmd().into_command())
            .expect_err("a duplicate fixed reward path must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert!(!attempt.has_started());
        assert_eq!(attempt.version(), 0);
    }

    // Scenario: ClaimFirstClearRewardCmd rejected - The fixed $MADE reward for a
    // mission is granted only on the player's first clear, ever.
    #[test]
    fn rejects_claim_when_first_clear_reward_was_already_claimed() {
        let mut attempt = ready_attempt();
        attempt.set_first_clear_reward_claimed(true);

        let err = attempt
            .execute(valid_claim_cmd().into_command())
            .expect_err("a repeated first-clear reward claim must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 0);
    }

    // Scenario: StartMissionCmd rejected - Prologue missions are gated in
    // sequence; a mission unlocks only after its predecessor is cleared.
    #[test]
    fn rejects_start_when_prologue_predecessor_has_not_been_cleared() {
        let mut attempt = ready_attempt();
        attempt.set_prologue_predecessor_cleared(false);

        let err = attempt
            .execute(valid_start_cmd().into_command())
            .expect_err("a locked Prologue mission must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert!(!attempt.has_started());
        assert_eq!(attempt.version(), 0);
    }

    // Scenario: StartMissionCmd rejected - Only missions in today's Reprise
    // rotation are eligible for repeat rewards.
    #[test]
    fn rejects_start_when_mission_is_not_in_todays_reprise_rotation() {
        let mut attempt = ready_attempt();
        attempt.set_reprise_rotation_eligible_today(false);

        let err = attempt
            .execute(valid_start_cmd().into_command())
            .expect_err("an out-of-rotation repeat reward mission must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert!(!attempt.has_started());
        assert_eq!(attempt.version(), 0);
    }

    // Scenario: StartMissionCmd rejected - Per-mission special rules and boss
    // HP-threshold barks fire exactly at their scripted points.
    #[test]
    fn rejects_start_when_scripted_rules_or_barks_are_off_script() {
        let mut attempt = ready_attempt();
        attempt.set_scripted_rules_and_barks_at_scripted_points(false);

        let err = attempt
            .execute(valid_start_cmd().into_command())
            .expect_err("off-script mission rules must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert!(!attempt.has_started());
        assert_eq!(attempt.version(), 0);
    }

    #[test]
    fn claim_reuses_mission_invariant_gates() {
        let mut attempt = ready_attempt();
        attempt.set_prologue_predecessor_cleared(false);

        let err = attempt
            .execute(valid_claim_cmd().into_command())
            .expect_err("an uncleared predecessor must block prologue reward claims");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 0);
    }

    #[test]
    fn rejects_start_with_missing_player_id() {
        let mut attempt = ready_attempt();

        let err = attempt
            .execute(StartMission::new("   ", "mission-01").into_command())
            .expect_err("a command with a missing playerId must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 0);
    }

    #[test]
    fn rejects_start_with_missing_mission_id() {
        let mut attempt = ready_attempt();

        let err = attempt
            .execute(StartMission::new("player-01", "   ").into_command())
            .expect_err("a command with a missing missionId must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 0);
    }

    #[test]
    fn rejects_claim_with_missing_player_id() {
        let mut attempt = ready_attempt();

        let err = attempt
            .execute(ClaimFirstClearReward::new("   ", "mission-01").into_command())
            .expect_err("missing playerId must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 0);
    }

    #[test]
    fn rejects_claim_with_missing_mission_id() {
        let mut attempt = ready_attempt();

        let err = attempt
            .execute(ClaimFirstClearReward::new("player-01", "   ").into_command())
            .expect_err("missing missionId must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 0);
    }

    #[test]
    fn rejects_start_for_a_different_mission() {
        let mut attempt = ready_attempt();

        let err = attempt
            .execute(StartMission::new("player-01", "mission-99").into_command())
            .expect_err("a command for another mission must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 0);
    }

    #[test]
    fn start_sets_mission_for_legacy_attempts_without_a_target() {
        let mut attempt = MissionAttempt::new("attempt-01");

        attempt
            .execute(valid_start_cmd().into_command())
            .expect("unbound mission attempt should accept the first valid mission");

        assert_eq!(attempt.mission_id(), "mission-01");
        assert!(attempt.has_started());
    }

    #[test]
    fn rejects_start_when_intro_panels_are_missing() {
        let mut attempt = ready_attempt();
        attempt.set_intro_panel_ids(Vec::new());

        let err = attempt
            .execute(valid_start_cmd().into_command())
            .expect_err("a start without intro panels must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 0);
    }

    #[test]
    fn rejects_a_repeated_start_of_the_same_attempt() {
        let mut attempt = ready_attempt();

        attempt
            .execute(valid_start_cmd().into_command())
            .expect("first start should succeed");

        let err = attempt
            .execute(valid_start_cmd().into_command())
            .expect_err("a repeated start must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 1);
        assert_eq!(attempt.uncommitted_events().len(), 1);
    }

    #[test]
    fn rejects_repeat_claim_after_successful_claim() {
        let mut attempt = ready_attempt();

        attempt
            .execute(valid_claim_cmd().into_command())
            .expect("first claim should succeed");
        let err = attempt
            .execute(valid_claim_cmd().into_command())
            .expect_err("second claim should be rejected");

        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(attempt.version(), 1);
    }

    #[test]
    fn rejects_unknown_command() {
        let mut attempt = MissionAttempt::new("attempt-01");
        let err = attempt.execute(Command::new("NoSuchCommand")).unwrap_err();
        match err {
            DomainError::UnknownCommand { aggregate, command } => {
                assert_eq!(aggregate, "MissionAttempt");
                assert_eq!(command, "NoSuchCommand");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn start_command_payload_round_trips() {
        let cmd = valid_start_cmd();
        let command = cmd.into_command();
        assert_eq!(command.name, StartMission::COMMAND);
        let decoded: StartMission = serde_json::from_slice(&command.payload).unwrap();
        assert_eq!(decoded, valid_start_cmd());
    }

    #[test]
    fn claim_command_payload_round_trips() {
        let cmd = valid_claim_cmd();
        let command = cmd.into_command();
        assert_eq!(command.name, ClaimFirstClearReward::COMMAND);
        let decoded: ClaimFirstClearReward = serde_json::from_slice(&command.payload).unwrap();
        assert_eq!(decoded, valid_claim_cmd());
    }
}
