//! CardDefinition bounded context — the catalog of playable card definitions.
//!
//! A [`CardDefinition`] is the authored, catalog-level description of a single
//! playable card: its type, class, Juice cost, effect-script reference, rarity
//! and per-Outfit copy cap. It is the source of truth deck-construction and the
//! rules engine validate against, so every definition must satisfy five
//! standing invariants to be *legal*:
//!
//! 1. **Cost range** — a card's Juice cost must fall within the legal cost range
//!    for its type.
//! 2. **Single class** — a card belongs to exactly one class or is Neutral; no
//!    card may claim two classes.
//! 3. **Typed** — every card is exactly one of the five card types: Operator,
//!    Job, Piece, Vehicle, or Heist.
//! 4. **Resolvable effect** — a card's effect-script reference must resolve to a
//!    registered effect in the engine.
//! 5. **Legendary copy cap** — Legendary rarity carries a per-Outfit copy cap of
//!    1, declared on the definition.
//!
//! The only command implemented so far is [`DeprecateCard`] (`DeprecateCardCmd`):
//! it retires a card from legal construction going forward. Because deprecation
//! is a catalog-integrity action, it may only be applied to a definition that is
//! itself legal — so the handler enforces all five invariants before emitting
//! [`Event::Deprecated`] (`card.deprecated`). This module is hand-written (it no
//! longer uses `shared::stub_aggregate!`) but preserves the same public surface
//! — a [`CardDefinition`] aggregate and a [`CardDefinitionRepository`] port — so
//! the persistence adapters in `crates/mocks` keep compiling unchanged.

use shared::{Aggregate, AggregateRoot, Command, DomainError, DomainEvent, Repository};

/// Stable aggregate type name, used in errors and event routing.
const AGGREGATE_TYPE: &str = "CardDefinition";

/// Effect-script references the rules engine knows how to resolve. A card whose
/// `effect_script` is not in this registry cannot be enacted, so it is illegal.
/// In a full build this would be sourced from the engine's registered-effect
/// table; here it is a fixed snapshot the catalog is authored against.
const REGISTERED_EFFECTS: &[&str] = &[
    "fx.noop",
    "fx.draw_card",
    "fx.deal_damage",
    "fx.gain_juice",
    "fx.steal_piece",
    "fx.pull_heist",
];

/// The five — and only five — card types. Every legal card is exactly one of
/// these (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardType {
    Operator,
    Job,
    Piece,
    Vehicle,
    Heist,
}

impl CardType {
    /// The inclusive `(min, max)` Juice cost range legal for this type
    /// (invariant 1). Costs outside the range make the definition illegal.
    pub fn legal_cost_range(self) -> (u32, u32) {
        match self {
            CardType::Operator => (1, 8),
            CardType::Job => (0, 6),
            CardType::Piece => (0, 4),
            CardType::Vehicle => (2, 7),
            CardType::Heist => (3, 10),
        }
    }
}

/// The playable classes a card may belong to. A card claims at most one of these
/// (invariant 2); claiming none makes it *Neutral*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardClass {
    Enforcer,
    Grifter,
    Hacker,
    Fixer,
    Wheelman,
}

/// A card's rarity tier. `Legendary` carries a special copy-cap rule
/// (invariant 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Legendary,
}

/// The catalog definition of a single playable card.
///
/// Fields are modelled so an *illegal* definition is representable — a card may
/// hold no type, two classes, an out-of-range cost, and so on — because the
/// invariants are enforced at command time, not made unrepresentable. This
/// mirrors the way the catalog receives authored, not-yet-validated data.
#[derive(Debug)]
pub struct CardDefinition {
    id: String,
    root: AggregateRoot,
    /// The card's type. `None` models a card that has failed to declare exactly
    /// one of the five types (invariant 3).
    card_type: Option<CardType>,
    /// The classes the card claims. Empty is *Neutral*; more than one violates
    /// the single-class invariant (invariant 2).
    classes: Vec<CardClass>,
    /// Declared Juice cost, validated against the type's legal range
    /// (invariant 1).
    juice_cost: u32,
    /// Effect-script reference, which must resolve in [`REGISTERED_EFFECTS`]
    /// (invariant 4).
    effect_script: String,
    /// Rarity tier; `Legendary` constrains `copy_cap` (invariant 5).
    rarity: Rarity,
    /// Declared per-Outfit copy cap (invariant 5).
    copy_cap: u32,
    /// Whether the card has already been retired from legal construction.
    deprecated: bool,
}

impl CardDefinition {
    /// Create a fresh, *legal* definition for the card identified by `id`.
    ///
    /// Sensible catalog defaults are chosen so a new definition satisfies every
    /// invariant out of the box; authors then refine it with the `with_*`
    /// builders below. Keeping a single-argument `new` preserves the surface the
    /// mock adapters in `crates/mocks` construct against.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root: AggregateRoot::new(),
            card_type: Some(CardType::Operator),
            classes: Vec::new(),
            juice_cost: 1,
            effect_script: "fx.noop".to_string(),
            rarity: Rarity::Common,
            copy_cap: 1,
            deprecated: false,
        }
    }

    /// Set the card's type.
    pub fn with_type(mut self, card_type: CardType) -> Self {
        self.card_type = Some(card_type);
        self
    }

    /// Set the classes the card claims (empty = Neutral).
    pub fn with_classes(mut self, classes: Vec<CardClass>) -> Self {
        self.classes = classes;
        self
    }

    /// Set the declared Juice cost.
    pub fn with_juice_cost(mut self, juice_cost: u32) -> Self {
        self.juice_cost = juice_cost;
        self
    }

    /// Set the effect-script reference.
    pub fn with_effect_script(mut self, effect_script: impl Into<String>) -> Self {
        self.effect_script = effect_script.into();
        self
    }

    /// Set the rarity and per-Outfit copy cap together, since Legendary ties
    /// the two.
    pub fn with_rarity(mut self, rarity: Rarity, copy_cap: u32) -> Self {
        self.rarity = rarity;
        self.copy_cap = copy_cap;
        self
    }

    /// This definition's identity.
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

    /// Whether this card has been retired from legal construction.
    pub fn is_deprecated(&self) -> bool {
        self.deprecated
    }

    /// A card is Neutral when it claims no class.
    pub fn is_neutral(&self) -> bool {
        self.classes.is_empty()
    }

    /// Invariant 3: the card declares exactly one of the five types.
    fn ensure_typed(&self) -> Result<CardType, DomainError> {
        self.card_type.ok_or_else(|| {
            DomainError::InvariantViolation(format!(
                "card '{}' has no type; every card must be exactly one of Operator, Job, Piece, \
                 Vehicle, or Heist",
                self.id
            ))
        })
    }

    /// Invariant 1: the Juice cost falls within the legal range for the type.
    fn ensure_cost_in_range(&self, card_type: CardType) -> Result<(), DomainError> {
        let (min, max) = card_type.legal_cost_range();
        if self.juice_cost < min || self.juice_cost > max {
            return Err(DomainError::InvariantViolation(format!(
                "card '{}' Juice cost {} is outside the legal range {min}..={max} for a {:?}",
                self.id, self.juice_cost, card_type
            )));
        }
        Ok(())
    }

    /// Invariant 2: the card belongs to exactly one class or is Neutral; it may
    /// never claim two.
    fn ensure_single_class(&self) -> Result<(), DomainError> {
        if self.classes.len() > 1 {
            return Err(DomainError::InvariantViolation(format!(
                "card '{}' claims {} classes; a card belongs to exactly one class or is Neutral",
                self.id,
                self.classes.len()
            )));
        }
        Ok(())
    }

    /// Invariant 4: the effect-script reference resolves to a registered effect.
    fn ensure_effect_resolves(&self) -> Result<(), DomainError> {
        if !REGISTERED_EFFECTS.contains(&self.effect_script.as_str()) {
            return Err(DomainError::InvariantViolation(format!(
                "card '{}' effect-script '{}' does not resolve to a registered effect in the engine",
                self.id, self.effect_script
            )));
        }
        Ok(())
    }

    /// Invariant 5: Legendary rarity carries a per-Outfit copy cap of exactly 1.
    fn ensure_legendary_copy_cap(&self) -> Result<(), DomainError> {
        if self.rarity == Rarity::Legendary && self.copy_cap != 1 {
            return Err(DomainError::InvariantViolation(format!(
                "card '{}' is Legendary but declares a copy cap of {}; Legendary rarity requires a \
                 per-Outfit copy cap of 1",
                self.id, self.copy_cap
            )));
        }
        Ok(())
    }

    /// Enforce every standing invariant, returning the first violation found.
    fn ensure_legal(&self) -> Result<(), DomainError> {
        let card_type = self.ensure_typed()?;
        self.ensure_cost_in_range(card_type)?;
        self.ensure_single_class()?;
        self.ensure_effect_resolves()?;
        self.ensure_legendary_copy_cap()?;
        Ok(())
    }

    /// Handle `DeprecateCardCmd`: retire a legal card from construction going
    /// forward and emit [`Event::Deprecated`].
    fn deprecate_card(&mut self, request: DeprecateCard) -> Result<Vec<Event>, DomainError> {
        // The command must name the card this definition actually describes.
        if request.card_id != self.id {
            return Err(DomainError::InvariantViolation(format!(
                "command targets card '{}' but this definition describes '{}'",
                request.card_id, self.id
            )));
        }

        // A card may only be retired once.
        if self.deprecated {
            return Err(DomainError::InvariantViolation(format!(
                "card '{}' is already deprecated",
                self.id
            )));
        }

        // Deprecation is a catalog-integrity action: only a legal definition may
        // be retired, so enforce every invariant before emitting.
        self.ensure_legal()?;

        self.deprecated = true;
        let event = Event::Deprecated {
            card_id: self.id.clone(),
            reason: request.reason,
        };
        self.root.record(Box::new(event.clone()));
        Ok(vec![event])
    }
}

/// Typed form of the `DeprecateCardCmd` command.
///
/// Retires a card from legal construction going forward, recording why. Because
/// the [`shared`] kernel carries commands as an opaque byte payload (no serde
/// dependency, for `wasm32`), this type also owns the trivial
/// `"<cardId>:<reason>"` wire encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeprecateCard {
    /// The card being retired; must match the definition it is executed against.
    pub card_id: String,
    /// The reason the card is being deprecated (must be non-empty).
    pub reason: String,
}

impl DeprecateCard {
    /// The command name this maps to.
    pub const COMMAND: &'static str = "DeprecateCardCmd";

    /// Build a deprecation of `card_id` with the given `reason`.
    pub fn new(card_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            card_id: card_id.into(),
            reason: reason.into(),
        }
    }

    /// Encode this request as a dispatchable [`Command`].
    pub fn into_command(self) -> Command {
        let payload = format!("{}:{}", self.card_id, self.reason).into_bytes();
        Command::with_payload(Self::COMMAND, payload)
    }

    /// Decode a command payload of the form `"<cardId>:<reason>"`.
    fn decode(payload: &[u8]) -> Result<Self, DomainError> {
        let text = std::str::from_utf8(payload).map_err(|_| {
            DomainError::InvariantViolation("DeprecateCardCmd payload is not UTF-8".to_string())
        })?;
        // Split on the first ':' so the reason may itself contain colons.
        let (card_id, reason) = text.split_once(':').ok_or_else(|| {
            DomainError::InvariantViolation(
                "DeprecateCardCmd payload must be '<cardId>:<reason>'".to_string(),
            )
        })?;
        if card_id.is_empty() {
            return Err(DomainError::InvariantViolation(
                "DeprecateCardCmd requires a non-empty cardId".to_string(),
            ));
        }
        if reason.is_empty() {
            return Err(DomainError::InvariantViolation(
                "DeprecateCardCmd requires a non-empty reason".to_string(),
            ));
        }
        Ok(Self {
            card_id: card_id.to_string(),
            reason: reason.to_string(),
        })
    }
}

/// Domain events emitted by [`CardDefinition`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A card was retired from legal construction going forward. Names the card
    /// and the reason it was deprecated.
    Deprecated {
        /// The card that was retired.
        card_id: String,
        /// Why the card was deprecated.
        reason: String,
    },
}

impl DomainEvent for Event {
    fn event_type(&self) -> &'static str {
        match self {
            Event::Deprecated { .. } => "card.deprecated",
        }
    }
}

impl Aggregate for CardDefinition {
    type Event = Event;

    fn aggregate_type() -> &'static str {
        AGGREGATE_TYPE
    }

    fn execute(&mut self, command: Command) -> Result<Vec<Self::Event>, DomainError> {
        match command.name.as_str() {
            DeprecateCard::COMMAND => {
                let request = DeprecateCard::decode(&command.payload)?;
                self.deprecate_card(request)
            }
            // Any other command is unknown to this aggregate.
            _ => Err(DomainError::unknown_command(
                <Self as Aggregate>::aggregate_type(),
                command.name,
            )),
        }
    }
}

/// Repository contract for the [`CardDefinition`] aggregate. Adapters implement
/// [`Repository`] for [`CardDefinition`] and then this marker trait.
pub trait CardDefinitionRepository: Repository<CardDefinition> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A legal, non-deprecated card definition for `card-1`.
    fn valid_card() -> CardDefinition {
        CardDefinition::new("card-1")
            .with_type(CardType::Operator)
            .with_classes(vec![CardClass::Enforcer])
            .with_juice_cost(3)
            .with_effect_script("fx.draw_card")
            .with_rarity(Rarity::Rare, 3)
    }

    // Scenario: Successfully execute DeprecateCardCmd.
    #[test]
    fn deprecates_and_emits_card_deprecated_event() {
        let mut card = valid_card();

        let events = card
            .execute(DeprecateCard::new("card-1", "power-crept out of the format").into_command())
            .expect("valid deprecation should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type(), "card.deprecated");
        match &events[0] {
            Event::Deprecated { card_id, reason } => {
                assert_eq!(card_id, "card-1");
                assert_eq!(reason, "power-crept out of the format");
            }
        }
        assert!(card.is_deprecated());
        assert_eq!(card.uncommitted_events().len(), 1);
        assert_eq!(card.version(), 1);
    }

    // Scenario: rejected — a card's Juice cost must fall within the legal cost
    // range for its type.
    #[test]
    fn rejects_when_juice_cost_is_out_of_range() {
        // Operator's legal range is 1..=8; 99 is illegal.
        let mut card = valid_card().with_juice_cost(99);

        let err = card
            .execute(DeprecateCard::new("card-1", "cleanup").into_command())
            .expect_err("out-of-range Juice cost must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
    }

    // Scenario: rejected — a card belongs to exactly one class or is Neutral; no
    // card may claim two classes.
    #[test]
    fn rejects_when_card_claims_two_classes() {
        let mut card = valid_card().with_classes(vec![CardClass::Enforcer, CardClass::Hacker]);

        let err = card
            .execute(DeprecateCard::new("card-1", "cleanup").into_command())
            .expect_err("a card claiming two classes must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
    }

    // Scenario: rejected — every card is exactly one of the five card types.
    #[test]
    fn rejects_when_card_has_no_type() {
        let mut card = valid_card();
        // Strip the type so the card is no longer exactly one of the five.
        card.card_type = None;

        let err = card
            .execute(DeprecateCard::new("card-1", "cleanup").into_command())
            .expect_err("an untyped card must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
    }

    // Scenario: rejected — a card's effect-script reference must resolve to a
    // registered effect in the engine.
    #[test]
    fn rejects_when_effect_script_does_not_resolve() {
        let mut card = valid_card().with_effect_script("fx.does_not_exist");

        let err = card
            .execute(DeprecateCard::new("card-1", "cleanup").into_command())
            .expect_err("an unresolved effect-script must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
    }

    // Scenario: rejected — Legendary rarity carries a per-Outfit copy cap of 1.
    #[test]
    fn rejects_when_legendary_copy_cap_is_not_one() {
        let mut card = valid_card().with_rarity(Rarity::Legendary, 3);

        let err = card
            .execute(DeprecateCard::new("card-1", "cleanup").into_command())
            .expect_err("a Legendary card with copy cap != 1 must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
    }

    // An unrecognized command is still an UnknownCommand for this aggregate,
    // preserving the contract the mock adapters rely on.
    #[test]
    fn rejects_unknown_command() {
        let mut card = CardDefinition::new("card-1");
        let err = card.execute(Command::new("NoSuchCommand")).unwrap_err();
        match err {
            DomainError::UnknownCommand { aggregate, command } => {
                assert_eq!(aggregate, "CardDefinition");
                assert_eq!(command, "NoSuchCommand");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn command_payload_round_trips() {
        let command = DeprecateCard::new("card-42", "banned: infinite combo").into_command();
        assert_eq!(command.name, DeprecateCard::COMMAND);
        let decoded = DeprecateCard::decode(&command.payload).unwrap();
        // The reason survives even though it contains a ':'.
        assert_eq!(
            decoded,
            DeprecateCard::new("card-42", "banned: infinite combo")
        );
    }
}
