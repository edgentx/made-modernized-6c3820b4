-- Solo/AI bounded context: AI opponent profiles and a player's attempts at
-- solo missions driven by them.
--
-- AIProfile is a reusable opponent definition; a MissionAttempt references the
-- profile it was played against, so profiles migrate before attempts.

-- AIProfile aggregate: a tunable AI opponent definition.
CREATE TABLE ai_profiles (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    difficulty TEXT NOT NULL DEFAULT 'Normal'
                   CHECK (difficulty IN ('Easy', 'Normal', 'Hard', 'Nightmare')),
    params     JSONB NOT NULL DEFAULT '{}'::jsonb,
    version    BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TRIGGER trg_ai_profiles_updated
    BEFORE UPDATE ON ai_profiles
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- MissionAttempt aggregate: one player's run at a solo mission. `ai_profile_id`
-- is nullable so a scripted (non-AI) mission can still be recorded.
CREATE TABLE mission_attempts (
    id            TEXT PRIMARY KEY,
    player_id     TEXT NOT NULL,
    mission_id    TEXT NOT NULL,
    ai_profile_id TEXT REFERENCES ai_profiles (id) ON DELETE SET NULL,
    status        TEXT NOT NULL DEFAULT 'InProgress'
                      CHECK (status IN ('InProgress', 'Cleared', 'Failed', 'Abandoned')),
    score         INTEGER NOT NULL DEFAULT 0 CHECK (score >= 0),
    started_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at  TIMESTAMPTZ,
    version       BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_mission_attempts_player ON mission_attempts (player_id);
CREATE INDEX idx_mission_attempts_mission ON mission_attempts (mission_id);
CREATE TRIGGER trg_mission_attempts_updated
    BEFORE UPDATE ON mission_attempts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
