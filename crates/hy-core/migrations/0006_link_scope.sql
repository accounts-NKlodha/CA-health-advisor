-- V1-5b review: everything a server link owns is bound to that link, so nothing queued
-- or remembered for one server or user reaches another after an unpair and re-pair.

-- The link (this PC's device_ref on that server) each operation was made under. Older
-- rows have none and are never sent: they predate any link that could have taken them.
ALTER TABLE policy_ops ADD COLUMN link_device TEXT CHECK (link_device IS NULL OR length(link_device) = 36);
ALTER TABLE tombstones ADD COLUMN link_device TEXT CHECK (link_device IS NULL OR length(link_device) = 36);

-- Removing the link (unpair, revocation, a backup import) also cancels its unsent policy
-- operations, forgets its policy sequence and share dates, and drops its sync status,
-- notices and upload revisions. Personal data, tombstones (which keep blocking local
-- re-upload) and the consent log stay.
DROP TRIGGER server_link_removal_purges;

CREATE TRIGGER server_link_removal_purges AFTER DELETE ON server_link
BEGIN
    DELETE FROM head_receipt;
    DELETE FROM paired_devices;
    DELETE FROM outbox;
    DELETE FROM policy_ops WHERE state IN ('pending', 'pending_recovery');
    UPDATE sharing_policy SET enabled = 0, share_from = NULL, seq = 0;
    DELETE FROM settings
    WHERE key IN ('server_status', 'server_notices', 'share_revisions', 'server_regression_suppress');
END;
