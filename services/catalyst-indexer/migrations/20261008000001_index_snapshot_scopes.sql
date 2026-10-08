ALTER TABLE authority_snapshots ADD COLUMN scope_id TEXT;
ALTER TABLE authority_snapshots ADD COLUMN observed_slot BLOB
    CHECK ((scope_id IS NULL AND observed_slot IS NULL) OR
           (scope_id IS NOT NULL AND typeof(observed_slot) = 'blob' AND length(observed_slot) = 8));

CREATE INDEX authority_snapshots_scope_slot ON authority_snapshots (scope_id, observed_slot DESC, id);

CREATE TRIGGER authority_snapshots_require_position
BEFORE INSERT ON authority_snapshots
WHEN NEW.scope_id IS NULL OR NEW.observed_slot IS NULL
BEGIN
    SELECT RAISE(ABORT, 'observation scope and slot required');
END;
