# V1-5 pairing and sync security proposal

Status: **proposal for Codex security sign-off**, revision 1 (23 Oct 2026). No product code. No V1-5 code lands until Codex signs off on the decisions in section 17 and the owner answers those in section 18.

**What this is.** Architecture section 5.2 says Codex must sign off the PC-to-server pairing contract before V1-5 work starts, not at the end of V1-5. This document gives Codex one complete contract to review. It covers the pairing ceremony, transport, credentials, signed head receipts, the server control log, sync rules, the recovery-hold interaction, storage at rest, logging, failure behaviour and tests. It also lists the crates to be used.

**Baseline.** Architecture revision 5 (`docs/Architecture_V1_V2.md` on the architecture branch, PR #3), which has Codex's conditional design acceptance. Also PRD 0.3 (`docs/PRD_0.3.md`, same branch), the recorded owner decisions, and the code on `main` at `aaabd21` (V1-0 to V1-4). This document **refines** 5.2, 5.3, 5.4, 6.1 and 9 of the architecture. It does not replace them. Where it adds something or reads the architecture a particular way, section 16 says so. Nothing in the architecture is changed silently.

**Codex's stated expectations, as carried into this document** (PR #3 reviews of 20 and 21 Oct; PR #5 reviews of 22 Oct):

- Use a standard construction or explicit out-of-band fingerprint verification, never an improvised authentication protocol. Pinned TLS plus an opaque single-use secret is the accepted direction.
- The full fingerprint is authoritative. The six words are only an auxiliary display.
- Specify initial trust, distinct client and server proofs, replay prevention, atomic single use and recovery.
- The implementation must show: certificate-pin enforcement on every connection, with an abort before the secret is sent; protected handling of the secret, token and server key; atomic single use; and failure tests.
- Ordering must be durable and transactional, never based on wall time (the V1-0 `local_seq` finding).
- Never apply stale responses.
- Every one of these is design evidence only, not runtime security certification.

Labels: **Arch** = already in the architecture; **New** = added here, needs sign-off; **Clar** = a reading of something the architecture leaves open; **Owner** = needs an owner decision.

---

## 0. Recommendations at a glance

| Area | Recommendation | Label |
| --- | --- | --- |
| Pairing ceremony | Pinned TLS 1.3 plus an 80-bit single-use secret, both on an admin-issued pairing slip, valid for 24 h. No PAKE | Arch |
| Slip encoding | `hy1-<host>-<port>-<fp52>-<secret16>-<check4>`, parsed right to left, case-insensitive Crockford base32, with a 4-character checksum so a typo is never reported as a "wrong server" | New (format detail) |
| Confirming a new device | The device starts **unconfirmed**. The administrator confirms it after the user reads out the six fingerprint words the companion pinned. Until then the device can enable nothing, restore nothing and delete nothing | New, Owner |
| Transport | rustls 0.23, TLS 1.3 only, no 0-RTT, no session resumption, ALPN `hy/1`, HTTP/1.1, and a custom verifier that pins the full SHA-256 of the certificate DER and **delegates signature checking** to the provider | Arch + Clar |
| Crypto provider | aws-lc-rs, the rustls default. On Windows MSVC it builds with prebuilt NASM objects and needs no CMake | New |
| Device credential | 256-bit bearer token. User-scope DPAPI with app entropy on the PC; the server stores SHA-256(token) | Arch |
| Token rotation | No periodic rotation in V1. Re-pairing is the rotation. Clones and restored PCs are detected from echoed receipts (7.4) | Clar, New |
| Server keys | ECDSA P-256 TLS key plus a **separate Ed25519 head-signing key**. Both are generated at install, protected with machine-scope DPAPI, sit behind a folder ACL, and are never put in backups. The service runs under a virtual service account | Clar, New |
| Control log | One hash chain per user, a fixed binary encoding, a mirror off the database volume, and a database-side mirror high-water used for startup checks | Arch + Clar |
| Receipts | Ed25519-signed head {key id, user, seq, chain hash, issued at}. The device checks that every new head **extends** its stored receipt, using the entries in between | Arch + Clar |
| Sync | Pull, then discard, then push. Strict per-category schemas. User id taken from the token. A remote enable needs confirmation on each PC before that PC uploads | Arch + New |
| At rest | Server database unencrypted, as disclosed (Q4). Backups encrypted with a DPAPI-protected key. `secure_delete` on | Arch + New |
| Logging | Allowlisted fields only, redacting secret newtypes, and canary tests that grep every log for planted secrets and payloads | New |

---

## 1. Scope

**In scope:** everything V1-5 needs for the companion to pair with the office server, sync opted-in daily summaries, and converge consent, deletions and recovery safely. That means the security parts of architecture sections 5.2 to 5.4, 6.1 (`server_link`, receipt), 8 (server backup) and 9.

**Out of scope:**

- Signed update and rollback files (V1-6), apart from noting that the update manifest travels over the same pinned channel.
- Camera, which never sends anything.
- V3 hosted sync. The `/v1` protocol stays versioned so `SyncTarget` can reuse it.
- End-to-end encryption of shared summaries (owner question Q4; not proposed).
- Legal applicability (owner question Q3 and assumption A13).

**Environment assumptions:**

- One office subnet with a private or domain network profile.
- One server PC running Windows 11.
- At most 3 real clients and 10 simulated ones.
- No internet use at run time.
- The pilot users are a small, mutually known group, and the server's operating-system administrator is a named person in the office (A1, Q3).

---

## 2. Assets, actors and trust boundaries

**Assets, most sensitive first:**

1. Opted-in daily health summaries on the server.
2. Device tokens (they give read and write access to one user's shared data).
3. The server's TLS key and head-signing key (whoever holds them can impersonate the server to every paired PC).
4. Pairing-slip secrets, during the 24 h they are valid.
5. Control-log integrity: consent, deletions, pairings and revocations.
6. Availability of sharing.

Local coaching data on each PC is **not** an asset of this protocol. It never leaves the PC, and the protocol must never put it at risk.

```
  Employee PC (Windows user U)                     Server PC
 +-------------------------------+              +------------------------------------+
 | Companion (per user)          |   TLS 1.3    | HealthierYou service               |
 |  hy.db: summaries, outbox,    |   pinned to  |  (virtual account NT SERVICE\...)  |
 |   policy, tombstones, receipt |  full SHA-256|  :8443 /v1 API  <-- LAN            |
 |  server_link: pin, head key,  |<============>|  127.0.0.1:8444 admin page         |
 |   DPAPI(token)                |  Bearer token|  server.db (SQLite, WAL, FULL)     |
 +-------------------------------+              |  keys/: DPAPI-machine blobs, ACL   |
      ^  boundary B1: Windows user                |  backups/ (encrypted)              |
      |  (DPAPI, profile ACL)                     +-----------------+------------------+
      |                                                             | boundary B3
  Pairing slip: out-of-band channel (B2):                           v
  printed, in person, or internal chat              control.mirror on a second disk or
                                                    a network folder (never backed up)
```

| Boundary | What crosses it | Who is trusted |
| --- | --- | --- |
| B1: Windows user on a PC | Token blob, local DB | The signed-in user. Malware running as that user or a local admin is out of scope: either one already holds the richer local data |
| B2: slip delivery | Host, port, full fingerprint, secret | Integrity is checked when the device is confirmed (4.5). Confidentiality is limited by the slip's single use and 24 h validity |
| Network (LAN) | TLS 1.3 only | Nobody. Any LAN host may be hostile |
| B3: server PC operating system | Everything on the server | The server's OS administrator is **trusted for confidentiality, as disclosed** (6.2 and Q4). The controls make misuse **visible** to users; they cannot prevent it |
| Admin page | Slips, device list, confirmations, revocations, recovery witness | Whoever is signed in at the server PC and has the admin passphrase |

---

## 3. Threat model

Each row gives the threat, what it would achieve, the control, what risk remains and the tests that cover it. Test ids T-P, T-D, T-J and T-L come from the architecture (section 11). T-S tests are new (section 14).

| # | Threat | Impact if it succeeds | Controls | Residual risk | Tests |
| --- | --- | --- | --- | --- | --- |
| TH1 | **Rogue server**: a LAN host that answers on the server's name or IP, or advertises itself | Collects tokens and uploads; feeds false policy | Every connection is accepted only if the certificate DER's full SHA-256 equals the pin, compared in constant time. The pin, not the address, gives trust. The client aborts before any HTTP bytes, so no token or secret is sent. There is no "continue anyway" button | None, unless the server's private key is stolen (TH9) | T-P1, T-P6, T-S1, T-S2 |
| TH2 | **MITM on first pairing**: the slip is altered in transit (fingerprint and host swapped), and the attacker relays to the real server | Permanent MITM: reads all of U's uploads and holds U's token | (a) The full fingerprint check is authoritative for the pin. (b) **New:** the device stays unconfirmed until the administrator compares the six fingerprint words the companion actually pinned with the server's own six words (4.5). A swapped fingerprint gives different words, so confirmation fails and the device can never enable sharing. (c) The owner chooses the delivery channel (O1) | Faking the 66-bit word prefix needs a 2^66 second-preimage search, which is not a realistic threat from an office insider. With no confirmation step (if the owner declines O1), the delivery channel must be printed or in person | T-S3 |
| TH3 | **Slip theft**: the attacker reads the slip (for example in chat) and redeems it first | A device paired as U | Single use. The genuine user sees "slip already used" and tells the administrator. The thief's device stays unconfirmed, so it cannot enable, delete, restore history or upload. Unconfirmed devices expire after 7 days. The administrator revokes it | Until it is revoked, the thief's device knows U's alias and that U exists | T-P2, T-S4 |
| TH4 | **Slip guessing** | As TH3 | 80-bit secret; per-IP and global rate limits; 24 h validity; the server stores SHA-256(secret) only | Negligible (2^80 online guesses) | T-S5 |
| TH5 | **Replay**: of a pairing request, a control operation or an upload | Double pairing; reapplying an old enable; duplicate rows | TLS 1.3 without 0-RTT or resumption, so the network cannot replay. Atomic single-use slip update. Operations are idempotent by `op_id` or `request_id`. An enable must have `base_seq` at or above the latest disable. Uploads are upserted by revision. A replayed old enable returns its original result and never re-applies | None identified | T-P3, T-P5, T-D3, T-S6 |
| TH6 | **Stolen token** (DPAPI blob plus the user's master key, malware as the user, or a cloned profile) | Reads U's shared history (all of U's devices), uploads false data for that device, disables or deletes | User-scope DPAPI. The token is never logged. Revocation works immediately (nothing is cached). **New:** clone detection by echoed receipts (7.4). U's other devices see every new pairing and revocation, and the companion lists U's devices | The token is a bearer secret until revoked. Anyone able to steal it already has U's local data. Binding to TPM or CNG is deferred (16, F11) | T-S7, T-S13, T-S19 |
| TH7 | **Rogue paired PC** (a device of U that is malicious) | False uploads under its own origin; deleting or disabling U's sharing; enabling U's sharing on U's other PCs | Uploads are bound to the token's device (origin must equal the device), and reviews show each device separately. Disables and deletions can only reduce sharing, and the change is shown on U's other PCs. **New:** a remote enable never starts uploads on another PC until that PC's user confirms it there (8.3). Receipts must carry a valid server signature, and a forged receipt is refused and logged without starting a discard | A rogue device can delete U's server history (a harm to availability, not confidentiality) | T-S8, T-S17, T-S21 |
| TH8 | **Another user's PC, or an unpaired LAN host** | Reading Y's data; denial of service | The user id comes from the token only, never from a path or body. No endpoint returns another user's data or any aggregate. Body, rate and connection limits. The admin page is not on the LAN listener | Denial of service from inside the LAN is limited, not prevented | T-S8, T-S9, T-S22 |
| TH9 | **Server key theft** (an administrator, or someone who gets the server's DPAPI machine key and the key files) | Can impersonate the server to every PC | Machine-scope DPAPI **plus** a folder ACL (machine scope alone lets any local user decrypt). Virtual service account. Keys never go in backups or the mirror | An OS administrator on the server PC is trusted (B3) | Review item |
| TH10 | **PC reimage or restore** (new Windows profile; or an older disk image or file restore) | Lost pairing; old queued records; a lost offline withdrawal; two live copies | Reimage: the DPAPI blob cannot be decrypted, so the app shows "re-pair needed" and keeps working locally. Restore: on reconnect the device pulls, then discards, then pushes (T-D1). **New:** the server sees a lower echoed receipt than the device presented before, so the PC is told to switch every category off until the user confirms each one (7.4) | A withdrawal made offline and lost with the restored disk cannot be recovered. Regression detection then turns sharing off, which is the safe direction | T-S13, T-L7 |
| TH11 | **Server restore or rollback** (backup restore, file copy, disk rollback) | Resurrected data, permissions, revoked devices or slips | Architecture 5.3 per-user recovery hold with a 7-day maximum. **New:** a startup comparison of database and mirror high-waters detects a rollback even when it happened outside the admin tool (7.3). **New:** a restore voids every outstanding slip. **Clar:** pairing entries carry the token verifier, so devices paired after the snapshot can still authenticate (9) | Covered by architecture 5.3. Global discard on shared-state failures | T-D5 to T-D13, T-J1 to T-J6, T-S15, T-S16 |
| TH12 | **Server fork** (two instances with the same key, for example a copied VM) | Diverging histories; a deletion on one is missing on the other | Devices verify that each new head extends their receipt (7.4). A fork shows as a mismatch: the device stops uploading and reports it | Detected only once a device has seen both branches | T-S14 |
| TH13 | **Administrator misuse**: pairing a device as U to read U's history, revoking or discarding U, or reading the database | Confidentiality and availability for U | No admin endpoint returns habit data. Every pairing, confirmation and revocation is a control-log entry in U's chain, signed into U's heads, and shown on U's devices with "Devices paired to you" and a new-device notice. The audit log has no habit data | An OS administrator can open the unencrypted database (disclosed, Q4). Misuse is made visible, not prevented | T-S18 |
| TH14 | **Health-data leaks in logs or diagnostics** | Data leaves the intended store | An allowlist logging schema and redacting newtypes. Payloads, aliases, tokens, secrets and slips are never logged. Canary tests (section 11) | Accidental future fields; canary tests check every build | T-S10 |
| TH15 | **Admin page attacks from the server PC's own browser** (CSRF, DNS rebinding) | Slips issued, devices revoked or confirmed | A separate 127.0.0.1 listener; `Host` and `Origin` allowlist; SameSite=Strict HttpOnly session cookie; a CSRF token on every POST; Argon2id passphrase with backoff | A local OS user with the passphrase is the administrator | T-S12 |
| TH16 | **Mirror tampering or loss** | Hides a deletion after a restore | Hash chain; database mirror high-water (7.3); global discard on corruption or an old copy (5.3) | Architecture 5.3 | T-J4, T-J5, T-S20 |
| TH17 | **Parser and memory-safety bugs** (slip, receipt, control entry, JSON) | Crash or worse | Rust; `#![forbid(unsafe_code)]` in the protocol modules (the DPAPI FFI sits in one audited module); fuzzing; body limits | Bugs in dependencies | T-S25 |

---

## 4. Pairing ceremony

### 4.1 Options considered

| Option | For | Against | Verdict |
| --- | --- | --- | --- |
| **A. Admin-issued slip: full certificate fingerprint plus an 80-bit single-use secret over pinned TLS 1.3** | Standard, audited TLS only (rustls). Server proof is the TLS handshake against the pin; client proof is the opaque secret inside that channel. Nothing new to analyse. Already accepted as a direction by Codex | A slip of about 90 characters is clumsier than a 6-digit code (A14). Slip integrity depends on the delivery channel, which is closed by the confirmation step (4.5) | **Recommended** |
| B. Short code plus a balanced PAKE (SPAKE2 or CPace), with the PAKE key authenticating the server certificate | A 6 to 8 digit code is easy to type | The Rust balanced-PAKE crates say they are unaudited (`spake2` 0.4). It needs a custom composition binding PAKE to TLS (an exporter or a certificate-hash confirmation), which is exactly the improvised protocol Codex asked us to avoid. It gives no better protection when the code channel can be read | Rejected (as in architecture 7) |
| B'. OPAQUE (`opaque-ke` 4, audited in 2021) | Audited | It is an asymmetric PAKE for long-term passwords, not one-time bootstrap codes, and it still needs TLS binding | Rejected |
| C. TOFU (trust whatever certificate is seen first) | Zero friction | No protection on first contact, which is the case that matters | Rejected |
| D. Mutual TLS with admin-issued client certificates | Removes the bearer token | Needs client key storage, a CA and renewal: more moving parts for 3 users | Rejected for V1 |

### 4.2 Ceremony steps

Before any step, the server has been installed (architecture 5.1). At install it generated the TLS key and certificate and the head-signing key (6.2), and it shows its fingerprint and six words on the admin page.

| Step | Actor | Action | Abort and error conditions |
| --- | --- | --- | --- |
| 1 | Administrator | On the admin page, creates user U (display alias only) or picks an existing user, and clicks **New slip**. The server makes a 10-byte secret from the OS CSPRNG, stores `slip_hash = SHA-256("HY-SLIP1" ‖ secret)`, `user_id`, `expires_at = now + 24 h` and `used_at = NULL`, and shows the slip **once**. It can be copied or printed. Printing also shows the six words | A user may have at most 3 outstanding slips. Creating a new slip never voids existing devices |
| 2 | Administrator to user | Hands over the slip by the channel the owner allows (O1) | n/a |
| 3 | User | Companion, Settings → Sharing & server → **Pair with the office server** → pastes or types the slip. First the pairing notice is shown, with the operational data disclosure (architecture 6.2, PRD 150) | Local checks: format, length, checksum. A checksum failure says "The slip looks mistyped", never "wrong server" |
| 4 | Companion | Resolves the host with the OS resolver (DNS, LLMNR, or mDNS `.local` handled by Windows), then the slip's port. Opens TLS 1.3 with the pinning verifier (section 5) set to the slip's fingerprint | Fingerprint mismatch: **abort before any application bytes**, show "This is not the office server named on your slip. Do not continue; tell your administrator", and log only `pin_mismatch` with the first 8 hex characters of each fingerprint. TLS below 1.3, no shared ALPN or a handshake error: abort |
| 5 | Companion | `POST /v1/pair` with `{protocol: 1, secret, origin_device, device_label, app_version, windows_build}` | n/a |
| 6 | Server | One `BEGIN IMMEDIATE` transaction (4.4). Returns `{user_id, device_ref, token, head_key, server_fingerprint, state}`, where `state` holds U's current policy, `share_from` dates, tombstones, devices, the chain from seq 1 and the signed head | `slip_invalid`, the same response for unknown, used or expired (timing-equalised), to avoid an oracle. `origin_conflict` if `origin_device` belongs to another user. 429 when rate-limited |
| 7 | Companion | Checks that `server_fingerprint` equals the pin. Verifies the chain from seq 1 and the head signature with `head_key`. Then, in one local transaction: stores `server_link` (pin, head key, host, port, user id, device ref, DPAPI(token)), stores the head as its receipt, applies the policy (every category stays **off**), merges tombstones (union), and writes a `paired` consent-log entry with the notice version | Any check fails: nothing is stored and the token is discarded. The server-side pairing expires as unconfirmed after 7 days |
| 8 | Companion | Shows "Paired as `<alias>`. Waiting for your administrator to confirm this PC" and the **six fingerprint words of the pin it holds**, with a device check code | n/a |
| 9 | User and administrator | The user reads the six words out, in person or by phone. The administrator compares them with the server's six words on the admin page, row for U's new device, and clicks **Confirm**. The server writes a `device_confirmed` control entry in U's chain | Words differ: the administrator clicks **Reject**, which revokes the device, and investigates. A MITM is likely |
| 10 | Companion | At the next sync it sees `confirmed`. It can now enable categories and restore history | n/a |

**Distinct proofs (Arch):**

- **Server to client:** TLS 1.3 CertificateVerify by the key whose certificate hashes to the pin.
- **Client to server, at pairing:** possession of the slip secret, sent once inside that channel.
- **Client to server, afterwards:** possession of the token, inside a channel pinned the same way.

Every proof is carried by TLS. There is no application-level handshake, HMAC or exporter binding (Codex, 20 Oct). Step 9 is an **administrative check of slip integrity, not a cryptographic proof**. It adds no protocol messages.

### 4.3 Slip format (New: a format detail on top of architecture 5.2)

```
hy1-<host>-<port>-<fingerprint: 52 base32 chars>-<secret: 16 base32 chars>-<check: 4 base32 chars>
```

- **Alphabet:** Crockford base32 (no I, L, O or U), case-insensitive. On input, `O` is read as `0` and `I` or `L` as `1`. Hyphens and spaces inside groups are ignored when typed from print. Print shows the fingerprint and secret in groups of 4.
- **Parse right to left.** The last three fields have fixed lengths and alphabets, then comes the port (decimal, 1 to 65535), and whatever is left is the host. This removes the ambiguity of hyphenated host names in the architecture's format.
- **`check`** = the first 20 bits of `SHA-256("HY-SLIPCHK1" ‖ canonical slip without check)`. It catches typos only and is not a security control.
- **`host`** is the server PC's computer name, or an IP as a fallback, and is limited to 253 characters of `[A-Za-z0-9.-]`. The slip carries no user alias, so a leaked slip does not name its user.
- **Length:** about 90 characters with a 15-character host. Assumption A14, that users accept pasting it, is still checked in the V1-5 owner slot.

### 4.4 Redemption transaction (server, Arch plus Clar)

All in one `BEGIN IMMEDIATE` transaction, in WAL mode with `synchronous=FULL`. Any failure rolls back everything:

1. `UPDATE slips SET used_at=:now WHERE slip_hash=:h AND used_at IS NULL AND expires_at>:now AND voided=0`. It must affect exactly 1 row.
2. Check that `origin_device` is not bound to a different user. The same user re-pairing the same PC is allowed: a new `device_ref`, and the old device must already be revoked or is revoked here with reason `superseded`.
3. Make `token` (32 bytes, CSPRNG). Insert the `devices` row {device_ref, user_id, origin_device, label, token_verifier = SHA-256("HY-TOK1" ‖ token), state='unconfirmed', created_at}.
4. Append U's control entry `pairing` {device_ref, origin_device, token_verifier} with the next seq and chain hash (7.1).
5. After commit, append to the mirror and fsync, update `mirror_hwm` (7.3), and only then send the response (architecture 5.3 ordering). If the response is lost, the device retries with the same slip and gets `slip_invalid`. The administrator then sees an unconfirmed device that never checked in and issues a new slip. This is accepted for 3 users, and it is better than making slips reusable.

**Rate limits:** `/v1/pair` allows 5 attempts per source IP per minute and 30 per hour, and 60 per hour globally. When exceeded, the server answers 429 with `Retry-After`, and the attempt counter is persisted so a restart does not reset it. Failed attempts are counted per IP in the audit log, without the secret.

### 4.5 After pairing

| Rule | Detail | Label |
| --- | --- | --- |
| Unconfirmed device | Can call only `GET /v1/state` (and receives only its own pairing status, U's alias and the head) and `POST /v1/unpair`. Control operations, uploads and history are refused with `device_unconfirmed` | New |
| Confirmation | A `device_confirmed` entry in U's chain. Shown on the admin page with the confirmation time | New |
| Expiry | An unconfirmed device is revoked automatically after 7 days (a `revocation` entry with reason `unconfirmed_expired`) | New |
| New-device notice | U's other confirmed devices show once: "A new PC '`<label>`' was paired to your HealthierYou on `<date>`. Not you? Tell your administrator." The same applies to revocations | New |
| Device list | Companion → Sharing & server shows "PCs paired to you" (label, paired date, last seen, confirmed or not) from `state.devices` | New |
| If the owner declines confirmation (O1 alternative) | The architecture's ceremony stands: the device is active at once, and the slip must be delivered printed or in person | Owner |

---

## 5. Transport

| Item | Contract |
| --- | --- |
| Protocol | TLS 1.3 only, on both sides (`builder_with_protocol_versions(&[&TLS13])`). 0-RTT off: the server's `max_early_data_size = 0`, and the client has `enable_early_data = false` |
| Resumption | Off on both sides: client `Resumption::disabled()`, server `send_tls13_tickets = 0` with no session storage. Every connection therefore goes through a full handshake and a fresh pin check. The cost is negligible for 13 clients |
| Key exchange and suites | Provider defaults (aws-lc-rs: X25519MLKEM768 hybrid, X25519, P-256; AES-GCM and ChaCha20-Poly1305) |
| ALPN | `hy/1` only, on both sides. A mismatch aborts, which prevents cross-protocol confusion with other services on 8443 |
| Server name | The client always sends the constant SNI `hy-server.invalid`. The certificate's SAN contains that name plus the computer name, for diagnostics only. Names are **not** checked: the pin is the identity |
| Client verifier | A custom `rustls::client::danger::ServerCertVerifier`: (1) `verify_server_cert` computes SHA-256(end-entity DER), compares it in constant time (`subtle`) with the 32-byte pin, rejects on mismatch, ignores intermediates, names and validity dates, and runs no OCSP. (2) `verify_tls13_signature` **delegates** to `rustls::crypto::verify_tls13_signature` with the provider's algorithms. (3) `verify_tls12_signature` always fails. (4) `supported_verify_schemes` comes from the provider. Item 2 is load-bearing: a verifier that skips it would accept a replayed certificate without its key (T-S2) |
| Server certificate | Self-signed X.509 v3, ECDSA P-256 / SHA-256, generated with rcgen at install. Validity 20 years, because the pin replaces expiry and a planned rotation means re-pairing (architecture 5.2). Serial from the CSPRNG |
| HTTP | HTTP/1.1 only (no HTTP/2 or 3). JSON bodies. Request body limit 256 KiB (a batch of at most 200 summary rows). Header timeout 10 s, idle timeout 60 s. At most 64 concurrent connections and 8 per IP |
| Auth header | `Authorization: Bearer <base64url token>` on every request except `/v1/pair`. Also `HY-Receipt: <encoded head>` (7.4) |
| Proxies | The companion's HTTP client has system-proxy support switched off (`no_proxy()`). Traffic goes straight to the LAN host |
| Address | The OS resolver for the slip's host, then the last IP that passed the pin (kept in `server_link`), then a manual address the user enters in settings. **Clar:** no application mDNS listener on clients. A per-user unsigned app that opens a UDP listener can trigger a Windows Firewall prompt that needs administrator rights. Windows itself resolves `<computer>.local` (16, F2) |

---

## 6. Credentials and keys

### 6.1 Inventory

| Secret or identity | Where it lives | Protection | Lifetime | Ever logged or backed up? |
| --- | --- | --- | --- | --- |
| Slip secret (80 bits) | Shown once on the admin page and on paper or in chat | Server stores `SHA-256("HY-SLIP1" ‖ s)` | 24 h, single use, voided on restore | Never logged. Hashes appear in server backups (harmless) |
| Device token (256 bits) | PC: `server_link.token_blob` = DPAPI user scope. Server: `SHA-256("HY-TOK1" ‖ t)` in `devices` and in U's `pairing` entry | DPAPI with `CRYPTPROTECT_UI_FORBIDDEN` and app entropy `"HealthierYou/server_link/v1"`. In memory as `Zeroizing<[u8;32]>` | Until revoked or re-paired | Never logged. Excluded from `.hybackup` (V1-4, already enforced) |
| Pin (SHA-256 of the certificate DER) | `server_link.pin` | Public value; integrity comes from the slip and confirmation | Until re-paired | May be logged as an 8-hex-character prefix |
| Head-signing public key (Ed25519) | `server_link.head_key` | Arrives over the pinned channel at pairing | Same as the server key | Prefix only |
| Server TLS key (P-256) | `%ProgramData%\HealthierYou\Server\keys\tls.key.dpapi` | Machine-scope DPAPI **and** a folder ACL (the service account, Administrators, SYSTEM). Machine scope alone lets any local user decrypt | Until a planned rotation (re-pair everyone) | Never in backups or the mirror |
| Head-signing key (Ed25519) | `keys\head.key.dpapi` | Same as the TLS key | Same | Never |
| Backup encryption key (256 bits) | `keys\backup.key.dpapi` | Same as the TLS key | Same | Never (it encrypts the backups) |
| Admin passphrase | `admin` table: Argon2id hash (64 MiB, 3 passes, 1 lane, 16-byte salt; the parameters used by `.hybackup`) | Hash only | Until changed | Never |
| Admin session | Random 256-bit cookie value, stored as a hash | HttpOnly, SameSite=Strict, `Path=/`, 30-minute idle timeout | Session | Never |

### 6.2 Server key handling

- **Generation:** at install, by the service's first run under its own account. The service runs as the virtual account `NT SERVICE\HealthierYouServer`, not LocalSystem (New). Keys come from aws-lc-rs and the OS CSPRNG.
- **Keys are never exported, printed, backed up or written to the mirror.** On replacement hardware or an OS reinstall, the keys and the DPAPI machine master key are gone, so architecture 5.3's global discard and re-pair apply. That is consistent with "lost server key resets every user". An escrowed key backup is **not** recommended: a copy of the key lets someone impersonate the server to every PC (O4).
- **Separate head-signing key (Clar).** The architecture asks for a "server signature" on heads without naming the key. A separate Ed25519 key keeps the two protocols apart: the TLS key is only ever used by rustls, and the head key only for `HY-HEAD1` messages. Both are "the server key" for the global-discard rule. Alternative: sign heads with the TLS key using a domain-separation prefix, which means one key fewer. This is decision C5.

### 6.3 Rotation and revocation

| Topic | Contract | Label |
| --- | --- | --- |
| Periodic token rotation | **Not in V1.** Rotation interacts badly with server restore: a token rotated after the snapshot is unknown to the restored database unless every rotation is also written to the control log and mirror. Anyone able to steal the token already holds U's richer local data. Re-pairing (revoke plus a new slip) is the rotation mechanism | Clar |
| Clone and restored-PC detection | Without rotation: echoed-receipt regression (7.4) | New |
| Revocation by the administrator | Admin page → device → Revoke. A `revocation` entry goes in U's chain, in the same transaction as `devices.state='revoked'`. The token verifier stays (it can no longer authenticate) and audit data is kept for 30 days. Every authenticated request reads the device state from the database, with no cache, so revocation takes effect on the next request | Arch |
| Self-unpair | Companion → Unpair: `POST /v1/unpair` (revocation, reason `self`). Done or not, locally in one transaction: delete `server_link` and the receipt, purge the outbox, set every category off (purge triggers), write `unpaired` to the consent log | New |
| Revoked device reconnects | It receives `401 {error:"device_revoked"}`, which it can tell apart from `401 unknown_token` because the verifier is kept. The local unpair transaction runs, and the app offers to re-pair | Arch (T-P6) |
| Planned server key rotation | Install new keys, which means every device re-pairs (architecture 5.2) | Arch |
| Unseen devices | The admin page flags devices unseen for more than 14 days, because they would block that user's release from recovery hold. Revocation stays manual | New |

---

## 7. Signed heads, receipts and the server control log

### 7.1 Control entries and the chain (Clar: an encoding for architecture 5.3)

There is one chain per user. Entries use a fixed binary encoding, defined once in `hy-core` (`hy_core::protocol`) and shared by the server, the companion and the simulator. JSON canonicalisation is not used anywhere in hashing or signing.

```
genesis(U)   = SHA-256("HY-CL1-GENESIS\0" ‖ key_id ‖ user_id)
entry_hash_n = SHA-256("HY-CL1\0" ‖ user_id[16] ‖ u64be(seq) ‖ u64be(at_ms) ‖ u8(kind)
                       ‖ u32be(len(body)) ‖ body ‖ prev_hash[32])
prev_hash_1  = genesis(U);  prev_hash_n = entry_hash_{n-1}
key_id       = SHA-256("HY-KEYID1\0" ‖ head_public_key[32])
```

| kind | body (fixed-width fields, dates as `u32be` days since 1970-01-01) | Effect, in the same transaction |
| --- | --- | --- |
| 1 `policy` | category u8, enabled u8, share_from u32 (0 if off), op_id[16], device_ref[16] | `policy` row |
| 2 `deletion` | category u8, from u32, to u32, request_id[16], device_ref[16] | Tombstone; delete the matching summaries |
| 3 `pairing` | device_ref[16], origin_device[16], token_verifier[32] | `devices` row |
| 4 `device_confirmed` | device_ref[16] | `devices.state='active'` |
| 5 `revocation` | device_ref[16], reason u8 (admin, self, superseded, unconfirmed_expired, recovery_discard) | `devices.state='revoked'` |
| 6 `recovery_discard` | snapshot_id[16] | Discard U's restored history; categories off (architecture 5.3) |
| 7 `history_access` | device_ref[16] | Records a history read by a newly confirmed device, so the user can see it |

- **No health payloads** are in any entry. Aliases, labels and IP addresses are never in entries. Users and devices appear only as random 128-bit ids.
- **What the user can learn:** the user's own devices receive the entries of the user's own chain (7.4). That is the user's own consent history and device list, which the user may see.

### 7.2 Signed head

```
head    = { key_id[32], user_id[16], seq u64, chain_hash[32], issued_at_ms u64 }
message = "HY-HEAD1\0" ‖ key_id ‖ user_id ‖ u64be(seq) ‖ chain_hash ‖ u64be(issued_at_ms)
sig     = Ed25519_sign(head_key, message)               (64 bytes, deterministic)
wire    = base64url(head ‖ sig)                         (in JSON and in the HY-Receipt header)
```

- **When it is sent:** every sync response carries U's current head, except during U's recovery hold (architecture 5.3: no new head is sent to a user in hold).
- **Acknowledging a control operation:** the response carries {seq, chain_hash, head}.
- **Verification:** a head is valid only under the `key_id` the device stored at pairing. A head with another `key_id` is refused and logged (T-D12).

### 7.3 Mirror and the startup checks (Arch plus New)

- **Mirror record:** one line per entry, `HYM1 <user_id hex> <seq> <at_ms> <kind> <body b64url> <prev hex> <hash hex>\n`. Each line is written whole, then fsynced. A torn final line is detected and truncated at repair (T-J4).
- **File header:** `HYM1-HEADER <mirror_id> <key_id> <created_at>`. The `mirror_id` is random, created at install and also stored in the database.
- **`mirror_hwm` (New).** After the mirror fsync, and **before** the client is acknowledged, the server records `mirror_hwm(U) = seq` in a small transaction of its own. Every acknowledged entry therefore has `mirror_hwm ≥ seq` in the live database.

**Startup comparison** (for each user, before the API opens):

| Database tail vs mirror tail for U | Meaning | Action |
| --- | --- | --- |
| Equal, same hash | Normal | Open |
| Database ahead and mirror tail ≥ `mirror_hwm(U)` | Crash after commit, before the mirror append (T-J2) | Re-append the missing entries, then open |
| Mirror ahead of the database | The database was rolled back or restored, even outside the admin tool | **Every user enters recovery hold** (architecture 5.3) |
| Mirror tail < `mirror_hwm(U)` | The mirror is an old copy | Global discard path (architecture 5.3) |
| Same seq, different hash; broken chain; `mirror_id` or `key_id` mismatch | Corruption or a foreign mirror | Global discard path |

A restore done through the admin tool also sets the hold explicitly, and voids all outstanding slips (9).

### 7.4 Device-side receipt handling (Arch plus Clar plus New)

| Rule | Detail |
| --- | --- |
| Store | Table `head_receipt` holds one row: the highest valid head the device has received for (server key, user). A trigger refuses a lower seq. Replacing it needs a verified chain extension (next row) |
| Extension check (Clar) | Sync responses include U's chain entries with seq greater than the device's receipt seq (paged at 500). The device recomputes the hashes from `receipt.chain_hash` up to `head.chain_hash`, then verifies the signature. If both hold, it replaces the receipt and applies the entries' effects (policy, tombstones, device list). If not, it does not apply anything, enters **"server history mismatch"** (uploads stop, local use carries on, the pet shows the sharing-paused badge) and reports the receipt at the next contact. This is what makes "never replace a receipt with a lower seq or a different chain" checkable on the device |
| Echo (New) | Every authenticated request carries `HY-Receipt`. The server records `max_echoed_seq(device)` |
| Regression (New) | The device's echoed receipt for U is valid and lower than `max_echoed_seq(device)` (not during a hold). That means the PC was restored from an older copy or cloned. The server answers `409 device_regressed`. The device switches every category off locally (purging its outbox) and asks the user to confirm each category again. The admin page shows "possible restored or duplicated PC". If it happens again within 24 h, which suggests two live copies, the server revokes that device automatically with reason `superseded`. Echoes reflect only what the device stored, so a lost response never causes a false positive |
| Head during hold | The device presents its stored receipt first (`POST /v1/receipt`). It receives no new head. Disables and deletions are acknowledged as `accepted_pending_recovery` with no seq, and are kept locally as `pending_recovery` (that state already exists in the schema) (architecture 5.3, T-D11) |

---

## 8. Sync rules

### 8.1 API surface (`/v1`, JSON over pinned TLS)

| Method and path | Auth | Purpose |
| --- | --- | --- |
| `POST /v1/pair` | Slip secret | 4.2 and 4.4 |
| `GET /v1/state?after_seq=` | Token | Policy, `share_from`, tombstones, devices, chain entries after `after_seq`, signed head, hold state, `confirmed`. Unconfirmed devices get pairing status only |
| `POST /v1/control` | Token, confirmed | Batch of {enable, disable, delete}, idempotent by op_id or request_id |
| `POST /v1/upload` | Token, confirmed | Batch of summary rows (8.6) |
| `GET /v1/history?from=&to=` | Token, confirmed | "Restore from server": U's opted-in daily summaries, keyed by origin, read-only. Writes a `history_access` entry |
| `POST /v1/receipt` | Token, or a known or unknown token during a hold | Presents a receipt as evidence (architecture 5.3) |
| `POST /v1/unpair` | Token | Self-revocation |
| `GET /v1/update/manifest` | Token | V1-6 |

**Common rules for every endpoint:**

- The user id always comes from the token.
- Path and body never carry a user id. Any device or origin id in a body must belong to the token's own device.
- Errors use one small enum; there is no free-text detail and no stack trace.

### 8.2 What is shared, per category (from architecture 6.2; all off by default)

| Category (schema name) | Fields that may reach the server (daily, per origin device) | Never sent |
| --- | --- | --- |
| `hydration` | ml logged, goal ml, number of entries, coverage | Entry times, serving names |
| `outcomes` | Reminders offered, done, snoozed, skipped and no-response counts; routine done, partial, none or skipped | Times, reminder text |
| `work_session` | Active minutes (estimate), longest block (estimate), focus count, Pomodoro count | Any within-day timing |
| `phone_reports` | Counts by label (work, personal, unsure) | Notes, times |
| `posture_summary` (V2) | Camera-on minutes, usable coverage, episode count | Frames, landmarks, per-episode times |
| Never (V1/V2) | Meditation, recovery plans, reflections, settings, pet, goals history, detailed events | All of these |
| Operational (disclosed separately) | Device ref, label, app version, Windows build, last seen, pairing state, policy seq, queue length | n/a |

Each category has a versioned JSON schema in `hy-core` with `deny_unknown_fields`, bounded numbers and no free text. The companion's preview (SHR-01) renders the exact record it would send, from the same serializer.

### 8.3 Consent and policy

| Rule | Detail | Label |
| --- | --- | --- |
| Ordering | A disable is always accepted. An enable needs `base_seq` at or above the seq of the category's latest disable or deletion, otherwise `stale_enable` (architecture 5.3; already implemented in the V1-0 store with durable `local_seq` and stale-response classification) | Arch |
| `share_from` | Set to the server's local date (in U's zone) when the enable is accepted. No backfill | Arch |
| **Remote enable** | When device B pulls an enable that device A made, B records it but **does not start building outbox records** until B's user confirms on B: "Sharing of `<category>` was switched on from '`<A label>`' on `<date>`. Share from this PC too?" (with the preview). B's consent log records `notice_shown` and `enabled`. Until then B treats the category as off for uploads. B's decision is local only, unless B's user picks "Switch off everywhere", which sends a disable | New, Owner (UX) |
| Remote disable | Applied on B at once: purge, off, and a one-line notice | Arch |
| Unconfirmed device | Cannot send enables (4.5) | New |

**Local schema effect (8.3):** `sharing_policy.enabled` keeps meaning "the server has accepted it". A new column `local_confirmed` (default 0, set by a local enable or by the remote-enable confirmation) must be 1 for `outbox_guard` to accept a record (section 13).

### 8.4 Deletion and tombstone propagation

1. Device A records the deletion locally (V1-0: tombstone `pending`, purge in the same transaction) and sends it in `/v1/control`.
2. Server, in one transaction: inserts the tombstone, deletes the matching rows and writes the `deletion` entry. After that: mirror fsync, `mirror_hwm`, acknowledgement with seq and head. A then marks the tombstone `acknowledged`.
3. Other devices of U receive the `deletion` entry through the extension check (7.4). Each inserts the tombstone `acknowledged` with the same `request_id`, which purges its outbox (an existing trigger).
4. Tombstoned dates are **permanently non-uploadable** for U and that category, both on devices (the `outbox_guard` trigger) and on the server (8.6).
5. The server sets `PRAGMA secure_delete=ON` and runs a `wal_checkpoint(TRUNCATE)` after a deletion, so deleted content is overwritten in the database and WAL. Backups keep it until they expire, within 30 days at most (disclosed in the deletion receipt; architecture 5.4).
6. Server tombstones are kept permanently in V1. They are tiny and hold no payload, which matches the local schema.

### 8.5 Outbox purges

| Trigger | Purge | Where it is enforced |
| --- | --- | --- |
| Local disable | That category, at once, even offline | Existing trigger `sharing_disable_purges_outbox` |
| Later `share_from` | Rows before it | Existing trigger |
| Tombstone (local or pulled) | Rows in its range | Existing trigger |
| Remote enable not yet confirmed on this PC | Nothing can be queued | New `outbox_guard` condition |
| Unpair, revocation or `device_regressed` | Everything, and every category off | New, one local transaction |
| `server history mismatch` | Nothing is purged, but nothing is sent until it is resolved | New |

### 8.6 Server upload validation (Arch plus New)

A row {category, origin_device, local_date, revision, payload} is accepted only if **all** of these hold:

- the device is confirmed and not revoked;
- `origin_device` equals the token device's origin;
- U is not in recovery hold;
- the category is enabled now and `local_date ≥ share_from`;
- `local_date` is outside every tombstone;
- `local_date` is not more than 1 day after the server's date in U's zone, and not more than 90 days before it;
- the payload validates against the category schema;
- `revision` is greater than the stored revision.

Otherwise the row is rejected with a per-row reason, and the client discards it, never re-stamping it (architecture 5.3).

**Retention (Clar):** the server keeps daily summaries for 90 days, the same as the device's `daily_summary`, and prunes them daily. There are no monthly rows on the server in V1.

---

## 9. Recovery-hold interaction

Architecture 5.3, revision 5, is unchanged: per-user hold, 7-day maximum (owner, 21 Oct), release only on the mirror plus all of that user's device receipts plus the administrator witness, and per-user or global discard. This proposal adds four points.

| Point | Contract | Label |
| --- | --- | --- |
| Authentication after a restore | `pairing` entries carry `token_verifier`, so replaying the mirror into a restored database recreates the device rows. A device paired after the snapshot can then authenticate with its token and present its receipt. Without this, it would show up as an unknown-token device | Clar |
| Slips | Any start from a restored snapshot voids every unused slip (`voided=1`). A slip redeemed after the snapshot cannot be redeemed again | New |
| Unconfirmed devices | They count as paired for U's evidence: strict, and consistent with "every device that U's chain lists as paired and not revoked". The administrator can revoke one (a disable-class operation, allowed during hold) before confirming U's list | Clar |
| Regression checks during hold | Turned off for U while U is in hold. Pre-restore receipts are expected to be higher than the restored database knows. After release, `max_echoed_seq` is rebuilt from the evidence receipts | Clar |
| Companion UI | "Server recovering: sharing paused". Local coaching, reviews and export are unaffected (T-D13) | Arch |

---

## 10. Server storage and encryption at rest

| Item | Decision | Label |
| --- | --- | --- |
| Database (`server.db`) | SQLite (WAL, `synchronous=FULL`, `secure_delete=ON`, `foreign_keys=ON`) in `%ProgramData%\HealthierYou\Server`. The folder ACL allows the service account, Administrators and SYSTEM only. Summary payloads are **unencrypted**, as disclosed in the sharing audience text (Q4). Encrypting them with a key held on the same PC would not stop the administrator and would make the disclosure harder to state | Arch |
| Backups | Daily and catch-up schedule as in architecture 5.1. Each is `VACUUM INTO` a temporary file, then encrypted with AES-256-GCM (the `.hybackup` container with a raw-key variant: version, nonce, AAD = header plus `snapshot_id` plus a high-water summary) under `backup.key`, then the temporary file is deleted. Reason: backups may sit on a second disk or a share that more people can read. Backups cannot be restored on another machine, which matches "lost server key means global reset" | New |
| Mirror | Holds no payloads, only random ids (7.1). Off the database volume (architecture 5.3, A1). It is not encrypted, so the recovery path stays inspectable. ACL as for the database where the medium allows | Arch |
| Disk | Recommend BitLocker on the server PC as a deployment check (A1). Not required: that is the owner's call (O3) | Owner |
| Data minimisation | The server stores a user alias only (no email, phone or employee number), random ids and the device label | Arch |

---

## 11. Logging without health data

| Rule | Detail |
| --- | --- |
| Format | JSON lines through `tracing` and `tracing-subscriber`, written to `%ProgramData%\...\logs` (server) and the existing companion log. Rotated at 7 days or 10 MB (architecture 10) |
| Allowlist | Request id, endpoint, status, error enum, latency, sizes and counts, user id and device ref (random), seq numbers, an 8-hex-character pin prefix, app version |
| Never | Payload fields or values, aliases, labels, IP addresses beyond the /24 of the source (for rate-limit diagnostics), tokens, secrets, slips, passphrases, cookies, full head signatures, file paths that contain a user name |
| Types | `Secret<T>` newtypes (token, slip secret, passphrase, keys) whose `Debug` and `Display` print `[redacted]`, which do not implement `Serialize`, and which zeroize on drop. Request bodies are never logged. A panic hook strips payloads |
| Audit log (admin page) | Slip issued, redeemed or expired; device confirmed or revoked; recovery events; admin logins and failures. Ids and aliases only, no habit data, kept 30 days |
| Proof | Canary tests (T-S10). Diagnostics zips from the companion run the same redaction before the user reviews them |

---

## 12. Failure modes and offline behaviour

The companion never waits on the server. Every failure leaves local coaching, logging, reviews and export working (PRD; architecture 5.4).

| Situation | Companion behaviour | Server behaviour |
| --- | --- | --- |
| Server unreachable | Backoff from 30 s to 15 min with jitter. Disconnected badge. The outbox is kept and its purge rules still apply offline | n/a |
| Pin mismatch on a paired PC | Sharing stops. "The office server's identity has changed. Ask your administrator." No override. The token is never sent | n/a |
| Slip mistyped, used, expired or unknown | Mistyped: caught locally by the checksum. The others: one generic message, "This slip can't be used. Ask for a new one." | One generic `slip_invalid`, with timing equalised |
| `device_unconfirmed` | "Waiting for your administrator to confirm this PC", with the six words shown | Read-only status |
| `device_revoked` | Local unpair transaction; offer to re-pair | 401 |
| `stale_enable` | The toggle reverts, with "Switched off on another PC on `<date>`. Switch on again?" | n/a |
| Recovery hold | "Server recovering: sharing paused". Disables and deletions stay `pending_recovery` | No history, uploads or enables for U; no new head |
| Server history mismatch (fork or rollback seen by the device) | Uploads stop. "Sharing paused: the server's history doesn't match this PC's." Receipt reported | Architecture 5.3 incident path |
| `device_regressed` | Every category off; asks to re-confirm | Flags it; auto-revokes a repeat within 24 h |
| Mirror unavailable (share offline) | Control operations get 503 `cannot_record`. Local disables are still in force and stay pending. Uploads continue for categories that are on | Control operations are refused, never acknowledged without the mirror |
| Server disk full | Uploads get 503; retries keep the outbox | Rejects writes, never partially. The admin page shows an alert |
| Server clock skew | Dates come from the server (`share_from`) and the device (`local_date`). Uploads outside the ±window are refused with `date_out_of_range` | n/a |
| DPAPI decryption fails (reimage, profile reset, copied DB) | "Re-pair needed". Local data is untouched | n/a |
| Rate-limited pairing | "Too many attempts. Try again in N minutes." | 429 |

---

## 13. Local schema changes for V1-5 (forward migration `0005`)

The v1 migration is frozen (PR #5), so these changes go in a new forward migration and are listed here for review only. **No code is in this document.**

| Change | Reason |
| --- | --- |
| `server_link` (one row): `server_key_id`, `pin` BLOB(32), `head_key` BLOB(32), `host`, `port`, `last_ip`, `user_id`, `device_ref`, `token_blob` (DPAPI), `confirmed` 0/1, `paired_at` | Deferred from V1-0 |
| `head_receipt` (one row): `key_id`, `user_id`, `seq`, `chain_hash`, `issued_at`, `sig`. A trigger refuses any decrease in seq. It is deleted only together with `server_link` | Deferred from V1-0 (architecture 6.1) |
| `paired_devices` (cache of `state.devices`) | Device list and new-device notices |
| `sharing_policy.local_confirmed` and an extra `outbox_guard` condition | Remote-enable confirmation (8.3) |
| `consent_log.kind` gains `paired`, `unpaired`, `remote_enable_confirmed`, `regression_reset`. That needs a **table rebuild**, because SQLite cannot change a CHECK constraint, and the rebuild must re-create the append-only trigger and keep the rows | Consent record |
| Tombstones pulled from the server inserted as `acknowledged` | Propagation (8.4) |
| Backup (V1-4) keeps excluding `server_link`, `head_receipt` and `paired_devices` | Pairing never travels in a backup (PR #14) |

---

## 14. Test plan

### 14.1 Carried over from the architecture (unchanged)

- **T-P1 to T-P6:** pairing.
- **T-D1 to T-D13:** consent, deletion and recovery hold.
- **T-J1 to T-J6:** control-log crash points.
- **T-L5 to T-L7:** multi-device and re-pair.

All run against the real server binary on Linux CI, and the Windows-specific items run on the owner's PCs.

### 14.2 New adversarial and security tests (T-S)

| Id | Test | Pass condition |
| --- | --- | --- |
| T-S1 | Pin enforcement on **every** connection: pairing, sync, history, receipt and manifest each go to a test server with a different self-signed certificate. A raw TCP capture on the test server records the bytes it receives | The handshake fails, and the server receives **no application data bytes**, so no `Authorization` header and no secret |
| T-S2 | Certificate replay: the test server presents the genuine pinned certificate DER but holds a different private key | The handshake fails (the signature check is delegated). Also a unit test that the verifier returns an error from `verify_tls12_signature` |
| T-S3 | Tampered slip, with a relay MITM: the attacker's slip carries the attacker's fingerprint, and the attacker relays to the real server | Pairing completes on the attacker's path, but the six words on the companion differ from the admin page's. The device stays unconfirmed, and enable, history and upload are all refused by the real server |
| T-S4 | Stolen slip redeemed first | The genuine user gets `slip_invalid`. The thief's device cannot enable, delete, upload or read history. It is auto-revoked at day 7 (injected clock) |
| T-S5 | Slip guessing: 1,000 random secrets from one IP and from 20 IPs | 429 after the limits. Counters survive a restart. A genuine slip still works after the window. Response timing does not tell unknown, used and expired apart (statistical check, generous bound) |
| T-S6 | Replay: resend a captured `/v1/control` enable after a later disable; resend a pairing body; resend an upload with the same revision | Original results only. No enable re-applied, no second device, no duplicate row |
| T-S7 | Token misuse: wrong, truncated, revoked, other-user and empty tokens; the token in a query string | 401 with the right enum. A revoked token gets `device_revoked`. Nothing else leaks |
| T-S8 | Cross-user isolation: X's device puts Y's user id, device ref or origin in paths, bodies and headers, on every endpoint (property-based) | X gets only X's data. Y's data is unchanged |
| T-S9 | Upload validation: unknown fields, a free-text field, oversized payload, negative numbers, a future date, a date older than 90 days, origin not equal to the device, a tombstoned date, a date before `share_from`, category off, unconfirmed device, user in hold | Each is rejected with its reason. Server row count unchanged |
| T-S10 | Log canaries: the full integration suite runs with planted values (alias `CANARY_ALIAS_7f3a`, label, water amount `987654`, token, slip secret, passphrase) | `grep` over every server and companion log, the audit log, the admin page HTML, panic output and the diagnostics zip finds **none** of them |
| T-S11 | DPAPI (Windows, owner PC): (a) a second Windows user on the same PC cannot open user A's token; (b) the companion database copied to another PC or profile shows "re-pair needed" without crashing; (c) the server key files cannot be read by a standard user | As stated |
| T-S12 | Admin page: (a) not reachable from another LAN host; (b) a cross-origin POST from a page on another port fails (CSRF and Origin checks); (c) `Host: attacker.example` is refused (DNS rebinding); (d) wrong passphrase gets exponential backoff | As stated |
| T-S13 | Regression and clone: restore an older copy of a paired companion's database and reconnect; then run two copies alternately | First: `device_regressed`, every category off locally, outbox empty, admin flag. A repeat within 24 h: the device is revoked |
| T-S14 | Server fork: two server instances from one key and database copy diverge (a deletion on one). A device syncs with both in turn | The device detects the extension mismatch, stops uploading, does not apply the other branch's policy, and reports its receipt |
| T-S15 | Restore voids slips: snapshot, then redeem slip S, then restore the snapshot, then redeem S again | `slip_invalid` |
| T-S16 | Post-snapshot pairing after a restore: device D paired after the snapshot | D authenticates after mirror replay and its receipt counts as evidence. Without the `token_verifier` in the entry (a negative test with a fault-injected build), D would be unknown, which proves the field is needed |
| T-S17 | Remote enable: A enables hydration; B syncs | B uploads nothing until B's user confirms. The consent logs show it on both PCs. A network capture from B shows no hydration payload before confirmation |
| T-S18 | Administrator visibility: the administrator pairs a second device for U and confirms it | U's existing PC shows the new-device notice and the device list. The `pairing`, `device_confirmed` and `history_access` entries are in U's chain under signed heads |
| T-S19 | Revocation propagation: revoke B while B is offline with a queued outbox | On reconnect: 401 `device_revoked`, local unpair transaction, outbox empty. The network capture shows no payload after the 401 |
| T-S20 | Mirror tamper: flip one byte; truncate in the middle; reorder two lines; use another install's mirror | Each takes the global discard path at startup. A torn final line alone is repaired (T-J4) |
| T-S21 | Forged receipts: bad signature, wrong `key_id`, another user's receipt, a valid receipt replayed by a different user's device | Refused and logged. No discard is started. Only genuine signed receipts count (architecture 5.3) |
| T-S22 | Denial of service: 200 idle connections, a slowloris header drip, a 10 MB body, 1,000 requests a second from one IP | Limits apply. A genuine client still syncs within 5 s. Memory stays within budget |
| T-S23 | Network capture (pktmon, architecture 11): before pairing, nothing; paired with every category off, operational data only; each category on, only its schema fields; after a disable, nothing for that category | As stated |
| T-S24 | Rogue server by mDNS or name: a second host answers to the server's name | Pin mismatch, abort, message shown; the token is never sent |
| T-S25 | Fuzzing (cargo-fuzz, Linux CI, time-boxed): slip parser, control-entry decoder and chain verifier, head and receipt decoder, upload JSON | No panics. Property tests: any single-bit change to an entry or head fails verification |

**Evidence for Codex at implementation review** (architecture 5.2 implementation gates):

- the verifier source and T-S1 and T-S2 results;
- the secret-handling call sites (DPAPI, zeroize, redaction) and T-S10;
- the slip transaction and T-P3;
- every T-P, T-S, T-D and T-J result, with exact commands and the CI run.

None of this is runtime certification.

---

## 15. Crate choices

Versions are the current stable releases on crates.io on 23 Oct 2026. At implementation, `Cargo.lock` takes the newest patch of each minor version that is at least 14 days old. `cargo-deny` (advisories, licences, bans, sources) is added in V1-5 with a `deny.toml`, as architecture 9 asks.

| Purpose | Crate and version | Notes |
| --- | --- | --- |
| TLS | `rustls` 0.23 (0.23.46) | TLS 1.3 only. `default-features = false` with features `aws_lc_rs` and `std`; `tls12` and `logging` stay off |
| Crypto provider, SHA-256, Ed25519, CSPRNG | `aws-lc-rs` 1.18 (1.18.2) | rustls's default provider, actively maintained, with X25519MLKEM768. Windows MSVC x64 builds with prebuilt NASM objects and needs no CMake (aws-lc-rs build docs). One crypto library for TLS, hashing, signing and randomness. Alternative: `ring` 0.17.14 (decision C4) |
| Async TLS | `tokio-rustls` 0.26 (0.26.6) | Server acceptor |
| Runtime | `tokio` 1.5x (1.54, already in the lock) | `rt-multi-thread`, `net`, `time`, `signal` |
| Server HTTP | `axum` 0.8 (0.8.9) with `hyper` 1.12 and `hyper-util` 0.1 | HTTP/1 only, `DefaultBodyLimit`; TLS accept loop with `tokio-rustls` |
| Timeouts and limits | `tower` 0.5 (0.5.3), `tower-http` 0.7 (0.7.1) | `timeout`, `limit` |
| Companion HTTP client | `reqwest` 0.13 (0.13.5), `default-features = false`, features `rustls-no-provider` and `json` | `use_preconfigured_tls(ClientConfig)` with the pinning verifier; `no_proxy()`; HTTP/1. Fallback: `hyper-util` client |
| Certificate generation | `rcgen` 0.14 (0.14.10), feature `aws_lc_rs` | Server install only |
| Constant-time compare | `subtle` 2.6 (2.6.1, already in the lock) | Pin and verifier comparisons |
| Secret memory | `zeroize` 1.9 (1.9.1) | `Zeroizing`, plus the `Secret<T>` newtype |
| Base32 and base64url | `data-encoding` 2.11 (2.11.1) | Custom Crockford spec and `BASE64URL_NOPAD` |
| DPAPI and service | `windows` 0.62 (0.62.2, already in the lock), features `Win32_Security_Cryptography`, `Win32_Foundation`; `windows-service` 0.8 (0.8.1) | `CryptProtectData` / `CryptUnprotectData` in one module that is allowed `unsafe`, with `LocalFree` handled |
| Admin passphrase, backup container | `argon2` 0.5.3, `aes-gcm` 0.10.3 (already used by V1-4) | Reuse `hy_core::backup` |
| Store | `rusqlite` 0.40 (0.40.2, bundled, already used) | Server and companion |
| Logging | `tracing` 0.1.44, `tracing-subscriber` 0.3.23 (`json`), `tracing-appender` 0.2.5 | Allowlist layer |
| Tests | `proptest` (already used), `cargo-fuzz` 0.13.2 with `libfuzzer-sys` 0.4 | Fuzzing on Linux CI only |
| Not used | `spake2` 0.4 and `opaque-ke` 4 (see 4.1); `mdns-sd` (see F2); OpenSSL; native-tls | Rejected for the reasons given in 4.1 and 5 |

The six-word display needs a 2048-word list (11 bits per word, 66 bits for six). Proposed: the BIP-39 English list, bundled as data, with its licence checked and recorded in the licence register before V1-5 merges.

---

## 16. Conflicts and refinements against the approved architecture

None of these silently changes the architecture. Each one is either a reading of something the architecture leaves open (Clar) or an addition (New) that needs sign-off. If Codex rejects an addition, the architecture's text stands.

| # | Architecture says | This proposal | Type |
| --- | --- | --- | --- |
| F1 | Slip `hy1-<host>-<port>-<fp>-<secret>` | Same fields, parsed right to left (hyphenated host names are otherwise ambiguous), plus a 4-character typo checksum | New (format) |
| F2 | "Finds the host by mDNS or host name" | Uses the OS resolver, which already handles `.local` on Windows, plus the last-known IP and a manual address. No application mDNS listener on clients, to avoid firewall prompts | Clar |
| F3 | Six words are "an aid for humans" | They stay auxiliary to the pin. **Also** used in an administrator confirmation step that checks slip integrity (4.5). The full fingerprint stays the only authoritative pin check | New, Owner |
| F4 | The device is active as soon as it pairs | The device starts unconfirmed, and enable, delete, history and upload wait for confirmation | New, Owner |
| F5 | "Server signature" on heads | A separate Ed25519 head key with a fixed binary signing format | Clar (C5) |
| F6 | "Never replaces its receipt with a lower seq or a different chain" | The device verifies chain extensions using the entries in between, and stops on a mismatch | Clar |
| F7 | Static token; nothing said about rotation | No periodic rotation in V1. Regression and clone detection from echoed receipts | Clar, New |
| F8 | Pairings are control operations | `pairing` entries carry `token_verifier`, so post-snapshot devices authenticate after replay | Clar |
| F9 | Any restore enters hold | Also detected automatically at startup (database vs mirror high-water). A restore voids outstanding slips | New |
| F10 | Each user's policy is shared across that user's PCs | A remote enable needs confirmation on each PC before that PC uploads | New, Owner (UX) |
| F11 | Server: machine-scope DPAPI | Kept, plus a folder ACL and a virtual service account, because machine scope alone lets any local user decrypt. TPM or CNG binding of device credentials is deferred | Clar |
| F12 | Shared summaries unencrypted (Q4) | Unchanged for the database. **Backups encrypted** with a machine-bound key | New |
| F13 | "The server keeps data no longer than the device" | Server daily summaries pruned at 90 days; no monthly rows on the server | Clar |
| F14 | Admin page localhost and Argon2id | Plus a separate listener, CSRF protection, a `Host` and `Origin` allowlist and backoff | Clar |
| F15 | `consent_log` kinds fixed in v1 | Migration 0005 rebuilds the table for new kinds | Clar (implementation) |
| F16 | V1-5 is 3.5 to 5 build days | The additions add about 1 to 1.5 days (confirmation, extension checks, regression, backup encryption, hardening, T-S tests). Suggested split into V1-5a (server core, pairing, transport, sync) and V1-5b (control log, mirror, receipts, hold, backup). Scheduling is the owner's decision | Owner |

Document location: the architecture and PRD 0.3 live on the architecture branch (PR #3), not on `main`. This document links to them by name. The owner may want them merged to `main` alongside this document, so the V1-5 baseline is in one place.

---

## 17. Decisions needing Codex sign-off

Each item asks for **accept**, **change** or **reject**. V1-5 starts only when every item is accepted or changed.

| # | Decision | Recommended |
| --- | --- | --- |
| C1 | Pairing route: pinned TLS 1.3 plus an 80-bit single-use slip secret; PAKE rejected (4.1) | Accept |
| C2 | Slip encoding: right-to-left parse, Crockford base32, 4-character checksum, 24 h, at most 3 outstanding per user, generic `slip_invalid`, rate limits (4.3, 4.4) | Accept |
| C3 | Pinning verifier and TLS profile: full SHA-256 of the DER, constant-time compare, delegated TLS 1.3 signature check, TLS 1.2 refused, no 0-RTT, no resumption, ALPN `hy/1`, constant SNI, names and dates ignored, HTTP/1.1, no proxy (5) | Accept |
| C4 | Crypto provider: aws-lc-rs (rather than ring) | Accept aws-lc-rs |
| C5 | A separate Ed25519 head-signing key, delivered at pairing over the pinned channel; the `HY-HEAD1` binary message (6.2, 7.2) | Accept (or sign with the TLS key under a domain prefix) |
| C6 | Control-entry encoding and per-user chain; mirror format; `mirror_hwm` before acknowledgement; the startup comparison table (7.1, 7.3) | Accept |
| C7 | Device-side extension verification and the "server history mismatch" state (7.4) | Accept |
| C8 | `token_verifier` in `pairing` entries; restore voids slips; unconfirmed devices count as paired in evidence (9) | Accept |
| C9 | No periodic token rotation in V1; regression and clone detection by echoed receipt; revocation and self-unpair semantics (6.3, 7.4) | Accept |
| C10 | Secret handling: client DPAPI user scope with entropy; server machine scope plus ACL plus virtual account; `Secret<T>` with zeroize; keys never backed up (6) | Accept |
| C11 | Admin page hardening (TH15, F14) | Accept |
| C12 | Upload validation rules and per-category strict schemas (8.2, 8.6) | Accept |
| C13 | Remote enable needs confirmation on each PC (8.3), subject to the owner's UX answer (O2) | Accept |
| C14 | Device confirmation gate (4.5), subject to the owner's answer (O1). Confirms the six words are used only as a slip-integrity check and never replace the full pin | Accept |
| C15 | At rest: database unencrypted (as disclosed), backups encrypted, `secure_delete` on (10) | Accept |
| C16 | Logging allowlist and canary tests (11, T-S10) | Accept |
| C17 | The test plan (14) is sufficient as the implementation evidence named in architecture 5.2 | Accept |
| C18 | Crate list (15), with `cargo-deny` added in V1-5 | Accept |

---

## 18. Decisions needing the owner

These are product or operational choices, not security mechanisms. Each has a recommended answer, and nothing is assumed until the owner answers.

| # | Question | Options | Recommended |
| --- | --- | --- | --- |
| O1 | **Confirming new PCs.** After pairing, should the administrator confirm each new PC by comparing six words the employee reads out (in person or by phone) before that PC can share or restore anything? | (a) Yes: slips may then be sent over internal chat. (b) No: slips must be handed over printed or in person, and a PC is active as soon as it pairs | (a). It catches a tampered or stolen slip and costs about a minute per PC |
| O2 | **Several PCs per person.** When sharing is switched on from one PC, should each of that person's other PCs ask before sharing from it too? | (a) Ask on each PC. (b) Apply to every PC automatically | (a). Consent is shown on the PC that will send the data |
| O3 | **Server PC disk encryption.** Ask for BitLocker on the server PC as a deployment check? | Yes, or no (record as a known risk) | Yes, if the PC supports it. Not a blocker |
| O4 | **Replacement server hardware.** If the server PC is replaced or reinstalled, every PC re-pairs and shared history restarts. A key backup file would avoid that, but anyone who copied it could impersonate the server | (a) Re-pair (no key backup). (b) A passphrase-protected key backup held by the administrator | (a). The 7-day hold and per-user rules stay as decided on 21 Oct |
| O5 | **Schedule.** The additions add about 1 to 1.5 build days to V1-5, which may be split into two PRs (F16) | Accept, or drop items (for example O1 and O2) | Accept |
| (existing) | Q3: a named firm contact and confirmed notice wording before the pilot. Q4: the disclosure-only default stays. A1: mirror location and server deployment checks on install day | n/a | Unchanged |

The 7-day maximum hold (Q9) is already decided and is not reopened here.

---

## Appendix A. Error enum (wire)

`slip_invalid`, `slip_malformed` (client only), `rate_limited`, `origin_conflict`, `unknown_token`, `device_revoked`, `device_unconfirmed`, `device_regressed`, `stale_enable`, `recovery_hold`, `accepted_pending_recovery`, `category_off`, `before_share_from`, `tombstoned`, `date_out_of_range`, `schema_invalid`, `revision_not_higher`, `origin_mismatch`, `cannot_record`, `payload_too_large`, `protocol_unsupported`.

## Appendix B. Domain-separation strings

`HY-SLIP1`, `HY-SLIPCHK1`, `HY-TOK1`, `HY-KEYID1\0`, `HY-CL1\0`, `HY-CL1-GENESIS\0`, `HY-HEAD1\0`, ALPN `hy/1`, SNI `hy-server.invalid`, DPAPI entropy `HealthierYou/server_link/v1`. Every one is versioned, so a later format change cannot be confused with v1.
