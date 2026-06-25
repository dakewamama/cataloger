CREATE TABLE trigger_events {
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    signature TEXT NOT NULL,
    program_id TEXT NOT NULL,
    discriminator INTEGER NOT NULL,
    raw_data BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
};

CREATE INDEX idx_trigger_events_discriminator ON trigger_events(discriminator);
CREATE INDEX idx_trigger_events_created_at ON trigger_events(created_at);