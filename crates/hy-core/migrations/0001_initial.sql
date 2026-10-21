-- HealthierYou local store, schema v1 (architecture 6.1).
-- Forward-only. Once released, this file must never change: a checksum test guards it.
-- Times are Unix milliseconds (UTC). Local dates are 'YYYY-MM-DD' text in the zone the
-- row records, so they sort and compare as dates.
-- Not in v1: server_link (certificate pin, device token) and the signed head receipt.
-- They arrive with V1-5, after the pairing security gate.

CREATE TABLE store_meta (
    key   TEXT PRIMARY KEY CHECK (length(key) BETWEEN 1 AND 64),
    value TEXT NOT NULL
) STRICT;

-- Append-only event log (90-day retention).
CREATE TABLE events (
    id             TEXT PRIMARY KEY CHECK (length(id) = 36),
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    type           TEXT NOT NULL CHECK (type GLOB '[a-z]*.[a-z]*' AND length(type) <= 64),
    occurred_at    INTEGER NOT NULL,
    recorded_at    INTEGER NOT NULL,
    zone           TEXT NOT NULL CHECK (length(zone) BETWEEN 1 AND 64),
    local_date     TEXT NOT NULL CHECK (local_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    source         TEXT NOT NULL CHECK (source IN ('manual', 'inferred', 'system', 'imported')),
    source_ref     TEXT CHECK (source_ref IS NULL OR source = 'imported'),
    status         TEXT NOT NULL CHECK (status IN ('self_reported', 'measured', 'estimated', 'no_reading')),
    confidence     REAL CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    supersedes     TEXT REFERENCES events (id) ON DELETE SET NULL CHECK (supersedes IS NULL OR supersedes <> id),
    origin_device  TEXT NOT NULL CHECK (length(origin_device) = 36),
    payload        TEXT NOT NULL CHECK (json_valid(payload) AND json_type(payload) = 'object')
) STRICT;

-- An event is corrected or retracted at most once; a correction can itself be corrected.
CREATE UNIQUE INDEX events_superseded_once ON events (supersedes) WHERE supersedes IS NOT NULL;
CREATE INDEX events_by_day ON events (local_date, type, occurred_at);

-- Append-only: the only permitted update is the foreign key clearing `supersedes`
-- when retention removes the older event.
CREATE TRIGGER events_append_only BEFORE UPDATE ON events
WHEN NOT (
    NEW.supersedes IS NULL
    AND NEW.id = OLD.id AND NEW.schema_version = OLD.schema_version AND NEW.type = OLD.type
    AND NEW.occurred_at = OLD.occurred_at AND NEW.recorded_at = OLD.recorded_at
    AND NEW.zone = OLD.zone AND NEW.local_date = OLD.local_date AND NEW.source = OLD.source
    AND NEW.source_ref IS OLD.source_ref AND NEW.status = OLD.status
    AND NEW.confidence IS OLD.confidence AND NEW.origin_device = OLD.origin_device
    AND NEW.payload = OLD.payload
)
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;

-- Per category and origin device. A missing row means no data, never zero.
CREATE TABLE daily_summary (
    category      TEXT NOT NULL CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    origin_device TEXT NOT NULL CHECK (length(origin_device) = 36),
    local_date    TEXT NOT NULL CHECK (local_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    revision      INTEGER NOT NULL CHECK (revision >= 1),
    coverage      REAL CHECK (coverage IS NULL OR (coverage >= 0.0 AND coverage <= 1.0)),
    payload       TEXT NOT NULL CHECK (json_valid(payload) AND json_type(payload) = 'object'),
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY (category, origin_device, local_date)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER daily_summary_revision_increases BEFORE UPDATE ON daily_summary
WHEN NEW.revision <= OLD.revision
BEGIN
    SELECT RAISE(ABORT, 'summary revision must increase');
END;

CREATE TABLE monthly_summary (
    category      TEXT NOT NULL CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    origin_device TEXT NOT NULL CHECK (length(origin_device) = 36),
    month         TEXT NOT NULL CHECK (month GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]'),
    revision      INTEGER NOT NULL CHECK (revision >= 1),
    coverage      REAL CHECK (coverage IS NULL OR (coverage >= 0.0 AND coverage <= 1.0)),
    payload       TEXT NOT NULL CHECK (json_valid(payload) AND json_type(payload) = 'object'),
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY (category, origin_device, month)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER monthly_summary_revision_increases BEFORE UPDATE ON monthly_summary
WHEN NEW.revision <= OLD.revision
BEGIN
    SELECT RAISE(ABORT, 'summary revision must increase');
END;

-- Read-only history from a backup import or a server restore (architecture 5.4).
-- Never read by the outbox, so it is never re-uploaded as this PC's totals.
CREATE TABLE imported_summary (
    category      TEXT NOT NULL CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    origin_device TEXT NOT NULL CHECK (length(origin_device) = 36),
    local_date    TEXT NOT NULL CHECK (local_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    revision      INTEGER NOT NULL CHECK (revision >= 1),
    coverage      REAL CHECK (coverage IS NULL OR (coverage >= 0.0 AND coverage <= 1.0)),
    payload       TEXT NOT NULL CHECK (json_valid(payload) AND json_type(payload) = 'object'),
    provenance    TEXT NOT NULL CHECK (provenance IN ('backup_import', 'server_restore')),
    imported_at   INTEGER NOT NULL,
    PRIMARY KEY (category, origin_device, local_date)
) STRICT, WITHOUT ROWID;

-- Goal values with effective-from dates: a week is judged against that week's goal.
CREATE TABLE goals_history (
    kind           TEXT NOT NULL CHECK (kind IN ('water_daily_ml')),
    effective_from TEXT NOT NULL CHECK (effective_from GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    value          INTEGER NOT NULL CHECK (value > 0),
    recorded_at    INTEGER NOT NULL,
    PRIMARY KEY (kind, effective_from)
) STRICT, WITHOUT ROWID;

CREATE TABLE settings (
    key        TEXT PRIMARY KEY CHECK (length(key) BETWEEN 1 AND 64),
    value      TEXT NOT NULL CHECK (json_valid(value)),
    updated_at INTEGER NOT NULL
) STRICT;

-- The user's pet and edits (wireframe W3). One row.
CREATE TABLE pet_profile (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    pet_id         TEXT NOT NULL CHECK (length(pet_id) BETWEEN 1 AND 64),
    display_name   TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 20),
    scale_pct      INTEGER NOT NULL CHECK (scale_pct BETWEEN 75 AND 150),
    colour_variant INTEGER NOT NULL CHECK (colour_variant BETWEEN 0 AND 2),
    edge           TEXT NOT NULL CHECK (edge IN ('left', 'right', 'free')),
    pos_x          INTEGER,
    pos_y          INTEGER,
    motion         INTEGER NOT NULL CHECK (motion IN (0, 1)),
    tone           TEXT NOT NULL CHECK (tone IN ('gentle', 'plain', 'cheerful')),
    updated_at     INTEGER NOT NULL,
    CHECK ((pos_x IS NULL) = (pos_y IS NULL))
) STRICT;

-- Local copy of the sharing policy (architecture 5.3). `enabled` is 1 only after the
-- server accepted an enable; every category starts off.
CREATE TABLE sharing_policy (
    category   TEXT PRIMARY KEY CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    enabled    INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    share_from TEXT CHECK (share_from IS NULL OR share_from GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    seq        INTEGER NOT NULL DEFAULT 0 CHECK (seq >= 0),
    updated_at INTEGER NOT NULL,
    CHECK (enabled = 0 OR share_from IS NOT NULL)
) STRICT;

INSERT INTO sharing_policy (category, enabled, share_from, seq, updated_at) VALUES
    ('hydration', 0, NULL, 0, 0),
    ('outcomes', 0, NULL, 0, 0),
    ('work_session', 0, NULL, 0, 0),
    ('phone_reports', 0, NULL, 0, 0),
    ('posture_summary', 0, NULL, 0, 0);

CREATE TRIGGER sharing_policy_no_delete BEFORE DELETE ON sharing_policy
BEGIN
    SELECT RAISE(ABORT, 'sharing policy rows are permanent');
END;

CREATE TABLE policy_ops (
    op_id      TEXT PRIMARY KEY CHECK (length(op_id) = 36),
    category   TEXT NOT NULL CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    kind       TEXT NOT NULL CHECK (kind IN ('enable', 'disable')),
    base_seq   INTEGER NOT NULL CHECK (base_seq >= 0),
    state      TEXT NOT NULL CHECK (state IN ('pending', 'accepted', 'rejected_stale', 'pending_recovery')),
    server_seq INTEGER CHECK (server_seq IS NULL OR server_seq >= 1),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (state <> 'accepted' OR server_seq IS NOT NULL)
) STRICT;

-- Deletion tombstones: dates in a tombstone are permanently non-uploadable for that
-- category. No health payload. Rows can never be deleted and their range never changes.
CREATE TABLE tombstones (
    request_id TEXT PRIMARY KEY CHECK (length(request_id) = 36),
    category   TEXT NOT NULL CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    from_date  TEXT NOT NULL CHECK (from_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    to_date    TEXT NOT NULL CHECK (to_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    state      TEXT NOT NULL CHECK (state IN ('pending', 'acknowledged', 'pending_recovery')),
    server_seq INTEGER CHECK (server_seq IS NULL OR server_seq >= 1),
    created_at INTEGER NOT NULL,
    CHECK (from_date <= to_date)
) STRICT;

CREATE INDEX tombstones_by_category ON tombstones (category, from_date, to_date);

CREATE TRIGGER tombstones_permanent BEFORE DELETE ON tombstones
BEGIN
    SELECT RAISE(ABORT, 'tombstones are permanent');
END;

CREATE TRIGGER tombstones_range_fixed BEFORE UPDATE OF request_id, category, from_date, to_date ON tombstones
BEGIN
    SELECT RAISE(ABORT, 'tombstone range cannot change');
END;

CREATE TABLE consent_log (
    id             INTEGER PRIMARY KEY,
    at             INTEGER NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN ('notice_shown', 'enable_requested', 'enabled', 'disabled', 'deletion_requested', 'stale_enable_rejected')),
    category       TEXT CHECK (category IS NULL OR category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    notice_version TEXT CHECK (notice_version IS NULL OR length(notice_version) BETWEEN 1 AND 32),
    op_ref         TEXT CHECK (op_ref IS NULL OR length(op_ref) = 36)
) STRICT;

CREATE TRIGGER consent_log_append_only BEFORE UPDATE ON consent_log
BEGIN
    SELECT RAISE(ABORT, 'consent log is append-only');
END;

-- Upload queue. Rows are never re-stamped: they are inserted or deleted, never updated.
CREATE TABLE outbox (
    id            INTEGER PRIMARY KEY,
    category      TEXT NOT NULL CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    origin_device TEXT NOT NULL CHECK (length(origin_device) = 36),
    local_date    TEXT NOT NULL CHECK (local_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    revision      INTEGER NOT NULL CHECK (revision >= 1),
    payload       TEXT NOT NULL CHECK (json_valid(payload) AND json_type(payload) = 'object'),
    created_at    INTEGER NOT NULL,
    UNIQUE (category, origin_device, local_date, revision)
) STRICT;

-- Never build a record for a blocked date (architecture 5.3, device side).
CREATE TRIGGER outbox_guard BEFORE INSERT ON outbox
BEGIN
    SELECT RAISE(ABORT, 'upload_blocked: category off')
    WHERE NOT EXISTS (SELECT 1 FROM sharing_policy WHERE category = NEW.category AND enabled = 1);
    SELECT RAISE(ABORT, 'upload_blocked: before share_from')
    WHERE NEW.local_date < (SELECT share_from FROM sharing_policy WHERE category = NEW.category);
    SELECT RAISE(ABORT, 'upload_blocked: tombstoned')
    WHERE EXISTS (
        SELECT 1 FROM tombstones
        WHERE category = NEW.category AND NEW.local_date BETWEEN from_date AND to_date
    );
    SELECT RAISE(ABORT, 'upload_blocked: not recorded by this PC')
    WHERE NEW.origin_device IS NOT (SELECT value FROM store_meta WHERE key = 'device_id');
END;

CREATE TRIGGER outbox_never_restamped BEFORE UPDATE ON outbox
BEGIN
    SELECT RAISE(ABORT, 'outbox rows are never re-stamped');
END;

-- A disable purges the category's queue at once, even offline.
CREATE TRIGGER sharing_disable_purges_outbox AFTER UPDATE OF enabled ON sharing_policy
WHEN NEW.enabled = 0
BEGIN
    DELETE FROM outbox WHERE category = NEW.category;
END;

-- A later share_from discards queued records dated before it.
CREATE TRIGGER sharing_share_from_purges_outbox AFTER UPDATE OF share_from ON sharing_policy
WHEN NEW.share_from IS NOT NULL
BEGIN
    DELETE FROM outbox WHERE category = NEW.category AND local_date < NEW.share_from;
END;

-- A tombstone discards queued records for its dates in the same transaction.
CREATE TRIGGER tombstone_purges_outbox AFTER INSERT ON tombstones
BEGIN
    DELETE FROM outbox
    WHERE category = NEW.category AND local_date BETWEEN NEW.from_date AND NEW.to_date;
END;
