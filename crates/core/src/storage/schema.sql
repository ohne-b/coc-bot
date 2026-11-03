CREATE TABLE sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mode TEXT NOT NULL CHECK (mode IN ('observe', 'farming', 'walls')),
    started_at_ms INTEGER NOT NULL CHECK (started_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= 0),
    state TEXT NOT NULL CHECK (state IN ('running', 'paused', 'stopped', 'completed', 'failed', 'interrupted')),
    active_ms INTEGER NOT NULL CHECK (active_ms >= 0)
) STRICT;
CREATE UNIQUE INDEX one_active_session ON sessions ((1)) WHERE state IN ('running', 'paused');
CREATE TABLE resources (
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    village TEXT NOT NULL,
    resource TEXT NOT NULL,
    checked_at_ms INTEGER NOT NULL,
    latest TEXT NOT NULL CHECK (json_valid(latest)),
    last_known TEXT CHECK (last_known IS NULL OR json_valid(last_known)),
    observed_at_ms INTEGER,
    PRIMARY KEY (session_id, village, resource)
) STRICT;
CREATE TABLE attacks (
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    village TEXT NOT NULL,
    gold INTEGER CHECK (gold >= 0),
    elixir INTEGER CHECK (elixir >= 0),
    dark INTEGER CHECK (dark >= 0),
    data TEXT NOT NULL CHECK (json_valid(data)),
    PRIMARY KEY (session_id, ordinal)
) STRICT;
CREATE TABLE walls (
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    village TEXT NOT NULL,
    resource TEXT NOT NULL,
    cost INTEGER NOT NULL CHECK (cost > 0),
    data TEXT NOT NULL CHECK (json_valid(data)),
    PRIMARY KEY (session_id, ordinal)
) STRICT;
CREATE TABLE activity (
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    data TEXT NOT NULL CHECK (json_valid(data)),
    PRIMARY KEY (session_id, sequence)
) STRICT;
PRAGMA application_id = 1129268034;
PRAGMA user_version = 1;
