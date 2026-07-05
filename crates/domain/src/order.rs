//! Order bounded context — a purchase in the shop-and-payments context.
//!
//! An [`Order`] is a single storefront purchase whose payment settles through
//! Stripe. Five invariants govern whether a payment may be confirmed, and every
//! one of them is re-checked when a Stripe webhook reports a payment intent has
//! succeeded:
//!
//! 1. **Fiat via Stripe only** — payment currency is fiat settled via Stripe; an
//!    Order may never settle in the in-game `$MADE` soft currency.
//! 2. **Total equals line items** — the order total must equal the sum of its
//!    line items; a mismatched total cannot be confirmed.
//! 3. **HMAC-verified webhook** — fulfillment occurs only after payment is
//!    confirmed via an HMAC-verified Stripe webhook; an unverified (spoofable)
//!    webhook may not confirm payment.
//! 4. **Idempotent per payment intent** — processing is idempotent per Stripe
//!    payment intent; a payment intent already processed may not be confirmed a
//!    second time (no double-fulfillment).
//! 5. **Refund reverses exactly** — a refund reverses exactly the entitlements
//!    the order granted; an Order whose refund/entitlement ledger is out of
//!    balance may not be confirmed.
//!
//! The only command implemented so far is [`ConfirmPayment`] (`ConfirmPaymentCmd`):
//! it marks payment confirmed from a verified Stripe webhook, enforcing every
//! invariant, and on success emits [`Event::PaymentConfirmed`]
//! (`payment.confirmed`). This module is hand-written (it does not use
//! `shared::stub_aggregate!`) but preserves the same public surface — an
//! [`Order`] aggregate and an [`OrderRepository`] port — so any persistence
//! adapters compile against it unchanged, exactly like its sibling
//! [`Outfit`](crate::outfit).

use serde::{Deserialize, Serialize};

use shared::{Aggregate, AggregateRoot, Command, DomainError, DomainEvent, Repository};

/// Stable aggregate type name, surfaced in [`DomainError::UnknownCommand`] and
/// used for command routing.
const AGGREGATE_TYPE: &str = "Order";

/// The command name [`Order::execute`] recognizes.
const CONFIRM_PAYMENT: &str = "ConfirmPaymentCmd";

/// The `ConfirmPaymentCmd` payload: which Order is being confirmed and the
/// Stripe payment intent reference the confirmation is for. Field names use the
/// payments service's `camelCase` schema.
///
/// Build one directly and turn it into a [`Command`] with
/// [`ConfirmPayment::into_command`], or decode it from a command payload via
/// [`serde_json`] inside [`Order::execute`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmPayment {
    /// The Order the payment is confirmed for; must name this Order, and must be
    /// non-empty.
    pub order_id: String,
    /// The Stripe payment intent the confirmation is for; must be non-empty.
    pub payment_intent_ref: String,
}

impl ConfirmPayment {
    /// The command name this maps to.
    pub const COMMAND: &'static str = CONFIRM_PAYMENT;

    /// Build a command confirming `payment_intent_ref` for `order_id`.
    pub fn new(order_id: impl Into<String>, payment_intent_ref: impl Into<String>) -> Self {
        Self {
            order_id: order_id.into(),
            payment_intent_ref: payment_intent_ref.into(),
        }
    }

    /// Encode this command as a [`shared::Command`] carrying a JSON payload,
    /// ready to hand to [`Order::execute`].
    pub fn into_command(&self) -> Command {
        // Serialization of a plain data struct to a Vec cannot fail here.
        let payload = serde_json::to_vec(self).expect("ConfirmPayment is always serializable");
        Command::with_payload(Self::COMMAND, payload)
    }
}

/// The payment that was confirmed, carried by [`Event::PaymentConfirmed`] and
/// thus by the emitted `payment.confirmed` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentConfirmed {
    /// The Order whose payment was confirmed.
    pub order_id: String,
    /// The Stripe payment intent that settled the Order.
    pub payment_intent_ref: String,
}

/// Domain events emitted by [`Order`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Payment for the Order was confirmed from a verified Stripe webhook.
    PaymentConfirmed(PaymentConfirmed),
}

impl DomainEvent for Event {
    fn event_type(&self) -> &'static str {
        match self {
            Event::PaymentConfirmed(_) => "payment.confirmed",
        }
    }
}

/// The Order aggregate: one storefront purchase settled through Stripe.
///
/// Mirrors the shape produced by [`shared::stub_aggregate!`] (identity plus an
/// embedded [`AggregateRoot`]) so the surrounding wiring is unchanged, while it
/// now carries the state the [`ConfirmPayment`] command validates against:
/// whether the payment currency is fiat (never `$MADE`), whether the order
/// total equals the sum of its line items, whether the confirming webhook was
/// HMAC-verified, whether this payment intent was already processed, and whether
/// the refund/entitlement ledger balances.
///
/// A fresh Order from [`Order::new`] is confirmable: it settles in fiat via
/// Stripe, its total matches its line items, its webhook is HMAC-verified, its
/// payment intent has not yet been processed, and its refunds reverse exactly
/// the entitlements granted. The configuration methods below drive it to a state
/// a command rejects, exactly as [`Outfit`](crate::outfit) is built up before a
/// command validates it.
#[derive(Debug)]
pub struct Order {
    id: String,
    root: AggregateRoot,
    /// Whether the payment currency is fiat settled via Stripe. `false` means it
    /// would settle in the in-game `$MADE` currency, which is never allowed.
    currency_is_fiat_via_stripe: bool,
    /// Whether the order total equals the sum of its line items.
    total_matches_line_items: bool,
    /// Whether the confirming Stripe webhook's signature was HMAC-verified.
    webhook_hmac_verified: bool,
    /// Whether this Stripe payment intent has already been processed. Confirming
    /// an already-processed intent would double-fulfill, so it is rejected.
    payment_intent_already_processed: bool,
    /// Whether every refund reverses exactly the entitlements the order granted
    /// (the refund/entitlement ledger balances).
    refund_reverses_exactly: bool,
}

impl Order {
    /// Create a new, confirmable Order identified by `id`: it settles in fiat via
    /// Stripe, its total matches its line items, its webhook is HMAC-verified,
    /// its payment intent has not been processed, and its refunds reverse exactly
    /// the entitlements granted. Use the configuration methods to drive it to the
    /// state a command validates.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root: AggregateRoot::new(),
            currency_is_fiat_via_stripe: true,
            total_matches_line_items: true,
            webhook_hmac_verified: true,
            payment_intent_already_processed: false,
            refund_reverses_exactly: true,
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

    /// Record whether the payment currency is fiat settled via Stripe (`false`
    /// models an attempt to settle in `$MADE`).
    pub fn set_currency_is_fiat_via_stripe(&mut self, ok: bool) {
        self.currency_is_fiat_via_stripe = ok;
    }

    /// Record whether the order total equals the sum of its line items.
    pub fn set_total_matches_line_items(&mut self, ok: bool) {
        self.total_matches_line_items = ok;
    }

    /// Record whether the confirming Stripe webhook was HMAC-verified.
    pub fn set_webhook_hmac_verified(&mut self, ok: bool) {
        self.webhook_hmac_verified = ok;
    }

    /// Record whether this Stripe payment intent has already been processed.
    pub fn set_payment_intent_already_processed(&mut self, already: bool) {
        self.payment_intent_already_processed = already;
    }

    /// Record whether every refund reverses exactly the entitlements granted.
    pub fn set_refund_reverses_exactly(&mut self, ok: bool) {
        self.refund_reverses_exactly = ok;
    }

    /// Currency invariant: payment currency is fiat via Stripe only — an Order
    /// may never settle in `$MADE`.
    fn ensure_fiat_via_stripe(&self) -> Result<(), DomainError> {
        if !self.currency_is_fiat_via_stripe {
            return Err(DomainError::InvariantViolation(format!(
                "order '{}' would settle in $MADE; payment currency is fiat via Stripe only — an \
                 Order may never settle in $MADE",
                self.id
            )));
        }
        Ok(())
    }

    /// Total invariant: the order total must equal the sum of its line items.
    fn ensure_total_matches_line_items(&self) -> Result<(), DomainError> {
        if !self.total_matches_line_items {
            return Err(DomainError::InvariantViolation(format!(
                "order '{}' total does not equal the sum of its line items; the order total must \
                 equal the sum of its line items",
                self.id
            )));
        }
        Ok(())
    }

    /// Webhook invariant: fulfillment occurs only after payment is confirmed via
    /// an HMAC-verified Stripe webhook.
    fn ensure_webhook_hmac_verified(&self) -> Result<(), DomainError> {
        if !self.webhook_hmac_verified {
            return Err(DomainError::InvariantViolation(format!(
                "order '{}' payment was not confirmed via an HMAC-verified Stripe webhook; \
                 fulfillment occurs only after payment is confirmed via an HMAC-verified Stripe \
                 webhook",
                self.id
            )));
        }
        Ok(())
    }

    /// Idempotency invariant: processing is idempotent per Stripe payment intent
    /// — an already-processed intent must not be confirmed again (no
    /// double-fulfillment).
    fn ensure_not_already_processed(&self) -> Result<(), DomainError> {
        if self.payment_intent_already_processed {
            return Err(DomainError::InvariantViolation(format!(
                "order '{}' payment intent was already processed; processing is idempotent per \
                 Stripe payment intent (no double-fulfillment)",
                self.id
            )));
        }
        Ok(())
    }

    /// Refund invariant: a refund reverses exactly the entitlements the order
    /// granted.
    fn ensure_refund_reverses_exactly(&self) -> Result<(), DomainError> {
        if !self.refund_reverses_exactly {
            return Err(DomainError::InvariantViolation(format!(
                "order '{}' refund/entitlement ledger is out of balance; a refund reverses exactly \
                 the entitlements the order granted",
                self.id
            )));
        }
        Ok(())
    }

    /// Handle `ConfirmPaymentCmd`: verify the command carries a valid orderId
    /// (naming this Order) and paymentIntentRef, enforce every invariant (fiat
    /// via Stripe, total-equals-line-items, HMAC-verified webhook, idempotency,
    /// and refund-reverses-exactly), mark the payment intent processed, and emit
    /// [`Event::PaymentConfirmed`].
    fn confirm_payment(&mut self, cmd: ConfirmPayment) -> Result<Vec<Event>, DomainError> {
        // A valid orderId and paymentIntentRef must be supplied.
        if cmd.order_id.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "order '{}' requires a valid orderId to confirm payment",
                self.id
            )));
        }
        if cmd.payment_intent_ref.trim().is_empty() {
            return Err(DomainError::InvariantViolation(format!(
                "order '{}' requires a valid paymentIntentRef to confirm payment",
                self.id
            )));
        }
        // The command must name the Order it is dispatched to.
        if cmd.order_id != self.id {
            return Err(DomainError::InvariantViolation(format!(
                "command targets order '{}' but this aggregate is order '{}'",
                cmd.order_id, self.id
            )));
        }

        // Enforce every invariant before recording the confirmation.
        self.ensure_fiat_via_stripe()?;
        self.ensure_total_matches_line_items()?;
        self.ensure_webhook_hmac_verified()?;
        self.ensure_not_already_processed()?;
        self.ensure_refund_reverses_exactly()?;

        // Mark the payment intent processed so a replayed webhook for the same
        // intent is rejected by the idempotency invariant — no double-fulfillment.
        self.payment_intent_already_processed = true;

        let event = Event::PaymentConfirmed(PaymentConfirmed {
            order_id: cmd.order_id,
            payment_intent_ref: cmd.payment_intent_ref,
        });
        self.root.record(Box::new(event.clone()));
        Ok(vec![event])
    }
}

impl Aggregate for Order {
    type Event = Event;

    fn aggregate_type() -> &'static str {
        AGGREGATE_TYPE
    }

    fn execute(&mut self, command: Command) -> Result<Vec<Self::Event>, DomainError> {
        match command.name.as_str() {
            CONFIRM_PAYMENT => {
                let cmd: ConfirmPayment =
                    serde_json::from_slice(&command.payload).map_err(|e| {
                        DomainError::InvariantViolation(format!(
                            "malformed ConfirmPaymentCmd payload: {e}"
                        ))
                    })?;
                self.confirm_payment(cmd)
            }
            // Any other command is unknown to this aggregate.
            _ => Err(DomainError::unknown_command(
                <Self as Aggregate>::aggregate_type(),
                command.name,
            )),
        }
    }
}

/// Repository contract for the [`Order`] aggregate. Adapters implement
/// [`shared::Repository`] for `Order` and then this marker trait.
pub trait OrderRepository: Repository<Order> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A confirmable Order `o-01`: fiat via Stripe, total matches line items,
    /// HMAC-verified webhook, payment intent not yet processed, refunds reverse
    /// exactly. Tests mutate one aspect at a time to drive a specific rejection.
    fn ready_order() -> Order {
        let mut order = Order::new("o-01");
        order.set_currency_is_fiat_via_stripe(true);
        order.set_total_matches_line_items(true);
        order.set_webhook_hmac_verified(true);
        order.set_payment_intent_already_processed(false);
        order.set_refund_reverses_exactly(true);
        order
    }

    /// A command confirming payment intent `pi_123` for order `o-01`.
    fn valid_cmd() -> ConfirmPayment {
        ConfirmPayment::new("o-01", "pi_123")
    }

    // Scenario: Successfully execute ConfirmPaymentCmd.
    #[test]
    fn confirms_and_emits_payment_confirmed_event() {
        let mut order = ready_order();

        let events = order
            .execute(valid_cmd().into_command())
            .expect("valid confirmation should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type(), "payment.confirmed");
        match &events[0] {
            Event::PaymentConfirmed(confirmed) => {
                assert_eq!(confirmed.order_id, "o-01");
                assert_eq!(confirmed.payment_intent_ref, "pi_123");
            }
        }
        // The Order recorded the event.
        assert_eq!(order.version(), 1);
        assert_eq!(order.uncommitted_events().len(), 1);
        assert_eq!(
            order.uncommitted_events()[0].event_type(),
            "payment.confirmed"
        );
    }

    // Scenario: rejected — Payment currency is fiat via Stripe only — an Order
    // may never settle in $MADE.
    #[test]
    fn rejects_when_currency_is_not_fiat_via_stripe() {
        let mut order = ready_order();
        // The Order attempts to settle in $MADE rather than fiat via Stripe.
        order.set_currency_is_fiat_via_stripe(false);

        let err = order
            .execute(valid_cmd().into_command())
            .expect_err("an Order settling in $MADE must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(order.version(), 0);
    }

    // Scenario: rejected — The order total must equal the sum of its line items.
    #[test]
    fn rejects_when_total_does_not_match_line_items() {
        let mut order = ready_order();
        // The order total no longer equals the sum of its line items.
        order.set_total_matches_line_items(false);

        let err = order
            .execute(valid_cmd().into_command())
            .expect_err("an Order whose total mismatches its line items must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(order.version(), 0);
    }

    // Scenario: rejected — Fulfillment occurs only after payment is confirmed via
    // an HMAC-verified Stripe webhook.
    #[test]
    fn rejects_when_webhook_not_hmac_verified() {
        let mut order = ready_order();
        // The confirming webhook's HMAC signature was not verified.
        order.set_webhook_hmac_verified(false);

        let err = order
            .execute(valid_cmd().into_command())
            .expect_err("an unverified Stripe webhook must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(order.version(), 0);
    }

    // Scenario: rejected — Processing is idempotent per Stripe payment intent (no
    // double-fulfillment).
    #[test]
    fn rejects_when_payment_intent_already_processed() {
        let mut order = ready_order();
        // This payment intent has already been processed once.
        order.set_payment_intent_already_processed(true);

        let err = order
            .execute(valid_cmd().into_command())
            .expect_err("an already-processed payment intent must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(order.version(), 0);
    }

    // Idempotency in practice: a second confirmation of the same payment intent
    // is rejected because the first marked it processed (no double-fulfillment).
    #[test]
    fn rejects_a_replayed_confirmation_of_the_same_intent() {
        let mut order = ready_order();

        order
            .execute(valid_cmd().into_command())
            .expect("first confirmation should succeed");
        // The webhook is redelivered for the same intent.
        let err = order
            .execute(valid_cmd().into_command())
            .expect_err("a replayed confirmation must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        // Still exactly one recorded event — no double-fulfillment.
        assert_eq!(order.version(), 1);
        assert_eq!(order.uncommitted_events().len(), 1);
    }

    // Scenario: rejected — A refund reverses exactly the entitlements the order
    // granted.
    #[test]
    fn rejects_when_refund_does_not_reverse_exactly() {
        let mut order = ready_order();
        // The refund/entitlement ledger is out of balance.
        order.set_refund_reverses_exactly(false);

        let err = order
            .execute(valid_cmd().into_command())
            .expect_err("an out-of-balance refund ledger must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(order.version(), 0);
    }

    // A command naming a different Order is rejected before any invariant runs.
    #[test]
    fn rejects_command_for_a_different_order() {
        let mut order = ready_order();
        let cmd = ConfirmPayment::new("o-99", "pi_123");

        let err = order
            .execute(cmd.into_command())
            .expect_err("a command for another order must be rejected");
        assert!(matches!(err, DomainError::InvariantViolation(_)));
        assert_eq!(order.version(), 0);
    }

    // Commands missing any required field are rejected.
    #[test]
    fn rejects_command_with_missing_fields() {
        for cmd in [
            ConfirmPayment::new("   ", "pi_123"),
            ConfirmPayment::new("o-01", "   "),
        ] {
            let mut order = ready_order();
            let err = order
                .execute(cmd.into_command())
                .expect_err("a command with a missing field must be rejected");
            assert!(matches!(err, DomainError::InvariantViolation(_)));
            assert_eq!(order.version(), 0);
        }
    }

    // An unrecognized command is still an UnknownCommand for this aggregate,
    // preserving the contract the mock adapters rely on.
    #[test]
    fn rejects_unknown_command() {
        let mut order = Order::new("o-01");
        let err = order.execute(Command::new("NoSuchCommand")).unwrap_err();
        match err {
            DomainError::UnknownCommand { aggregate, command } => {
                assert_eq!(aggregate, "Order");
                assert_eq!(command, "NoSuchCommand");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn command_payload_round_trips() {
        let cmd = valid_cmd();
        let command = cmd.into_command();
        assert_eq!(command.name, ConfirmPayment::COMMAND);
        let decoded: ConfirmPayment = serde_json::from_slice(&command.payload).unwrap();
        assert_eq!(decoded, valid_cmd());
    }
}
