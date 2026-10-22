-- HealthierYou local store, schema v2: tracker and coaching engine (slice V1-2).
-- Forward-only and additive ("expand"): a v1 build can still open a v2 database.
-- Once released, this file must never change: a checksum test guards it.

-- The coaching engine's state, one row. `revision` is the durable order of engine steps:
-- every step commits with `revision = old + 1` in the same transaction as the events it
-- produced, and a writer holding an older revision is refused (compare-and-set in code,
-- and this trigger as a second line of defence). Wall-clock time never orders steps.
CREATE TABLE coach_state (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    revision       INTEGER NOT NULL CHECK (revision >= 1),
    format         INTEGER NOT NULL CHECK (format >= 1),
    engine_time_ms INTEGER NOT NULL CHECK (engine_time_ms >= 0),
    saved_at       INTEGER NOT NULL,
    state          TEXT NOT NULL CHECK (json_valid(state) AND json_type(state) = 'object')
) STRICT;

CREATE TRIGGER coach_state_revision_increases BEFORE UPDATE ON coach_state
WHEN NEW.revision <> OLD.revision + 1
BEGIN
    SELECT RAISE(ABORT, 'coach state revision must increase by one');
END;

CREATE TRIGGER coach_state_engine_time_monotonic BEFORE UPDATE ON coach_state
WHEN NEW.engine_time_ms < OLD.engine_time_ms
BEGIN
    SELECT RAISE(ABORT, 'coach engine time never goes backwards');
END;

-- Per local date tallies from the activity tracker (architecture 4.3 "Reported as").
-- Durations are whole seconds and are estimates: no input is never proof of a break or of
-- movement. No within-day timing is kept. A missing row means no data, never zero.
CREATE TABLE activity_day (
    local_date        TEXT PRIMARY KEY CHECK (local_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    pc_active_s       INTEGER NOT NULL CHECK (pc_active_s >= 0),
    work_active_s     INTEGER NOT NULL CHECK (work_active_s >= 0),
    work_quiet_s      INTEGER NOT NULL CHECK (work_quiet_s >= 0),
    unknown_s         INTEGER NOT NULL CHECK (unknown_s >= 0),
    away_s            INTEGER NOT NULL CHECK (away_s >= 0),
    confirmed_breaks  INTEGER NOT NULL CHECK (confirmed_breaks >= 0),
    longest_block_s   INTEGER NOT NULL CHECK (longest_block_s >= 0),
    focus_sessions    INTEGER NOT NULL CHECK (focus_sessions >= 0),
    pomodoro_rounds   INTEGER NOT NULL CHECK (pomodoro_rounds >= 0),
    revision          INTEGER NOT NULL CHECK (revision >= 1),
    updated_at        INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TRIGGER activity_day_revision_increases BEFORE UPDATE ON activity_day
WHEN NEW.revision <= OLD.revision
BEGIN
    SELECT RAISE(ABORT, 'activity tally revision must increase');
END;

-- One offer per reminder id, and at most one response to it. The response is the
-- activity's outcome for that offer, so no ordering between responses is ever needed.
CREATE UNIQUE INDEX events_one_offer_per_reminder
    ON events (json_extract(payload, '$.reminder_id'))
    WHERE type = 'reminder.offered';

CREATE UNIQUE INDEX events_one_response_per_reminder
    ON events (json_extract(payload, '$.reminder_id'))
    WHERE type = 'reminder.response';

-- A response must answer an offer of the same activity that is already stored.
CREATE TRIGGER events_response_needs_offer BEFORE INSERT ON events
WHEN NEW.type = 'reminder.response' AND NOT EXISTS (
    SELECT 1 FROM events o
    WHERE o.type = 'reminder.offered'
      AND json_extract(o.payload, '$.reminder_id') = json_extract(NEW.payload, '$.reminder_id')
      AND json_extract(o.payload, '$.activity') = json_extract(NEW.payload, '$.activity')
)
BEGIN
    SELECT RAISE(ABORT, 'a reminder response must answer a stored offer of the same activity');
END;
