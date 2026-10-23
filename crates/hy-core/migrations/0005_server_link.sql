-- HealthierYou local store, schema v5: the office-server link (V1-5 contract section 13).
-- Forward-only. Once released, this file must never change: a checksum test guards it.
-- None of these tables is ever written to a personal backup: pairing never travels in a
-- backup file (V1-4), so an imported PC must pair again.

-- The pairing with the office server. One row while paired. The token is a DPAPI blob
-- (user scope, app entropy): only this Windows user on this PC can open it.
CREATE TABLE server_link (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    server_key_id  BLOB NOT NULL CHECK (length(server_key_id) = 32),
    pin            BLOB NOT NULL CHECK (length(pin) = 32),
    head_key       BLOB NOT NULL CHECK (length(head_key) = 32),
    host           TEXT NOT NULL CHECK (length(host) BETWEEN 1 AND 253),
    port           INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    last_ip        TEXT CHECK (last_ip IS NULL OR length(last_ip) BETWEEN 2 AND 64),
    manual_address TEXT CHECK (manual_address IS NULL OR length(manual_address) BETWEEN 1 AND 260),
    user_id        TEXT NOT NULL CHECK (length(user_id) = 36),
    device_ref     TEXT NOT NULL CHECK (length(device_ref) = 36),
    alias          TEXT NOT NULL CHECK (length(alias) BETWEEN 1 AND 40),
    token_blob     BLOB NOT NULL CHECK (length(token_blob) BETWEEN 1 AND 4096),
    confirmed      INTEGER NOT NULL DEFAULT 0 CHECK (confirmed IN (0, 1)),
    confirmed_on   TEXT CHECK (confirmed_on IS NULL OR confirmed_on GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    paired_at      INTEGER NOT NULL,
    CHECK ((confirmed = 0) = (confirmed_on IS NULL))
) STRICT;

-- The pin, keys and identity never change while paired; re-pairing is a new row.
CREATE TRIGGER server_link_identity_fixed BEFORE UPDATE OF server_key_id, pin, head_key, user_id, device_ref, token_blob, paired_at ON server_link
BEGIN
    SELECT RAISE(ABORT, 'server link identity cannot change');
END;

-- Confirmation is one-way.
CREATE TRIGGER server_link_confirmation_one_way BEFORE UPDATE OF confirmed, confirmed_on ON server_link
WHEN OLD.confirmed = 1
BEGIN
    SELECT RAISE(ABORT, 'confirmation cannot change');
END;

-- The highest valid signed head received for (server key, user): the receipt (7.4).
CREATE TABLE head_receipt (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    key_id     BLOB NOT NULL CHECK (length(key_id) = 32),
    user_id    TEXT NOT NULL CHECK (length(user_id) = 36),
    seq        INTEGER NOT NULL CHECK (seq >= 1),
    chain_hash BLOB NOT NULL CHECK (length(chain_hash) = 32),
    issued_at  INTEGER NOT NULL,
    sig        BLOB NOT NULL CHECK (length(sig) = 64)
) STRICT;

-- First written only after confirmation, for the paired server key and user.
CREATE TRIGGER head_receipt_needs_confirmation BEFORE INSERT ON head_receipt
WHEN NOT EXISTS (
    SELECT 1 FROM server_link
    WHERE confirmed = 1 AND server_key_id = NEW.key_id AND user_id = NEW.user_id
)
BEGIN
    SELECT RAISE(ABORT, 'receipt needs a confirmed link for its key and user');
END;

-- Never a lower seq, another key or another user.
CREATE TRIGGER head_receipt_never_decreases BEFORE UPDATE ON head_receipt
WHEN NEW.seq < OLD.seq OR NEW.key_id <> OLD.key_id OR NEW.user_id <> OLD.user_id
BEGIN
    SELECT RAISE(ABORT, 'receipt seq never decreases');
END;

-- Deleted only together with the link.
CREATE TRIGGER head_receipt_kept_while_paired BEFORE DELETE ON head_receipt
WHEN EXISTS (SELECT 1 FROM server_link)
BEGIN
    SELECT RAISE(ABORT, 'receipt is deleted only with the server link');
END;

-- Cache of the user's devices from the server ("PCs paired to you", new-device notices).
CREATE TABLE paired_devices (
    device_ref   TEXT PRIMARY KEY CHECK (length(device_ref) = 36),
    label        TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 40),
    paired_at    INTEGER NOT NULL,
    last_seen    INTEGER,
    confirmed    INTEGER NOT NULL CHECK (confirmed IN (0, 1)),
    notice_shown INTEGER NOT NULL DEFAULT 0 CHECK (notice_shown IN (0, 1))
) STRICT;

-- Unpairing, revocation or a re-pair needed: removing the link removes everything that
-- belongs to it in the same transaction, and nothing stays queued.
CREATE TRIGGER server_link_removal_purges AFTER DELETE ON server_link
BEGIN
    DELETE FROM head_receipt;
    DELETE FROM paired_devices;
    DELETE FROM outbox;
END;

-- The queue guard gains two conditions (8.3, 8.5): nothing is queued while this PC is not
-- paired or still pending, and nothing dated before this PC's confirmation date.
DROP TRIGGER outbox_guard;

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
    SELECT RAISE(ABORT, 'upload_blocked: not confirmed')
    WHERE NOT EXISTS (SELECT 1 FROM server_link WHERE confirmed = 1);
    SELECT RAISE(ABORT, 'upload_blocked: before confirmation')
    WHERE NEW.local_date < (SELECT confirmed_on FROM server_link);
END;

-- The consent log gains four kinds. SQLite cannot change a CHECK constraint, so the table
-- is rebuilt with every row kept and its append-only trigger re-created.
CREATE TABLE consent_log_v5 (
    id             INTEGER PRIMARY KEY,
    at             INTEGER NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN ('notice_shown', 'enable_requested', 'enabled', 'disabled', 'deletion_requested', 'stale_enable_rejected', 'paired', 'unpaired', 'remote_enable_applied', 'regression_reset')),
    category       TEXT CHECK (category IS NULL OR category IN ('hydration', 'outcomes', 'work_session', 'phone_reports', 'posture_summary')),
    notice_version TEXT CHECK (notice_version IS NULL OR length(notice_version) BETWEEN 1 AND 32),
    op_ref         TEXT CHECK (op_ref IS NULL OR length(op_ref) = 36)
) STRICT;

INSERT INTO consent_log_v5 (id, at, kind, category, notice_version, op_ref)
SELECT id, at, kind, category, notice_version, op_ref FROM consent_log ORDER BY id;

DROP TABLE consent_log;

ALTER TABLE consent_log_v5 RENAME TO consent_log;

CREATE TRIGGER consent_log_append_only BEFORE UPDATE ON consent_log
BEGIN
    SELECT RAISE(ABORT, 'consent log is append-only');
END;
