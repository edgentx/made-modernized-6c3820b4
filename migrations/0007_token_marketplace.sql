-- Token-and-marketplace bounded context: on-chain card tokens, their peer-to-
-- peer $MADE listings, and the per-season emission pool that funds rewards.
--
-- Two more ledger-style invariants are enforced in the schema here:
--   * Unique serials       -> card_tokens.serial_number is UNIQUE
--   * Non-negative pool balance that never exceeds what was minted
--       -> emission_pools CHECK (0 <= remaining_balance <= starting_balance)

-- CardToken aggregate: a single mintable ERC-1155 card token.
CREATE TABLE card_tokens (
    id             TEXT PRIMARY KEY,
    token_id       TEXT NOT NULL,                 -- on-chain ERC-1155 id
    serial_number  TEXT UNIQUE,                   -- serialized editions carry a unique, non-reusable serial
    metadata_uri   TEXT,                          -- staged IPFS metadata record
    owner_wallet   TEXT,                          -- linked custodial / WalletConnect wallet
    minted         BOOLEAN NOT NULL DEFAULT FALSE,
    version        BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_card_tokens_token_id ON card_tokens (token_id);
CREATE INDEX idx_card_tokens_owner ON card_tokens (owner_wallet);
CREATE TRIGGER trg_card_tokens_updated
    BEFORE UPDATE ON card_tokens
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- MarketplaceListing aggregate: an owned token listed for sale in $MADE.
CREATE TABLE marketplace_listings (
    id            TEXT PRIMARY KEY,
    card_token_id TEXT NOT NULL REFERENCES card_tokens (id) ON DELETE CASCADE,
    seller_id     TEXT NOT NULL,
    price_made    BIGINT NOT NULL CHECK (price_made > 0),
    status        TEXT NOT NULL DEFAULT 'Open'
                      CHECK (status IN ('Open', 'Cancelled', 'Purchased', 'Settled')),
    jurisdiction  TEXT,
    buyer_id      TEXT,
    version       BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_marketplace_listings_token ON marketplace_listings (card_token_id);
CREATE INDEX idx_marketplace_listings_open ON marketplace_listings (status) WHERE status = 'Open';
CREATE TRIGGER trg_marketplace_listings_updated
    BEFORE UPDATE ON marketplace_listings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- EmissionPool aggregate: the per-season $MADE reward pool. One pool per season
-- (UNIQUE season_id). The balance ledger invariant lives in the CHECK: the
-- remaining balance is non-negative and can never exceed the minted starting
-- balance (solvency ceiling).
CREATE TABLE emission_pools (
    id                TEXT PRIMARY KEY,
    season_id         TEXT NOT NULL UNIQUE REFERENCES seasons (id) ON DELETE CASCADE,
    starting_balance  BIGINT NOT NULL CHECK (starting_balance >= 0),
    remaining_balance BIGINT NOT NULL CHECK (remaining_balance >= 0),
    low_pool_warned   BOOLEAN NOT NULL DEFAULT FALSE,
    version           BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Solvency ceiling: the pool can never hold more than it opened with.
    CONSTRAINT emission_pools_solvent
        CHECK (remaining_balance <= starting_balance)
);
CREATE TRIGGER trg_emission_pools_updated
    BEFORE UPDATE ON emission_pools
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Append-only ledger of individual emissions drawn from a pool. Each row is a
-- positive draw to a recipient; the running pool balance is kept on
-- emission_pools above. Useful as the audit trail behind the balance.
CREATE TABLE emission_ledger (
    id           TEXT PRIMARY KEY,
    pool_id      TEXT NOT NULL REFERENCES emission_pools (id) ON DELETE CASCADE,
    recipient_id TEXT NOT NULL,
    amount       BIGINT NOT NULL CHECK (amount > 0),
    emitted_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_emission_ledger_pool ON emission_ledger (pool_id, emitted_at);
