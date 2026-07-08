ALTER TABLE trigger_events ADD COLUMN plan TEXT;
ALTER TABLE trigger_events ADD COLUMN subscriber TEXT;
ALTER TABLE trigger_events ADD COLUMN mint TEXT;
ALTER TABLE trigger_events ADD COLUMN amount INTEGER;
ALTER TABLE trigger_events ADD COLUMN period_start_s INTEGER;
ALTER TABLE trigger_events ADD COLUMN period_end_ts INTEGER;