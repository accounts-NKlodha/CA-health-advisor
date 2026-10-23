-- HealthierYou office server database (server.db), schema v1.
-- V1-5 contract: users and slips (4.2, 4.4), devices and tokens (6.1, 6.3), the per-user
-- control log (7.1), rate limits (4.4), the admin passphrase and sessions (6.1, TH15) and
-- the audit log (11). Forward-only; frozen once released.
-- Times are Unix milliseconds (UTC). Ids are random 128-bit values (BLOB, 16 bytes). No
-- table holds a health payload, a token, a slip secret or a passphrase: only hashes.

CREATE TABLE meta (
    key   TEXT PRIMARY KEY CHECK (length(key) BETWEEN 1 AND 64),
    value TEXT NOT NULL
) STRICT;

-- Users carry a display alias only: no email, phone or employee number.
CREATE TABLE users (
    user_id    BLOB PRIMARY KEY CHECK (length(user_id) = 16),
    alias      TEXT NOT NULL UNIQUE CHECK (length(alias) BETWEEN 1 AND 40),
    zone       TEXT NOT NULL DEFAULT 'Asia/Kolkata' CHECK (length(zone) BETWEEN 1 AND 64),
    hold       INTEGER NOT NULL DEFAULT 0 CHECK (hold IN (0, 1)),
    created_at INTEGER NOT NULL
) STRICT;

-- Pairing slips: the server keeps SHA-256("HY-SLIP1" ‖ secret) only. Single use: once
-- used_at is set it never changes, and a slip is never un-voided.
CREATE TABLE slips (
    slip_hash  BLOB PRIMARY KEY CHECK (length(slip_hash) = 32),
    user_id    BLOB NOT NULL REFERENCES users (user_id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at    INTEGER,
    voided     INTEGER NOT NULL DEFAULT 0 CHECK (voided IN (0, 1)),
    CHECK (expires_at > created_at)
) STRICT;

CREATE INDEX slips_by_user ON slips (user_id, used_at, voided, expires_at);

CREATE TRIGGER slips_single_use BEFORE UPDATE ON slips
WHEN NEW.slip_hash <> OLD.slip_hash OR NEW.user_id <> OLD.user_id
    OR NEW.created_at <> OLD.created_at OR NEW.expires_at <> OLD.expires_at
    OR (OLD.used_at IS NOT NULL AND NEW.used_at IS NOT OLD.used_at)
    OR (OLD.voided = 1 AND NEW.voided = 0)
BEGIN
    SELECT RAISE(ABORT, 'slips are single use');
END;

-- Devices. The token verifier is SHA-256("HY-TOK1" ‖ token); the token itself is never
-- stored. 'unconfirmed' is the pending state: no history, chain, control, upload or
-- receipt access until the administrator confirms. Revocation is final.
CREATE TABLE devices (
    device_ref     BLOB PRIMARY KEY CHECK (length(device_ref) = 16),
    user_id        BLOB NOT NULL REFERENCES users (user_id),
    origin_device  BLOB NOT NULL CHECK (length(origin_device) = 16),
    label          TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 40),
    app_version    TEXT NOT NULL CHECK (length(app_version) BETWEEN 1 AND 32),
    windows_build  TEXT NOT NULL CHECK (length(windows_build) BETWEEN 1 AND 32),
    token_verifier BLOB NOT NULL UNIQUE CHECK (length(token_verifier) = 32),
    state          TEXT NOT NULL CHECK (state IN ('unconfirmed', 'active', 'revoked')),
    created_at     INTEGER NOT NULL,
    confirmed_at   INTEGER,
    confirmed_on   TEXT CHECK (confirmed_on IS NULL OR confirmed_on GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    revoked_at     INTEGER,
    revoke_reason  TEXT CHECK (revoke_reason IS NULL OR revoke_reason IN ('admin', 'self', 'superseded', 'unconfirmed_expired', 'recovery_discard')),
    last_seen      INTEGER,
    CHECK ((state = 'revoked') = (revoked_at IS NOT NULL)),
    CHECK ((revoked_at IS NULL) = (revoke_reason IS NULL)),
    CHECK ((confirmed_at IS NULL) = (confirmed_on IS NULL)),
    CHECK (state <> 'active' OR confirmed_at IS NOT NULL)
) STRICT;

CREATE INDEX devices_by_user ON devices (user_id, state);
CREATE INDEX devices_by_origin ON devices (origin_device, state);

CREATE TRIGGER devices_identity_fixed BEFORE UPDATE OF device_ref, user_id, origin_device, token_verifier, created_at ON devices
BEGIN
    SELECT RAISE(ABORT, 'device identity cannot change');
END;

CREATE TRIGGER devices_revocation_final BEFORE UPDATE OF state ON devices
WHEN OLD.state = 'revoked' OR (OLD.state = 'active' AND NEW.state = 'unconfirmed')
BEGIN
    SELECT RAISE(ABORT, 'device state only moves forward');
END;

-- The per-user control log (contract 7.1): append-only, one hash chain per user.
CREATE TABLE control_log (
    user_id   BLOB NOT NULL REFERENCES users (user_id),
    seq       INTEGER NOT NULL CHECK (seq >= 1),
    at_ms     INTEGER NOT NULL,
    kind      INTEGER NOT NULL CHECK (kind BETWEEN 1 AND 7),
    body      BLOB NOT NULL CHECK (length(body) <= 256),
    prev_hash BLOB NOT NULL CHECK (length(prev_hash) = 32),
    hash      BLOB NOT NULL CHECK (length(hash) = 32),
    PRIMARY KEY (user_id, seq)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER control_log_no_update BEFORE UPDATE ON control_log
BEGIN
    SELECT RAISE(ABORT, 'control log is append-only');
END;

CREATE TRIGGER control_log_no_delete BEFORE DELETE ON control_log
BEGIN
    SELECT RAISE(ABORT, 'control log is append-only');
END;

-- Pairing rate-limit counters, persisted so a restart does not reset them (4.4).
CREATE TABLE rate_counters (
    scope        TEXT NOT NULL CHECK (length(scope) BETWEEN 1 AND 80),
    window_start INTEGER NOT NULL,
    count        INTEGER NOT NULL CHECK (count >= 0),
    PRIMARY KEY (scope, window_start)
) STRICT, WITHOUT ROWID;

-- The admin passphrase (Argon2id hash) and its login backoff.
CREATE TABLE admin (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    passphrase_hash TEXT NOT NULL CHECK (length(passphrase_hash) BETWEEN 1 AND 256),
    failures        INTEGER NOT NULL DEFAULT 0 CHECK (failures >= 0),
    locked_until    INTEGER NOT NULL DEFAULT 0,
    updated_at      INTEGER NOT NULL
) STRICT;

-- Admin sessions: SHA-256 of the cookie value only.
CREATE TABLE admin_sessions (
    session_hash BLOB PRIMARY KEY CHECK (length(session_hash) = 32),
    created_at   INTEGER NOT NULL,
    last_used    INTEGER NOT NULL
) STRICT;

-- The audit log (contract 11): ids and fixed event names only, never habit data, aliases,
-- secrets or full addresses. Kept 30 days.
CREATE TABLE audit (
    id         INTEGER PRIMARY KEY,
    at         INTEGER NOT NULL,
    event      TEXT NOT NULL CHECK (event IN (
        'user_created', 'slip_issued', 'slip_redeemed', 'slip_refused', 'pair_rate_limited',
        'device_confirmed', 'device_rejected', 'device_revoked', 'device_unpaired',
        'device_expired', 'device_superseded', 'admin_login', 'admin_login_failed',
        'admin_logout', 'pending_rejected_for_user'
    )),
    user_id    BLOB CHECK (user_id IS NULL OR length(user_id) = 16),
    device_ref BLOB CHECK (device_ref IS NULL OR length(device_ref) = 16),
    net24      TEXT CHECK (net24 IS NULL OR length(net24) BETWEEN 1 AND 48)
) STRICT;

CREATE INDEX audit_by_time ON audit (at);
