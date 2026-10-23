-- HealthierYou office server database, schema v2: sync (V1-5 contract 7.3, 7.4, 8, 9
-- and 10). Forward-only; frozen once released.
-- Summary payloads are stored unencrypted, as disclosed (contract 10). Nothing here
-- holds a token, a slip secret or free text.

-- Recovery hold per user (contract 9): when it started, the user's chain tail in the
-- restored snapshot, and whether the administrator witnessed the user's device list.
ALTER TABLE users ADD COLUMN hold_started_at INTEGER;
ALTER TABLE users ADD COLUMN snapshot_hwm INTEGER NOT NULL DEFAULT 0 CHECK (snapshot_hwm >= 0);
ALTER TABLE users ADD COLUMN witnessed INTEGER NOT NULL DEFAULT 0 CHECK (witnessed IN (0, 1));

-- Receipt echoes (contract 7.4): the highest valid receipt seq a device has shown, and
-- when it last regressed (a second regression within 24 h revokes it).
ALTER TABLE devices ADD COLUMN max_echoed_seq INTEGER NOT NULL DEFAULT 0 CHECK (max_echoed_seq >= 0);
ALTER TABLE devices ADD COLUMN regressed_at INTEGER;

-- The user's policy per category (contract 8.3). `seq` is the latest policy or deletion
-- entry for the category, `off_seq` the latest disable or deletion: an enable needs
-- base_seq >= off_seq.
CREATE TABLE policy (
    user_id    BLOB NOT NULL REFERENCES users (user_id),
    category   TEXT NOT NULL CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    enabled    INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    share_from TEXT CHECK (share_from IS NULL OR share_from GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    seq        INTEGER NOT NULL CHECK (seq >= 0),
    off_seq    INTEGER NOT NULL CHECK (off_seq >= 0),
    CHECK (enabled = 0 OR share_from IS NOT NULL),
    PRIMARY KEY (user_id, category)
) STRICT, WITHOUT ROWID;

-- Deletion tombstones (contract 8.4): permanent, no payload. `seq` is NULL while the
-- deletion waits for release from recovery hold.
CREATE TABLE tombstones (
    request_id BLOB PRIMARY KEY CHECK (length(request_id) = 16),
    user_id    BLOB NOT NULL REFERENCES users (user_id),
    category   TEXT NOT NULL CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    from_date  TEXT NOT NULL CHECK (from_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    to_date    TEXT NOT NULL CHECK (to_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    seq        INTEGER CHECK (seq IS NULL OR seq >= 1),
    created_at INTEGER NOT NULL,
    CHECK (from_date <= to_date)
) STRICT;

CREATE INDEX tombstones_by_user ON tombstones (user_id, category, from_date, to_date);

CREATE TRIGGER tombstones_permanent BEFORE DELETE ON tombstones
BEGIN
    SELECT RAISE(ABORT, 'tombstones are permanent');
END;

CREATE TRIGGER tombstones_range_fixed BEFORE UPDATE OF request_id, user_id, category, from_date, to_date ON tombstones
BEGIN
    SELECT RAISE(ABORT, 'tombstone range cannot change');
END;

-- Daily summaries, keyed by origin device (contract 8.6). Kept 90 days.
CREATE TABLE summaries (
    user_id       BLOB NOT NULL REFERENCES users (user_id),
    category      TEXT NOT NULL CHECK (category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    origin_device BLOB NOT NULL CHECK (length(origin_device) = 16),
    local_date    TEXT NOT NULL CHECK (local_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    revision      INTEGER NOT NULL CHECK (revision >= 1),
    payload       TEXT NOT NULL CHECK (json_valid(payload) AND json_type(payload) = 'object' AND length(payload) <= 1024),
    device_ref    BLOB NOT NULL CHECK (length(device_ref) = 16),
    received_at   INTEGER NOT NULL,
    PRIMARY KEY (user_id, category, origin_device, local_date)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER summaries_revision_increases BEFORE UPDATE ON summaries
WHEN NEW.revision <= OLD.revision
BEGIN
    SELECT RAISE(ABORT, 'summary revision must increase');
END;

-- Control operations already answered, by id (idempotency, contract 8.1).
CREATE TABLE control_ops (
    op_id      BLOB PRIMARY KEY CHECK (length(op_id) = 16),
    user_id    BLOB NOT NULL REFERENCES users (user_id),
    outcome    TEXT NOT NULL CHECK (json_valid(outcome)),
    created_at INTEGER NOT NULL
) STRICT;

-- Disables and deletions accepted during recovery hold, sequenced on release (contract
-- 9). Their effects were applied at once.
CREATE TABLE pending_ops (
    id          INTEGER PRIMARY KEY,
    user_id     BLOB NOT NULL REFERENCES users (user_id),
    device_ref  BLOB NOT NULL CHECK (length(device_ref) = 16),
    op          TEXT NOT NULL CHECK (json_valid(op)),
    received_at INTEGER NOT NULL
) STRICT;

-- After the mirror fsync and before the acknowledgement, the user's mirrored seq
-- (contract 7.3, mirror_hwm).
CREATE TABLE mirror_hwm (
    user_id BLOB PRIMARY KEY REFERENCES users (user_id),
    seq     INTEGER NOT NULL CHECK (seq >= 0)
) STRICT;

-- Release evidence (contract 9): a device's receipt, presented with its own token.
CREATE TABLE evidence (
    user_id     BLOB NOT NULL REFERENCES users (user_id),
    device_ref  BLOB NOT NULL REFERENCES devices (device_ref),
    seq         INTEGER NOT NULL CHECK (seq >= 0),
    recorded_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, device_ref)
) STRICT, WITHOUT ROWID;

-- Encrypted backups written (contract 10).
CREATE TABLE backups (
    file        TEXT PRIMARY KEY CHECK (length(file) BETWEEN 1 AND 128),
    snapshot_id BLOB NOT NULL CHECK (length(snapshot_id) = 16),
    created_at  INTEGER NOT NULL,
    weekly      INTEGER NOT NULL CHECK (weekly IN (0, 1))
) STRICT;

-- The audit log gains the sync and recovery events. SQLite cannot change a CHECK, so the
-- table is rebuilt with its rows.
CREATE TABLE audit_v2 (
    id         INTEGER PRIMARY KEY,
    at         INTEGER NOT NULL,
    event      TEXT NOT NULL CHECK (event IN (
        'user_created', 'slip_issued', 'slip_redeemed', 'slip_refused', 'pair_rate_limited',
        'device_confirmed', 'device_rejected', 'device_revoked', 'device_unpaired',
        'device_expired', 'device_superseded', 'admin_login', 'admin_login_failed',
        'admin_logout', 'pending_rejected_for_user',
        'device_regressed', 'history_read', 'receipt_recorded', 'receipt_refused',
        'recovery_hold', 'user_released', 'user_discarded', 'global_discard',
        'users_witnessed', 'backup_written', 'restored'
    )),
    user_id    BLOB CHECK (user_id IS NULL OR length(user_id) = 16),
    device_ref BLOB CHECK (device_ref IS NULL OR length(device_ref) = 16),
    net24      TEXT CHECK (net24 IS NULL OR length(net24) BETWEEN 1 AND 48)
) STRICT;

INSERT INTO audit_v2 (id, at, event, user_id, device_ref, net24)
    SELECT id, at, event, user_id, device_ref, net24 FROM audit;
DROP TABLE audit;
ALTER TABLE audit_v2 RENAME TO audit;
CREATE INDEX audit_by_time ON audit (at);
