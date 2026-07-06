-- Shop-and-payments bounded context: fiat Orders settled via Stripe, plus the
-- purchasable CardPack and BattlePass catalog entries.
--
-- Order is the aggregate root; order_line_items is its line-item ledger. The
-- domain invariant "order total equals the sum of line items" is application-
-- enforced, but money columns are constrained non-negative here, and the Stripe
-- payment intent is UNIQUE so a webhook cannot double-fulfill an order.
-- Amounts are stored as BIGINT minor units (cents) — never floating point.

-- Order aggregate: one storefront purchase.
CREATE TABLE orders (
    id                       TEXT PRIMARY KEY,
    player_id                TEXT NOT NULL,
    currency                 TEXT NOT NULL CHECK (char_length(currency) = 3),
    total_amount             BIGINT NOT NULL DEFAULT 0 CHECK (total_amount >= 0),
    status                   TEXT NOT NULL DEFAULT 'Created'
                                 CHECK (status IN ('Created', 'PaymentConfirmed', 'Fulfilled', 'Refunded')),
    stripe_payment_intent_id TEXT UNIQUE,     -- idempotency key; NULL until a PI is attached
    version                  BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_orders_player ON orders (player_id);
CREATE TRIGGER trg_orders_updated
    BEFORE UPDATE ON orders
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Line-item ledger for an order. `line_amount = unit_amount * quantity` is the
-- application's responsibility; the schema keeps every money column non-negative
-- and quantities strictly positive.
CREATE TABLE order_line_items (
    id          TEXT PRIMARY KEY,
    order_id    TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    sku         TEXT NOT NULL,
    unit_amount BIGINT NOT NULL CHECK (unit_amount >= 0),
    quantity    INTEGER NOT NULL CHECK (quantity > 0),
    line_amount BIGINT NOT NULL CHECK (line_amount >= 0)
);
CREATE INDEX idx_order_line_items_order ON order_line_items (order_id);

-- CardPack aggregate: a purchasable pack of cards from an expansion.
CREATE TABLE card_packs (
    id               TEXT PRIMARY KEY,
    expansion_set_id TEXT NOT NULL REFERENCES expansion_sets (id) ON DELETE RESTRICT,
    name             TEXT NOT NULL,
    price_amount     BIGINT NOT NULL CHECK (price_amount >= 0),
    card_count       INTEGER NOT NULL CHECK (card_count > 0),
    version          BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_card_packs_expansion ON card_packs (expansion_set_id);
CREATE TRIGGER trg_card_packs_updated
    BEFORE UPDATE ON card_packs
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- BattlePass aggregate: a season-scoped progression track.
CREATE TABLE battle_passes (
    id           TEXT PRIMARY KEY,
    season_id    TEXT NOT NULL REFERENCES seasons (id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    tier_count   INTEGER NOT NULL CHECK (tier_count > 0),
    price_amount BIGINT NOT NULL CHECK (price_amount >= 0),
    version      BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- At most one battle pass per season.
    CONSTRAINT battle_passes_one_per_season UNIQUE (season_id)
);
CREATE TRIGGER trg_battle_passes_updated
    BEFORE UPDATE ON battle_passes
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
