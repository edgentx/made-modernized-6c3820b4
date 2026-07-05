//! PlayerCollection bounded context — a single player's owned cards and the
//! cosmetic skins they have equipped onto base cards (collection & deckbuilding).
//!
//! A [`PlayerCollection`] tracks, for one player, the cards they own and the
//! server-resolved cosmetic equips layered on top of those cards. Four
//! invariants govern whether a cosmetic may be equipped onto — or, here,
//! unequipped from — a base card:
//!
//! 1. **Server-resolved equips** — cosmetic equips are resolved server-side and
//!    never trusted from the client; an equip whose provenance is a client
//!    assertion rather than a server resolution is inadmissible.
//! 2. **Non-negative quantities** — owned card quantities are always
//!    non-negative; a collection recording a negative owned quantity is corrupt.
//! 3. **Card present for Outfit inclusion** — a card may only be included in an
//!    Outfit if it is present (qty ≥ 1) in the collection; a base card that is
//!    not present cannot carry a cosmetic bound for Outfit play.
//! 4. **Cosmetic targets an owned base card** — a cosmetic skin may only be
//!    equipped onto a base card the player actually owns; a cosmetic bound to an
//!    unowned base card is inconsistent.
//!
//! One command is implemented. [`UnequipCosmetic`] (`UnequipCosmeticCmd`)
//! removes the cosmetic currently equipped on a base card, enforcing every
//! invariant, and on success emits [`Event::CosmeticUnequipped`]
//! (`cosmetic.unequipped`). This module is hand-written (it does not use
//! `shared::stub_aggregate!`) but preserves the same public surface — a
//! [`PlayerCollection`] aggregate and a [`PlayerCollectionRepository`] port — so
//! the persistence adapters in `crates/mocks` follow the same convention.

use serde::{Deserialize, Serialize};

use shared::{Aggregate, AggregateRoot, Command, DomainError, DomainEvent, Repository};

/// Stable aggregate type name, surfaced in [`DomainError::UnknownCommand`] and
/// used for command routing.
const AGGREGATE_TYPE: &str = "PlayerCollection";

/// The command name [`PlayerCollection::execute`] recognizes to remove a
/// cosmetic from a base card.
const UNEQUIP_COSMETIC: &str = "UnequipCosmeticCmd";

/// The `UnequipCosmeticCmd` payload: the player and base card whose cosmetic is
/// being removed. Field names are the collection service's `camelCase` schema.
///
/// Build one directly and turn it into a [`Command`] with
/// [`UnequipCosmetic::into_command`], or decode it from a command payload via
/// [`serde_json`] inside [`PlayerCollection::execute`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnequipCosmetic {
    /// Identity of the player whose collection is being modified; must name the
    /// player this aggregate records, and must be non-empty.
    pub player_id: String,
    /// Identity of the base card the cosmetic is being removed from; must be
    /// non-empty.
    pub base_card_id: String,
}

impl UnequipCosmetic {
    /// The command name this maps to.
    pub const COMMAND: &'static str = UNEQUIP_COSMETIC;

    /// Build a command unequipping `base_card_id`'s cosmetic for `player_id`.
    pub fn new(player_id: impl Into<String>, base_card_id: impl Into<String>) -> Self {
        Self {
            player_id: player_id.into(),
            base_card_id: base_card_id.into(),
        }
    }

    /// Encode this command as a [`shared::Command`] carrying a JSON payload,
    /// ready to hand to [`PlayerCollection::execute`].
    pub fn into_command(&self) -> Command {
        // Serialization of a plain data struct to a Vec cannot fail here.
        let payload = serde_json::to_vec(self).expect("UnequipCosmetic is always serializable");
        Command::with_payload(Self::COMMAND, payload)
    }
}

/// The record of a cosmetic being removed from a base card, carried by
/// [`Event::CosmeticUnequipped`] and thus by the emitted `cosmetic.unequipped`
/// event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CosmeticUnequipped {
    /// The player whose collection was modified.
    pub player_id: String,
    /// The base card the cosmetic was removed from.
    pub base_card_id: String,
}

/// Domain events emitted by [`PlayerCollection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A cosmetic was removed from a base card the player owns.
    CosmeticUnequipped(CosmeticUnequipped),
}

impl DomainEvent for Event {
    fn event_type(&self) -> &'static str {
        match self {
            Event::CosmeticUnequipped(_) => "cosmetic.unequipped",
        }
    }
}

/// The PlayerCollection aggregate: one player's owned cards and cosmetic equips.
///
/// Mirrors the shape produced by [`shared::stub_aggregate!`] (identity plus an
/// embedded [`AggregateRoot`]) so the surrounding wiring — the in-memory
/// repository adapters, the server — follows the same convention, while it
/// carries the collection's state: the owned quantity of the base card, whether
/// the player owns that base card, whether a cosmetic is currently equipped on
/// it, and whether the equip was resolved server-side. Its `execute` handles
/// [`UnequipCosmeticCmd`].
///
/// A fresh collection from [`PlayerCollection::new`] owns one copy of the base
/// card, has a server-resolved cosmetic equipped, and is therefore unequip-ready.
/// The configuration methods below drive it to a state a command rejects,
/// exactly as [`Season`](crate::season) is built up before a command validates
/// it.
#[derive(Debug)]
pub struct PlayerCollection {
    id: String,
    root: AggregateRoot,
    /// Owned quantity of the base card. Must be non-negative (invariant 2) and
    /// at least 1 for the card to be present for Outfit inclusion (invariant 3).
    base_card_quantity: i64,
    /// Whether the player actually owns the base card the cosmetic targets
    /// (invariant 4).
    owns_base_card: bool,
    /// Whether a cosmetic is currently equipped on the base card; cleared on a
    /// successful unequip.
    cosmetic_equipped: bool,
    /// Whether the cosmetic equip was resolved server-side rather than trusted
    /// from the client (invariant 1).
    server_resolved: bool,
}

impl PlayerCollection {
    /// Create a new collection identified by `id` (the owning player): owns one
    /// copy of the base card, has a server-resolved cosmetic equipped, and is
    /// unequip-ready. Use the configuration methods to drive it to the state a
    /// command validates.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root: AggregateRoot::new(),
            base_card_quantity: 1,
            owns_base_card: true,
            cosmetic_equipped: true,
            server_resolved: true,
        }
    }

    /// This aggregate's identity (the owning player).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Owned quantity of the base card.
    pub fn base_card_quantity(&self) -> i64 {
        self.base_card_quantity
    }

    /// Whether a cosmetic is currently equipped on the base card.
    pub fn cosmetic_equipped(&self) -> bool {
        self.cosmetic_equipped
    }

    /// Current version (delegates to the embedded [`AggregateRoot`]).
    pub fn version(&self) -> u64 {
        self.root.version()
    }

    /// Events produced but not yet persisted.
    pub fn uncommitted_events(&self) -> &[Box<dyn DomainEvent>] {
        self.root.uncommitted_events()
    }

    /// Set the owned quantity of the base card (e.g. to zero — not present — or
    /// to a corrupt negative value).
    pub fn set_base_card_quantity(&mut self, quantity: i64) {
        self.base_card_quantity = quantity;
    }

    /// Record whether the player owns the base card the cosmetic targets.
    pub fn set_owns_base_card(&mut self, owns: bool) {
        self.owns_base_card = owns;
    }

    /// Record whether a cosmetic is currently equipped on the base card.
    pub fn set_cosmetic_equipped(&mut self, equipped: bool) {
        self.cosmetic_equipped = equipped;
    }

    /// Record whether the cosmetic equip was resolved server-side.
    pub fn set_server_resolved(&mut self, resolved: bool) {
        self.server_resolved = resolved;
    }

    /// Server-resolved-equip invariant: cosmetic equips are resolved server-side
    /// and never trusted from the client.
    fn ensure_server_resolved(&self) -> Result<(), DomainError> {
        if !self.server_resolved {
            return Err(DomainError::InvariantViolation(format!(
                "player '{}' has a client-asserted cosmetic equip; cosmetic equips are resolved \
                 server-side and never trusted from the client",
                self.id
            )));
        }
        Ok(())
    }

    /// Non-negative-quantity invariant: owned card quantities are always
    /// non-negative.
    fn ensure_quantities_non_negative(&self) -> Result<(), DomainError> {
        if self.base_card_quantity < 0 {
            return Err(DomainError::InvariantViolation(format!(
                "player '{}' records a base-card quantity of {}; owned card quantities are always \
                 non-negative",
                self.id, self.base_card_quantity
            )));
        }
        Ok(())
    }

    /// Card-present invariant: a card may only be included in an Outfit if it is
    /// present (qty ≥ 1) in the collection.
    fn ensure_base_card_present(&self) -> Result<(), DomainError> {
        if self.base_card_quantity < 1 {
            return Err(DomainError::InvariantViolation(format!(
                "player '{}' does not have the base card present (qty {}); a card may only be \
                 included in an Outfit if it is present (qty ≥ 1) in the collection",
                self.id, self.base_card_quantity
            )));
        }
        Ok(())
    }

    /// Owned-base-card invariant: a cosmetic skin may only be equipped onto a
    /// base card the player actually owns.
    fn ensure_owns_base_card(&self) -> Result<(), DomainError> {
        if !self.owns_base_card {
            return Err(DomainError::InvariantViolation(format!(
                "player '{}' does not own the base card; a cosmetic skin may only be equipped onto \
                 a base card the player actually owns",
                self.id
            )));
        }
        Ok(())
    }

    /// Handle `UnequipCosmeticCmd`: verify the command targets this player with a
    /// valid identity and names a valid base card, enforce every invariant
    /// (server-resolved equips, non-negative quantities, card present, and owned
    /// base card), remove the cosmetic, and emit [`Event::CosmeticUnequipped`].
    fn unequip_cosmetic(&mut self, cmd: UnequipCosmetic) -> Result<Vec<Event>, DomainError> {
        // A valid playerId must be supplied.
        if cmd.player_id.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "collection '{}' requires a valid playerId to unequip a cosmetic",
                self.id
            )));
        }
        // A valid baseCardId must be supplied.
        if cmd.base_card_id.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "player '{}' requires a valid baseCardId to unequip a cosmetic",
                self.id
            )));
        }
        // The command must name the player this aggregate actually records.
        if cmd.player_id != self.id {
            return Err(DomainError::InvariantViolation(format!(
                "command targets player '{}' but this aggregate records '{}'",
                cmd.player_id, self.id
            )));
        }

        // Enforce every invariant before removing the cosmetic.
        self.ensure_server_resolved()?;
        self.ensure_quantities_non_negative()?;
        self.ensure_base_card_present()?;
        self.ensure_owns_base_card()?;

        let event = Event::CosmeticUnequipped(CosmeticUnequipped {
            player_id: cmd.player_id,
            base_card_id: cmd.base_card_id,
        });
        // Remove the cosmetic: nothing is equipped on the base card anymore.
        self.cosmetic_equipped = false;
        self.root.record(Box::new(event.clone()));
        Ok(vec![event])
    }
}

impl Aggregate for PlayerCollection {
    type Event = Event;

    fn aggregate_type() -> &'static str {
        AGGREGATE_TYPE
    }

    fn execute(&mut self, command: Command) -> Result<Vec<Self::Event>, DomainError> {
        match command.name.as_str() {
            UNEQUIP_COSMETIC => {
                let cmd: UnequipCosmetic =
                    serde_json::from_slice(&command.payload).map_err(|e| {
                        DomainError::InvariantViolation(format!(
                            "malformed UnequipCosmeticCmd payload: {e}"
                        ))
                    })?;
                self.unequip_cosmetic(cmd)
            }
            // Any other command is unknown to this aggregate.
            _ => Err(DomainError::unknown_command(
                <Self as Aggregate>::aggregate_type(),
                command.name,
            )),
        }
    }
}

/// Repository contract for the [`PlayerCollection`] aggregate. Adapters implement
/// [`shared::Repository`] for `PlayerCollection` and then this marker trait.
pub trait PlayerCollectionRepository: Repository<PlayerCollection> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unequip-ready collection for player `p-01`: a server-resolved cosmetic
    /// is equipped on a base card the player owns one copy of. Tests mutate one
    /// aspect at a time to drive a specific rejection.
    fn ready_collection() -> PlayerCollection {
        let mut collection = PlayerCollection::new("p-01");
        collection.set_base_card_quantity(1);
        collection.set_owns_base_card(true);
        collection.set_cosmetic_equipped(true);
        collection.set_server_resolved(true);
        collection
    }

    /// A command unequipping base card `card-01`'s cosmetic for player `p-01`.
    fn valid_cmd() -> UnequipCosmetic {
        UnequipCosmetic::new("p-01", "card-01")
    }

    // Scenario: Successfully execute UnequipCosmeticCmd.
    #[test]
    fn unequips_and_emits_cosmetic_unequipped_event() {
        let mut collection = ready_collection();

        let events = collection
            .execute(valid_cmd().into_command())
            .expect("valid unequip should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type(), "cosmetic.unequipped");
        match &events[0] {
            Event::CosmeticUnequipped(unequipped) => {
                assert_eq!(unequipped.player_id, "p-01");
                assert_eq!(unequipped.base_card_id, "card-01");
            }
        }
        // The collection recorded the event and the cosmetic is now removed.
        assert!(!collection.cosmetic_equipped());
        assert_eq!(collection.version(), 1);
        assert_eq!(collection.uncommitted_events().len(), 1);
        assert_eq!(
            collection.uncommitted_events()[0].event_type(),
            "cosmetic.unequipped"
        );
    }

    // Scenario: rejected — a card may only be included in an Outfit if it is
    // present (qty ≥ 1) in the collection.
    #[test]
    fn rejects_when_base_card_not_present() {
        let mut collection = ready_collection();
        // Zero copies owned: present-but-non-negative, so this fails presence,
        // not the non-negative invariant.
        collection.set_base_card_quantity(0);

        let err = collection
            .execute(valid_cmd().into_command())
            .expect_err("a base card not present must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(collection.version(), 0);
    }

    // Scenario: rejected — owned card quantities are always non-negative.
    #[test]
    fn rejects_when_quantity_is_negative() {
        let mut collection = ready_collection();
        // A corrupt negative owned quantity.
        collection.set_base_card_quantity(-1);

        let err = collection
            .execute(valid_cmd().into_command())
            .expect_err("a negative owned quantity must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(collection.version(), 0);
    }

    // Scenario: rejected — a cosmetic skin may only be equipped onto a base card
    // the player actually owns.
    #[test]
    fn rejects_when_base_card_not_owned() {
        let mut collection = ready_collection();
        // The player does not own the targeted base card.
        collection.set_owns_base_card(false);

        let err = collection
            .execute(valid_cmd().into_command())
            .expect_err("an unowned base card must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(collection.version(), 0);
    }

    // Scenario: rejected — cosmetic equips are resolved server-side and never
    // trusted from the client.
    #[test]
    fn rejects_when_equip_not_server_resolved() {
        let mut collection = ready_collection();
        // The equip was asserted by the client, not resolved server-side.
        collection.set_server_resolved(false);

        let err = collection
            .execute(valid_cmd().into_command())
            .expect_err("a client-trusted equip must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(collection.version(), 0);
    }

    // A command naming a different player is rejected before any invariant runs.
    #[test]
    fn rejects_command_for_a_different_player() {
        let mut collection = ready_collection();
        let cmd = UnequipCosmetic::new("p-99", "card-01");

        let err = collection
            .execute(cmd.into_command())
            .expect_err("a command for another player must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(collection.version(), 0);
    }

    // A command with no playerId is rejected.
    #[test]
    fn rejects_command_without_a_player_id() {
        let mut collection = ready_collection();
        let cmd = UnequipCosmetic::new("   ", "card-01");

        let err = collection
            .execute(cmd.into_command())
            .expect_err("a missing playerId must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(collection.version(), 0);
    }

    // A command with no baseCardId is rejected.
    #[test]
    fn rejects_command_without_a_base_card_id() {
        let mut collection = ready_collection();
        let cmd = UnequipCosmetic::new("p-01", "   ");

        let err = collection
            .execute(cmd.into_command())
            .expect_err("a missing baseCardId must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(collection.version(), 0);
    }

    // An unrecognized command is still an UnknownCommand for this aggregate,
    // preserving the contract the mock adapters rely on.
    #[test]
    fn rejects_unknown_command() {
        let mut collection = PlayerCollection::new("p-01");
        let err = collection
            .execute(Command::new("NoSuchCommand"))
            .unwrap_err();
        match err {
            DomainError::UnknownCommand { aggregate, command } => {
                assert_eq!(aggregate, "PlayerCollection");
                assert_eq!(command, "NoSuchCommand");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn command_payload_round_trips() {
        let cmd = valid_cmd();
        let command = cmd.into_command();
        assert_eq!(command.name, UnequipCosmetic::COMMAND);
        let decoded: UnequipCosmetic = serde_json::from_slice(&command.payload).unwrap();
        assert_eq!(decoded, valid_cmd());
    }
}
