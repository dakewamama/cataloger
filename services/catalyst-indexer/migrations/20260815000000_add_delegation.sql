ALTER TABLE trigger_events ADD COLUMN delegation TEXT;
CREATE INDEX idx_trigger_events_delegation ON trigger_events(delegation);