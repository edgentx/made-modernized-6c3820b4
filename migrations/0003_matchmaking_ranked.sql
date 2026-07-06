-- Matchmaking-and-ranked bounded context: seasons, the matchmaking queue, and
-- per-player competitive standings.
--
-- A Season is the top-level container; a MatchmakingTicket is a queue entry
-- scoped to a season; a RankedStanding is one player's competitive record for a
-- season. The leaderboard query ("top standings in a season, by rating") is the
-- hot read path, so it gets a dedicated covering index verified via EXPLAIN.

-- Season aggregate: a competitive season window.
CREATE TABLE seasons (
    id         TEXT PRIMARY KEY,
    number     INTEGER NOT NULL UNIQUE CHECK (number >= 1),
    name       TEXT NOT NULL,
    opened_at  TIMESTAMPTZ,
    closed_at  TIMESTAMPTZ,
    version    BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A season cannot close before it opens.
    CONSTRAINT seasons_window_ordered
        CHECK (closed_at IS NULL OR opened_at IS NULL OR closed_at >= opened_at)
);
CREATE TRIGGER trg_seasons_updated
    BEFORE UPDATE ON seasons
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- MatchmakingTicket aggregate: a player's queue entry within a season.
CREATE TABLE matchmaking_tickets (
    id          TEXT PRIMARY KEY,
    player_id   TEXT NOT NULL,
    season_id   TEXT NOT NULL REFERENCES seasons (id) ON DELETE CASCADE,
    status      TEXT NOT NULL DEFAULT 'Queued'
                    CHECK (status IN ('Queued', 'Matched', 'Cancelled', 'Expired')),
    enqueued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version     BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A player holds at most one live ticket per season.
    CONSTRAINT matchmaking_tickets_one_live_per_player
        UNIQUE (season_id, player_id)
);
CREATE INDEX idx_matchmaking_tickets_queue ON matchmaking_tickets (season_id, status, enqueued_at);
CREATE TRIGGER trg_matchmaking_tickets_updated
    BEFORE UPDATE ON matchmaking_tickets
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- RankedStanding aggregate: one player's competitive record for a season.
-- Carries the hidden Glicko-2 skill estimate (rating/rd/volatility) plus the
-- visible ladder rank (tier + stars) with anti-tilt floor protection.
CREATE TABLE ranked_standings (
    id             TEXT PRIMARY KEY,
    player_id      TEXT NOT NULL,
    season_id      TEXT NOT NULL REFERENCES seasons (id) ON DELETE CASCADE,
    rating         DOUBLE PRECISION NOT NULL DEFAULT 1500.0,
    rating_dev     DOUBLE PRECISION NOT NULL DEFAULT 350.0 CHECK (rating_dev >= 0),
    volatility     DOUBLE PRECISION NOT NULL DEFAULT 0.06 CHECK (volatility >= 0),
    tier           TEXT NOT NULL DEFAULT 'Block'
                       CHECK (tier IN ('Block', 'Corner', 'Contender', 'Champion', 'Legend')),
    stars          SMALLINT NOT NULL DEFAULT 0 CHECK (stars >= 0),
    floor_tier     TEXT NOT NULL DEFAULT 'Block'
                       CHECK (floor_tier IN ('Block', 'Corner', 'Contender', 'Champion', 'Legend')),
    matches_played INTEGER NOT NULL DEFAULT 0 CHECK (matches_played >= 0),
    version        BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- One standing per player per season.
    CONSTRAINT ranked_standings_one_per_player_season
        UNIQUE (season_id, player_id)
);

-- Leaderboard covering index: the standings query filters by season and orders
-- by rating descending, projecting only (player_id, tier, stars). INCLUDE-ing
-- those payload columns lets Postgres satisfy the query with an index-only scan
-- (no heap fetch). Verified via EXPLAIN in the persistence crate / CI.
CREATE INDEX idx_ranked_standings_leaderboard
    ON ranked_standings (season_id, rating DESC)
    INCLUDE (player_id, tier, stars);

CREATE TRIGGER trg_ranked_standings_updated
    BEFORE UPDATE ON ranked_standings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
