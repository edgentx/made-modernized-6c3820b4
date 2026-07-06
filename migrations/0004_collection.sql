-- Collection-and-deckbuilding bounded context: a player's owned cards (the
-- ledger) and the Outfits (decks + cosmetics) built from them.
--
-- PlayerCollection is the aggregate root; player_collection_cards is its owned-
-- card ledger. Two ledger invariants from the domain are enforced here in the
-- schema itself:
--   * Non-negative quantities  -> CHECK (quantity >= 0)
--   * Copy caps (e.g. Legendary is capped)  -> CHECK (quantity <= max_copies)
--     plus one row per (collection, card) so a card cannot be double-booked.

-- PlayerCollection aggregate: the set of cards one player owns.
CREATE TABLE player_collections (
    id         TEXT PRIMARY KEY,
    player_id  TEXT NOT NULL UNIQUE,
    version    BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TRIGGER trg_player_collections_updated
    BEFORE UPDATE ON player_collections
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Owned-card ledger. The composite primary key guarantees a single balance row
-- per (collection, card) — the uniqueness constraint — while the two CHECKs
-- enforce non-negative balances and the per-card copy cap. `max_copies`
-- defaults to the Legendary cap of 1; non-Legendary rows are inserted with the
-- looser deck cap the domain allows (e.g. 3), so a single constraint covers the
-- whole rarity ladder without a cross-table trigger.
CREATE TABLE player_collection_cards (
    collection_id      TEXT NOT NULL REFERENCES player_collections (id) ON DELETE CASCADE,
    card_definition_id TEXT NOT NULL REFERENCES card_definitions (id) ON DELETE RESTRICT,
    quantity           INTEGER NOT NULL DEFAULT 0,
    max_copies         INTEGER NOT NULL DEFAULT 1 CHECK (max_copies >= 1),
    acquired_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (collection_id, card_definition_id),
    -- Non-negative balance: a card can never be owned in negative quantity.
    CONSTRAINT player_collection_cards_qty_non_negative
        CHECK (quantity >= 0),
    -- Copy cap: owned quantity may never exceed the card's cap (Legendary = 1).
    CONSTRAINT player_collection_cards_qty_within_cap
        CHECK (quantity <= max_copies)
);
CREATE INDEX idx_player_collection_cards_card ON player_collection_cards (card_definition_id);

-- Outfit aggregate: a named deck + cosmetic loadout built from owned cards.
CREATE TABLE outfits (
    id            TEXT PRIMARY KEY,
    player_id     TEXT NOT NULL,
    collection_id TEXT NOT NULL REFERENCES player_collections (id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    version       BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_outfits_collection ON outfits (collection_id);
CREATE TRIGGER trg_outfits_updated
    BEFORE UPDATE ON outfits
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Cards included in an Outfit, with the cosmetic skin equipped onto each. A
-- card appears at most once per outfit (composite PK). The domain enforces
-- "card must be present (qty >= 1) in the collection" server-side before insert.
CREATE TABLE outfit_cards (
    outfit_id          TEXT NOT NULL REFERENCES outfits (id) ON DELETE CASCADE,
    card_definition_id TEXT NOT NULL REFERENCES card_definitions (id) ON DELETE RESTRICT,
    cosmetic_ref       TEXT,
    PRIMARY KEY (outfit_id, card_definition_id)
);
