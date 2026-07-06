-- Match-play bounded context: live authoritative matches and their sealed
-- replays.
--
-- A GameSession is one authoritative match on the server; a MatchReplay is the
-- immutable, sealed record produced when that session ends (one replay per
-- session, so the FK is UNIQUE).

-- GameSession aggregate: a live authoritative match.
CREATE TABLE game_sessions (
    id             TEXT PRIMARY KEY,
    host_player_id TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'Pending'
                       CHECK (status IN ('Pending', 'Active', 'Conceded', 'Completed', 'Abandoned')),
    started_at     TIMESTAMPTZ,
    ended_at       TIMESTAMPTZ,
    version        BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_game_sessions_host ON game_sessions (host_player_id);
CREATE INDEX idx_game_sessions_status ON game_sessions (status);
CREATE TRIGGER trg_game_sessions_updated
    BEFORE UPDATE ON game_sessions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- MatchReplay aggregate: the sealed, immutable record of a finished session.
-- One replay per session -> UNIQUE foreign key. `checksum` fingerprints the
-- sealed frame stream so tampering is detectable.
CREATE TABLE match_replays (
    id         TEXT PRIMARY KEY,
    session_id TEXT NOT NULL UNIQUE REFERENCES game_sessions (id) ON DELETE CASCADE,
    sealed     BOOLEAN NOT NULL DEFAULT FALSE,
    checksum   TEXT,
    frame_uri  TEXT,
    sealed_at  TIMESTAMPTZ,
    version    BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A sealed replay must carry the checksum that fingerprints its frames.
    CONSTRAINT match_replays_sealed_has_checksum
        CHECK (NOT sealed OR checksum IS NOT NULL)
);
CREATE TRIGGER trg_match_replays_updated
    BEFORE UPDATE ON match_replays
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
