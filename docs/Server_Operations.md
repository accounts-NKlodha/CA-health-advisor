# Office server: keys, backups and recovery (operator notes)

These notes are for the administrator of the HealthierYou office server. They restate the approved rules (V1-5 contract 6.2, 9 and 10; architecture 5.1 and 5.3) as operating steps. They do not change them. The admin page and the `hy-server` usage text carry the same warnings.

## 1. The server's keys

The server has three private keys in its keys folder: the TLS key (its identity, the pin every PC checks), the head-signing key, and `backup.key`. All three are protected with **machine-scope DPAPI**. Only this Windows installation on this PC can open them.

- **A copy of the keys folder is not a recovery route.** It opens only on the same Windows installation it came from. It can help if the files are deleted by mistake on that installation. It does not help if the PC or its disk fails, if Windows is reinstalled, or if the server moves to another PC. It also cannot open backups anywhere else.
- **Replacement server, Windows reinstall, lost or unopenable keys: full reset.** This is the approved route (owner decision O4). There is no key escrow and no other way:
  1. Install the server on the new or reinstalled PC and run `hy-server init` on a fresh data folder. This creates new keys and a new identity.
  2. Create the users again on the admin page.
  3. Every PC re-pairs with a new slip, and the administrator confirms each one.
  4. Each user switches their sharing categories on again. Sharing starts from that date; nothing earlier is sent again.
  5. Backups made under the old `backup.key` can never be opened. Delete them once the new server runs.
- Treat a lost or stolen server PC or disk as key theft (contract 10): do the same full reset.

Each PC keeps all of its own data through a reset. Only the server's copy of shared summaries is lost.

## 2. Backups and restore

Backups run daily at 13:00 IST while the service runs, with a catch-up after 24 hours. 14 daily and 2 weekly copies are kept, none older than 30 days. A backup opens only with this server's `backup.key`, so it can be restored only on this Windows installation. If that key is lost, every existing backup is permanently unrecoverable.

`hy-server restore --backup FILE` (service stopped) puts every user in recovery hold. A user is released only when the mirror, every one of that user's PCs and the administrator's confirmation all agree. The longest hold is 7 days. A user whose evidence is still incomplete then is discarded; time alone never releases anyone.

## 3. What a release puts back on the server

A restored database holds only what was shared up to the snapshot. After a user is released, each of that user's PCs sends its rows again:

- **Window.** The PC goes back over the server's whole retention window: 90 days before the day of release. A snapshot can be up to 30 days old and a hold can last 7 days, so the gap is at most about 37 days. That is well inside the window.
- **What is sent again.** Every day within the window that this PC had shared before, rebuilt from the PC's own current records with a higher revision. For the last 7 days, any day with a record is sent, as on every sync.
- **What is not sent.** Days the PC never shared (no backfill). Days before the category's `share_from`, or before the PC's own confirmation date. Days deleted by the user (tombstones), including deletions made after the snapshot and replayed from the mirror. Categories switched off. Rows imported from another PC's backup or restored from the server (each PC sends only its own).

Limits that remain:

- A PC can send again only what it still holds. A PC that was reset, reimaged or replaced, or whose own data was replaced by importing a backup, has no record of what it sent. Its rows from after the snapshot are not rebuilt.
- A PC that is revoked during the hold, or that re-pairs, sends under its new pairing only from its new confirmation date.
- A user who is discarded (incomplete evidence, a fork, or the 7-day limit) loses their restored server history. Their PCs re-pair and they switch sharing on again from that date (architecture 5.3).
- If the global discard path runs (mirror missing or older than the database, keys lost, users not confirmed), every PC re-pairs and every user switches sharing on again, as in section 1, steps 3 and 4.

## 4. Stopping the server

Stopping the service stops both listeners. Each one stops its open connections, then waits until every database operation a request had already started has finished. Only then does it return. The Windows service must not replace the database (for example to restore a backup) until it has fully stopped; use `hy-server restore` with the service stopped.
