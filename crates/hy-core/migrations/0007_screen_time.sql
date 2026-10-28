-- HealthierYou local store, schema v7: screen time per day (DEC13).
-- Forward-only and additive ("expand"): a v6 build can still open a v7 database.
-- Once released, this file must never change: a checksum test guards it.

-- The time the user was at this PC on the date (the app running, the session unlocked,
-- the PC awake), whole seconds; an estimate, like the other tallies. Rows written before
-- v7 have none: no data, never zero. The weekly and monthly reviews show it per day.
ALTER TABLE activity_day ADD COLUMN screen_s INTEGER CHECK (screen_s IS NULL OR screen_s >= 0);
