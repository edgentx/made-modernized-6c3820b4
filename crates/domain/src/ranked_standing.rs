//! RankedStanding bounded context — a player's competitive rank/rating over a season.
//!
//! A [`RankedStanding`] tracks one player's standing in ranked play: their
//! visible rank (a tier plus stars within that tier), the hidden Glicko-2
//! rating that actually drives matchmaking, and the anti-abuse state
//! (rank-floor protection, smurf elevation, escalating disconnect penalties)
//! that keeps ranked healthy. Five invariants govern a standing:
//!
//! 1. **Glicko-2 freshness** — a player's Glicko-2 rating, RD, and volatility are
//!    recalculated after *every* rated match, so a standing whose ratings are
//!    stale (not yet recomputed for the match being scored) cannot be advanced.
//! 2. **Rank progression** — the visible rank advances through tiers
//!    Block→Legend with [`STARS_PER_TIER`] stars per tier; a win streak grants at
//!    most one bonus star ([`MAX_STREAK_BONUS`]). A tier that is already full, or
//!    a bonus larger than one star, is inconsistent.
//! 3. **Rank-floor protection** — anti-tilt protection prevents demotion below a
//!    reached tier floor (it applies to the low Block/Corner tiers), so a
//!    standing sitting below its own floor is invalid.
//! 4. **Smurf elevation** — a suspected smurf is auto-elevated to a higher
//!    bracket after [`SMURF_ELEVATION_MATCHES`] matches, so a suspected smurf
//!    past that threshold that has *not* been elevated is invalid.
//! 5. **Disconnect penalties** — disconnect penalties escalate by doubling on
//!    repeated abandonment; an unsettled escalating penalty must be cleared
//!    before a star can be awarded.
//!
//! One command is implemented. [`AwardStar`] (`AwardStarCmd`) grants a star (plus
//! any streak bonus) to a standing on a win, enforcing every invariant; on
//! success it always emits [`Event::StarAwarded`] (`star.awarded`) and, when the
//! awarded star completes the current tier, also emits [`Event::RankPromoted`]
//! (`rank.promoted`). This module is hand-written (it no longer uses
//! `shared::stub_aggregate!`) but preserves the same public surface — a
//! [`RankedStanding`] aggregate and a [`RankedStandingRepository`] port — so the
//! persistence adapters in `crates/mocks` keep compiling unchanged.

use serde::{Deserialize, Serialize};

use shared::{Aggregate, AggregateRoot, Command, DomainError, DomainEvent, Repository};

/// Stable aggregate type name, surfaced in [`DomainError::UnknownCommand`] and
/// used for command routing.
const AGGREGATE_TYPE: &str = "RankedStanding";

/// The command name [`RankedStanding::execute`] recognizes for awarding a star.
const AWARD_STAR: &str = "AwardStarCmd";

/// Stars required to fill a tier before the visible rank promotes to the next
/// tier: the rank advances through tiers Block→Legend with 3 stars per tier.
pub const STARS_PER_TIER: u32 = 3;

/// The most stars a single win streak may grant on top of the base star: a win
/// streak grants at most one bonus star.
pub const MAX_STREAK_BONUS: u32 = 1;

/// Number of matches after which a suspected smurf is auto-elevated to a higher
/// bracket.
pub const SMURF_ELEVATION_MATCHES: u32 = 20;

/// The visible rank tiers, ordered from the entry tier ([`Tier::Block`]) to the
/// apex ([`Tier::Legend`]). The rank advances one tier at a time as stars fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tier {
    /// Entry tier; rank-floor protection (anti-tilt) applies here.
    Block,
    /// Second tier; rank-floor protection (anti-tilt) applies here.
    Corner,
    /// Third tier.
    Edge,
    /// Fourth tier.
    Core,
    /// Fifth tier.
    Apex,
    /// The top of the ladder.
    Legend,
}

impl Tier {
    /// The tiers in ascending order; the index of a tier in this slice is its
    /// ordinal, used for the ordered floor comparison.
    const LADDER: [Tier; 6] = [
        Tier::Block,
        Tier::Corner,
        Tier::Edge,
        Tier::Core,
        Tier::Apex,
        Tier::Legend,
    ];

    /// This tier's position on the ladder, `0` for [`Tier::Block`].
    pub fn ordinal(self) -> usize {
        Self::LADDER.iter().position(|&t| t == self).unwrap_or(0)
    }

    /// The next tier up the ladder, or `None` if already at [`Tier::Legend`].
    pub fn next(self) -> Option<Tier> {
        Self::LADDER.get(self.ordinal() + 1).copied()
    }

    /// Whether this is the apex tier ([`Tier::Legend`]).
    pub fn is_apex(self) -> bool {
        self == Tier::Legend
    }
}

/// The `AwardStarCmd` payload: grant a star (plus a `streak_bonus`) to the named
/// player's standing on a win. Field names are the ranked service's `camelCase`
/// schema.
///
/// Build one directly and turn it into a [`Command`] with
/// [`AwardStar::into_command`], or decode it from a command payload via
/// [`serde_json`] inside [`RankedStanding::execute`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwardStar {
    /// Identity of the standing being advanced; must name the standing this
    /// aggregate records.
    pub standing_id: String,
    /// The player whose standing is advanced; must match this standing's player.
    pub player_id: String,
    /// Bonus stars granted by the current win streak, on top of the base star.
    /// Must not exceed [`MAX_STREAK_BONUS`].
    pub streak_bonus: u32,
}

impl AwardStar {
    /// The command name this maps to.
    pub const COMMAND: &'static str = AWARD_STAR;

    /// Build a command awarding a star (plus `streak_bonus`) to `player_id`'s
    /// standing `standing_id`.
    pub fn new(
        standing_id: impl Into<String>,
        player_id: impl Into<String>,
        streak_bonus: u32,
    ) -> Self {
        Self {
            standing_id: standing_id.into(),
            player_id: player_id.into(),
            streak_bonus,
        }
    }

    /// Encode this command as a [`shared::Command`] carrying a JSON payload,
    /// ready to hand to [`RankedStanding::execute`].
    pub fn into_command(&self) -> Command {
        // Serialization of a plain data struct to a Vec cannot fail here.
        let payload = serde_json::to_vec(self).expect("AwardStar is always serializable");
        Command::with_payload(Self::COMMAND, payload)
    }
}

/// A star was awarded to a standing, carried by [`Event::StarAwarded`] and thus
/// by the emitted `star.awarded` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StarAwarded {
    /// The standing that received the star.
    pub standing_id: String,
    /// The player whose standing was advanced.
    pub player_id: String,
    /// Total stars granted by this award: the base star plus any streak bonus.
    pub stars_awarded: u32,
    /// The tier the standing was in when the star was awarded.
    pub tier: Tier,
}

/// A standing's visible rank advanced to the next tier, carried by
/// [`Event::RankPromoted`] and thus by the emitted `rank.promoted` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankPromoted {
    /// The standing that was promoted.
    pub standing_id: String,
    /// The tier the standing advanced from.
    pub from_tier: Tier,
    /// The tier the standing advanced to.
    pub to_tier: Tier,
}

/// Domain events emitted by [`RankedStanding`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A star (plus any streak bonus) was awarded to a standing.
    StarAwarded(StarAwarded),
    /// A standing's visible rank advanced to the next tier.
    RankPromoted(RankPromoted),
}

impl DomainEvent for Event {
    fn event_type(&self) -> &'static str {
        match self {
            Event::StarAwarded(_) => "star.awarded",
            Event::RankPromoted(_) => "rank.promoted",
        }
    }
}

/// The RankedStanding aggregate: one player's competitive standing over a season.
///
/// Mirrors the shape produced by [`shared::stub_aggregate!`] (identity plus an
/// embedded [`AggregateRoot`]) so the surrounding wiring — the in-memory
/// repository adapters, the server — is unchanged, while it now carries the
/// standing's ranked state: the player, the visible rank (tier + stars), the
/// reached tier floor, the Glicko-2 freshness flag, and the anti-abuse state
/// (matches played, smurf suspicion/elevation, outstanding disconnect penalty).
/// Its `execute` handles [`AwardStarCmd`].
///
/// A fresh standing from [`RankedStanding::new`] is intentionally *not
/// award-ready* (its Glicko-2 ratings have not been recomputed for a match yet);
/// the configuration methods below drive it to the state a command validates,
/// exactly as [`MatchmakingTicket`](crate::matchmaking_ticket) is built up before
/// a command validates it.
#[derive(Debug)]
pub struct RankedStanding {
    id: String,
    root: AggregateRoot,
    /// The player this standing belongs to. An award command must name them.
    player_id: String,
    /// Current visible tier.
    tier: Tier,
    /// Stars accumulated within the current tier; always less than
    /// [`STARS_PER_TIER`] for a consistent standing.
    stars_in_tier: u32,
    /// Highest tier the standing has ever reached — the anti-tilt floor. The
    /// current tier may never sit below it.
    floor_tier: Tier,
    /// Whether the Glicko-2 rating/RD/volatility have been recalculated for the
    /// match currently being scored. Awards require fresh ratings.
    ratings_recalculated: bool,
    /// Rated matches this standing has played, used for smurf elevation.
    matches_played: u32,
    /// Whether the player is a suspected smurf.
    suspected_smurf: bool,
    /// Whether the player has been auto-elevated to a higher bracket.
    elevated: bool,
    /// Outstanding, escalating disconnect penalty (doubles on repeated
    /// abandonment). Must be settled to `0` before a star can be awarded.
    outstanding_disconnect_penalty: u32,
}

impl RankedStanding {
    /// Create a new standing identified by `id`, at the entry tier with no stars
    /// and stale ratings (not yet award-ready). Use the configuration methods to
    /// drive it to the state a command validates.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root: AggregateRoot::new(),
            player_id: String::new(),
            tier: Tier::Block,
            stars_in_tier: 0,
            floor_tier: Tier::Block,
            ratings_recalculated: false,
            matches_played: 0,
            suspected_smurf: false,
            elevated: false,
            outstanding_disconnect_penalty: 0,
        }
    }

    /// This aggregate's identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The player this standing belongs to.
    pub fn player_id(&self) -> &str {
        &self.player_id
    }

    /// Current visible tier.
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Stars accumulated within the current tier.
    pub fn stars_in_tier(&self) -> u32 {
        self.stars_in_tier
    }

    /// Current version (delegates to the embedded [`AggregateRoot`]).
    pub fn version(&self) -> u64 {
        self.root.version()
    }

    /// Events produced but not yet persisted.
    pub fn uncommitted_events(&self) -> &[Box<dyn DomainEvent>] {
        self.root.uncommitted_events()
    }

    /// Set the player this standing belongs to.
    pub fn set_player(&mut self, player_id: impl Into<String>) {
        self.player_id = player_id.into();
    }

    /// Place the standing at a tier with a given number of stars. Also raises the
    /// tier floor to this tier when it is higher, keeping the floor consistent.
    pub fn set_rank(&mut self, tier: Tier, stars_in_tier: u32) {
        self.tier = tier;
        self.stars_in_tier = stars_in_tier;
        if tier > self.floor_tier {
            self.floor_tier = tier;
        }
    }

    /// Set the anti-tilt floor tier directly. Used to drive the rank-floor
    /// invariant (e.g. a floor above the current tier is an invalid demotion).
    pub fn set_floor_tier(&mut self, floor_tier: Tier) {
        self.floor_tier = floor_tier;
    }

    /// Mark whether the Glicko-2 ratings have been recalculated for the match
    /// currently being scored.
    pub fn set_ratings_recalculated(&mut self, recalculated: bool) {
        self.ratings_recalculated = recalculated;
    }

    /// Record how many rated matches the standing has played.
    pub fn set_matches_played(&mut self, matches_played: u32) {
        self.matches_played = matches_played;
    }

    /// Flag whether the player is a suspected smurf, and whether they have been
    /// elevated to a higher bracket.
    pub fn set_smurf_status(&mut self, suspected_smurf: bool, elevated: bool) {
        self.suspected_smurf = suspected_smurf;
        self.elevated = elevated;
    }

    /// Set the outstanding, escalating disconnect penalty (doubles on repeated
    /// abandonment). A non-zero penalty blocks a star award.
    pub fn set_outstanding_disconnect_penalty(&mut self, penalty: u32) {
        self.outstanding_disconnect_penalty = penalty;
    }

    /// Glicko-2 invariant: rating, RD, and volatility are recalculated after
    /// every rated match, so a standing whose ratings are stale for the match
    /// being scored cannot be advanced.
    fn ensure_ratings_recalculated(&self) -> Result<(), DomainError> {
        if !self.ratings_recalculated {
            return Err(DomainError::InvariantViolation(format!(
                "standing '{}' has stale Glicko-2 ratings; rating, RD, and volatility must be \
                 recalculated after every rated match before a star can be awarded",
                self.id
            )));
        }
        Ok(())
    }

    /// Rank-progression invariant: the visible rank advances through tiers
    /// Block→Legend with [`STARS_PER_TIER`] stars per tier and a win streak grants
    /// at most one bonus star. A tier already full of stars, or a streak bonus
    /// larger than one star, is inconsistent.
    fn ensure_rank_progress_consistent(&self, streak_bonus: u32) -> Result<(), DomainError> {
        if streak_bonus > MAX_STREAK_BONUS {
            return Err(DomainError::InvariantViolation(format!(
                "standing '{}' streak bonus {} exceeds the maximum of {} bonus star; a win streak \
                 grants at most one bonus star",
                self.id, streak_bonus, MAX_STREAK_BONUS
            )));
        }
        if self.stars_in_tier >= STARS_PER_TIER {
            return Err(DomainError::InvariantViolation(format!(
                "standing '{}' already holds {} of {} stars in {:?}; the tier should have already \
                 promoted, so its rank progression is inconsistent",
                self.id, self.stars_in_tier, STARS_PER_TIER, self.tier
            )));
        }
        Ok(())
    }

    /// Rank-floor invariant: anti-tilt protection prevents demotion below a
    /// reached tier floor, so the current tier may never sit below the floor.
    fn ensure_above_rank_floor(&self) -> Result<(), DomainError> {
        if self.tier < self.floor_tier {
            return Err(DomainError::InvariantViolation(format!(
                "standing '{}' is at {:?} below its reached floor {:?}; rank-floor protection \
                 prevents demotion below a reached tier floor",
                self.id, self.tier, self.floor_tier
            )));
        }
        Ok(())
    }

    /// Smurf-elevation invariant: a suspected smurf is auto-elevated to a higher
    /// bracket after [`SMURF_ELEVATION_MATCHES`] matches, so a suspected smurf
    /// past that threshold that has not been elevated is invalid.
    fn ensure_smurf_elevated(&self) -> Result<(), DomainError> {
        if self.suspected_smurf && self.matches_played >= SMURF_ELEVATION_MATCHES && !self.elevated
        {
            return Err(DomainError::InvariantViolation(format!(
                "standing '{}' is a suspected smurf with {} matches but was not auto-elevated; a \
                 suspected smurf must be elevated to a higher bracket after {} matches",
                self.id, self.matches_played, SMURF_ELEVATION_MATCHES
            )));
        }
        Ok(())
    }

    /// Disconnect-penalty invariant: disconnect penalties escalate by doubling on
    /// repeated abandonment, and an unsettled escalating penalty must be cleared
    /// before a star can be awarded.
    fn ensure_no_outstanding_penalty(&self) -> Result<(), DomainError> {
        if self.outstanding_disconnect_penalty > 0 {
            return Err(DomainError::InvariantViolation(format!(
                "standing '{}' has an outstanding escalating disconnect penalty of {}; penalties \
                 double on repeated abandonment and must be settled before a star is awarded",
                self.id, self.outstanding_disconnect_penalty
            )));
        }
        Ok(())
    }

    /// Handle `AwardStarCmd`: verify the command targets this standing and player,
    /// enforce every invariant (fresh Glicko-2 ratings, consistent rank
    /// progression, above the rank floor, smurf elevation applied, no outstanding
    /// disconnect penalty), award the star (plus streak bonus), and emit
    /// [`Event::StarAwarded`] — plus [`Event::RankPromoted`] when the award
    /// completes the current tier.
    fn award_star(&mut self, cmd: AwardStar) -> Result<Vec<Event>, DomainError> {
        // The command must name the standing this aggregate actually records.
        if cmd.standing_id != self.id {
            return Err(DomainError::InvariantViolation(format!(
                "command targets standing '{}' but this aggregate records '{}'",
                cmd.standing_id, self.id
            )));
        }
        // ...and the player it belongs to.
        if cmd.player_id != self.player_id {
            return Err(DomainError::InvariantViolation(format!(
                "command names player '{}' but standing '{}' belongs to '{}'",
                cmd.player_id, self.id, self.player_id
            )));
        }

        // Enforce every invariant before awarding anything.
        self.ensure_ratings_recalculated()?;
        self.ensure_rank_progress_consistent(cmd.streak_bonus)?;
        self.ensure_above_rank_floor()?;
        self.ensure_smurf_elevated()?;
        self.ensure_no_outstanding_penalty()?;

        let stars_awarded = 1 + cmd.streak_bonus;
        let from_tier = self.tier;
        let total = self.stars_in_tier + stars_awarded;

        let mut events = vec![Event::StarAwarded(StarAwarded {
            standing_id: cmd.standing_id.clone(),
            player_id: cmd.player_id,
            stars_awarded,
            tier: from_tier,
        })];

        // A completed tier promotes the visible rank to the next tier, carrying
        // any overflow stars forward and raising the anti-tilt floor. Legend is
        // the apex, so a standing already there simply banks the extra stars.
        if total >= STARS_PER_TIER {
            if let Some(next_tier) = from_tier.next() {
                self.tier = next_tier;
                self.stars_in_tier = total - STARS_PER_TIER;
                self.floor_tier = next_tier;
                events.push(Event::RankPromoted(RankPromoted {
                    standing_id: cmd.standing_id,
                    from_tier,
                    to_tier: next_tier,
                }));
            } else {
                // Already at the apex tier: keep the accumulated stars.
                self.stars_in_tier = total;
            }
        } else {
            self.stars_in_tier = total;
        }

        for event in &events {
            self.root.record(Box::new(event.clone()));
        }
        Ok(events)
    }
}

impl Aggregate for RankedStanding {
    type Event = Event;

    fn aggregate_type() -> &'static str {
        AGGREGATE_TYPE
    }

    fn execute(&mut self, command: Command) -> Result<Vec<Self::Event>, DomainError> {
        match command.name.as_str() {
            AWARD_STAR => {
                let cmd: AwardStar = serde_json::from_slice(&command.payload).map_err(|e| {
                    DomainError::InvariantViolation(format!("malformed AwardStarCmd payload: {e}"))
                })?;
                self.award_star(cmd)
            }
            // Any other command is unknown to this aggregate.
            _ => Err(DomainError::unknown_command(
                <Self as Aggregate>::aggregate_type(),
                command.name,
            )),
        }
    }
}

/// Repository contract for the [`RankedStanding`] aggregate. Adapters implement
/// [`shared::Repository`] for `RankedStanding` and then this marker trait.
pub trait RankedStandingRepository: Repository<RankedStanding> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// An award-ready standing `r-01` for player `p-self`: fresh Glicko-2
    /// ratings, sitting at `Edge` with two stars and its floor at `Edge`, no
    /// smurf suspicion, no outstanding penalty. Awarding one base star fills the
    /// tier (2 + 1 = 3) and thus promotes. Tests mutate one aspect at a time to
    /// drive a specific rejection.
    fn ready_standing() -> RankedStanding {
        let mut standing = RankedStanding::new("r-01");
        standing.set_player("p-self");
        standing.set_rank(Tier::Edge, STARS_PER_TIER - 1);
        standing.set_ratings_recalculated(true);
        standing.set_matches_played(10);
        standing.set_smurf_status(false, false);
        standing.set_outstanding_disconnect_penalty(0);
        standing
    }

    /// A command awarding a star (no streak bonus) to `r-01`'s player `p-self`.
    fn valid_cmd() -> AwardStar {
        AwardStar::new("r-01", "p-self", 0)
    }

    // Scenario: Successfully execute AwardStarCmd — emits star.awarded AND
    // rank.promoted.
    #[test]
    fn awards_star_and_promotes_emitting_both_events() {
        let mut standing = ready_standing();

        let events = standing
            .execute(valid_cmd().into_command())
            .expect("valid award should succeed");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type(), "star.awarded");
        assert_eq!(events[1].event_type(), "rank.promoted");
        match &events[0] {
            Event::StarAwarded(awarded) => {
                assert_eq!(awarded.standing_id, "r-01");
                assert_eq!(awarded.player_id, "p-self");
                assert_eq!(awarded.stars_awarded, 1);
                assert_eq!(awarded.tier, Tier::Edge);
            }
            other => panic!("expected StarAwarded, got {other:?}"),
        }
        match &events[1] {
            Event::RankPromoted(promoted) => {
                assert_eq!(promoted.standing_id, "r-01");
                assert_eq!(promoted.from_tier, Tier::Edge);
                assert_eq!(promoted.to_tier, Tier::Core);
            }
            other => panic!("expected RankPromoted, got {other:?}"),
        }
        // The standing advanced a tier and recorded both events.
        assert_eq!(standing.tier(), Tier::Core);
        assert_eq!(standing.stars_in_tier(), 0);
        assert_eq!(standing.version(), 2);
        assert_eq!(standing.uncommitted_events().len(), 2);
    }

    // A star award that does not fill the tier emits only star.awarded.
    #[test]
    fn awards_star_without_promotion_emits_single_event() {
        let mut standing = ready_standing();
        // Start with no stars so a single star does not complete the tier.
        standing.set_rank(Tier::Edge, 0);

        let events = standing
            .execute(valid_cmd().into_command())
            .expect("valid award should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type(), "star.awarded");
        assert_eq!(standing.tier(), Tier::Edge);
        assert_eq!(standing.stars_in_tier(), 1);
        assert_eq!(standing.version(), 1);
    }

    // Scenario: rejected — Glicko-2 rating, RD, and volatility are recalculated
    // after every rated match.
    #[test]
    fn rejects_when_glicko_ratings_are_stale() {
        let mut standing = ready_standing();
        standing.set_ratings_recalculated(false);

        let err = standing
            .execute(valid_cmd().into_command())
            .expect_err("stale Glicko-2 ratings must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(standing.version(), 0);
    }

    // Scenario: rejected — visible rank advances through tiers Block→Legend with
    // 3 stars per tier; a win streak grants a bonus star.
    #[test]
    fn rejects_when_rank_progression_is_inconsistent() {
        let mut standing = ready_standing();
        // A streak bonus larger than one star breaks the "at most one bonus
        // star" rule.
        let cmd = AwardStar::new("r-01", "p-self", MAX_STREAK_BONUS + 1);

        let err = standing
            .execute(cmd.into_command())
            .expect_err("an over-large streak bonus must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(standing.version(), 0);
    }

    #[test]
    fn rejects_when_tier_is_already_full_of_stars() {
        let mut standing = ready_standing();
        // A tier holding a full set of stars should have already promoted.
        standing.set_rank(Tier::Edge, STARS_PER_TIER);

        let err = standing
            .execute(valid_cmd().into_command())
            .expect_err("an already-full tier must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(standing.version(), 0);
    }

    // Scenario: rejected — rank-floor protection prevents demotion below a
    // reached tier floor (anti-tilt applies to Block/Corner).
    #[test]
    fn rejects_when_below_reached_rank_floor() {
        let mut standing = ready_standing();
        // The standing sits at Block but has reached the Corner floor: an
        // invalid demotion below the anti-tilt floor.
        standing.set_rank(Tier::Block, 0);
        standing.set_floor_tier(Tier::Corner);

        let err = standing
            .execute(valid_cmd().into_command())
            .expect_err("a standing below its reached floor must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(standing.version(), 0);
    }

    // Scenario: rejected — a suspected smurf is auto-elevated to a higher bracket
    // after 20 matches.
    #[test]
    fn rejects_when_suspected_smurf_not_elevated() {
        let mut standing = ready_standing();
        standing.set_matches_played(SMURF_ELEVATION_MATCHES);
        // Suspected smurf past the threshold but not elevated.
        standing.set_smurf_status(true, false);

        let err = standing
            .execute(valid_cmd().into_command())
            .expect_err("an un-elevated suspected smurf must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(standing.version(), 0);
    }

    // Scenario: rejected — disconnect penalties escalate (doubling) on repeated
    // abandonment.
    #[test]
    fn rejects_when_disconnect_penalty_outstanding() {
        let mut standing = ready_standing();
        // An unsettled, escalated disconnect penalty blocks the award.
        standing.set_outstanding_disconnect_penalty(4);

        let err = standing
            .execute(valid_cmd().into_command())
            .expect_err("an outstanding disconnect penalty must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(standing.version(), 0);
    }

    // A command naming a different standing is rejected before any invariant runs.
    #[test]
    fn rejects_command_for_a_different_standing() {
        let mut standing = ready_standing();
        let cmd = AwardStar::new("r-99", "p-self", 0);

        let err = standing
            .execute(cmd.into_command())
            .expect_err("a command for another standing must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(standing.version(), 0);
    }

    // A command naming a different player is likewise rejected.
    #[test]
    fn rejects_command_for_a_different_player() {
        let mut standing = ready_standing();
        let cmd = AwardStar::new("r-01", "p-other", 0);

        let err = standing
            .execute(cmd.into_command())
            .expect_err("a command for another player must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(standing.version(), 0);
    }

    // An unrecognized command is still an UnknownCommand for this aggregate,
    // preserving the contract the mock adapters rely on.
    #[test]
    fn rejects_unknown_command() {
        let mut standing = RankedStanding::new("r-01");
        let err = standing.execute(Command::new("NoSuchCommand")).unwrap_err();
        match err {
            DomainError::UnknownCommand { aggregate, command } => {
                assert_eq!(aggregate, "RankedStanding");
                assert_eq!(command, "NoSuchCommand");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn command_payload_round_trips() {
        let cmd = valid_cmd();
        let command = cmd.into_command();
        assert_eq!(command.name, AwardStar::COMMAND);
        let decoded: AwardStar = serde_json::from_slice(&command.payload).unwrap();
        assert_eq!(decoded, valid_cmd());
    }
}
