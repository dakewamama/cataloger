CREATE UNIQUE INDEX idx_trigger_events_unique 
ON trigger_events(signature, discriminator, raw_data);