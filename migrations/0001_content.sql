-- Content bounded context: the card catalog that every other context references.
--
-- ExpansionSet groups a wave of CardDefinitions; a BossDefinition is a scripted
-- PvE encounter shipped inside an expansion. These are the *reference* tables —
-- the roots of the foreign-key graph — so they migrate first.

-- Bumps `updated_at` on any row this trigger is attached to. Defined once here
-- and reused by every mutable table across the later migrations.
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at := now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ExpansionSet aggregate: a released wave of content.
CREATE TABLE expansion_sets (
    id          TEXT PRIMARY KEY,
    code        TEXT NOT NULL UNIQUE,          -- stable short code, e.g. "BASE"
    name        TEXT NOT NULL,
    released_at TIMESTAMPTZ,
    version     BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TRIGGER trg_expansion_sets_updated
    BEFORE UPDATE ON expansion_sets
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- CardDefinition aggregate: one card in the catalog. `rarity` drives the
-- Legendary copy cap enforced later in the collection ledger.
CREATE TABLE card_definitions (
    id               TEXT PRIMARY KEY,
    expansion_set_id TEXT NOT NULL REFERENCES expansion_sets (id) ON DELETE RESTRICT,
    name             TEXT NOT NULL,
    rarity           TEXT NOT NULL CHECK (rarity IN ('Common', 'Rare', 'Epic', 'Legendary')),
    cost             INTEGER NOT NULL CHECK (cost >= 0),
    effect_ref       TEXT,
    version          BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_card_definitions_expansion ON card_definitions (expansion_set_id);
CREATE INDEX idx_card_definitions_rarity ON card_definitions (rarity);
CREATE TRIGGER trg_card_definitions_updated
    BEFORE UPDATE ON card_definitions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- BossDefinition aggregate: a scripted PvE boss with a JSONB roster/script.
CREATE TABLE boss_definitions (
    id               TEXT PRIMARY KEY,
    expansion_set_id TEXT NOT NULL REFERENCES expansion_sets (id) ON DELETE RESTRICT,
    name             TEXT NOT NULL,
    roster           JSONB NOT NULL DEFAULT '{}'::jsonb,
    version          BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_boss_definitions_expansion ON boss_definitions (expansion_set_id);
CREATE TRIGGER trg_boss_definitions_updated
    BEFORE UPDATE ON boss_definitions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
