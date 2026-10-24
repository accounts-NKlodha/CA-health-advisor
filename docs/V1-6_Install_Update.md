# V1-6 install, update and uninstall

Status: **slices V1-6a (the companion) and V1-6b (the office server)** (24 Oct 2026). V1-6a covers the companion's per-user installer, start at sign-in, updating from a signed file with rollback, and a clean uninstall. V1-6b (section 8, a stacked PR) covers the server's per-machine installer and Windows service. Baseline: architecture section 8 (`docs/Architecture_V1_V2.md` on the architecture branch), assumptions A5, A15 and A16, and the V1-5 contract 6.2 for the server.

## 1. The companion installer

- One file per release: `HealthierYou_<version>_x64-setup.exe`. It is the Tauri 2.12 NSIS installer, with the Microsoft **fixed WebView2 runtime** bundled. Nothing is downloaded at install time, and the PC's own WebView2 is not used. That is about 180 MB installed (A16).
- **Per user, no administrator rights.** It installs to `%LOCALAPPDATA%\HealthierYou`. The uninstall entry goes under `HKCU` only, so it never asks for UAC.
- **Start at sign-in** uses the per-user `Run` value (A15). It can be switched off in Settings > App & updates. That choice is a marker file in the data folder, so a reinstall or an update keeps it. A choice made in Task Manager is kept by Windows itself.
- **Display scaling.** The companion's manifest declares per-monitor DPI awareness (v2), with common controls v6. The installer was already DPI-aware. The CI self-test fails if the running app is not per-monitor aware. Visual checks at 100/125/150/200% need a real screen (section 6).

Folders under `%LOCALAPPDATA%\HealthierYou`:

| Folder | What | On uninstall |
| --- | --- | --- |
| `hy-companion.exe`, `uninstall.exe`, `webview2-fixed\` | the program | removed |
| `updates\` | update record, staged installer, previous-version copy, update log (no personal data) | removed |
| `data\` | personal data (`hy.db`, its pre-migration copies) | kept, unless "Also delete my data" is ticked (unticked by default, folder shown) or a silent uninstall passes `/DELETEDATA` |

## 2. Updating from a file, and rollback

The process is in Settings > App & updates > **Choose update file...** (or on the command line, `hy-companion.exe --hy-update-check|--hy-update-install <installer> [--report <file>]`, which CI uses):

1. **Check before anything runs.** The installer must have its `.sig` file beside it, a minisign signature made with `tauri signer sign --app-version <version>`. The companion checks, in order, that:
   - the file name is `HealthierYou_<version>_x64-setup.exe`, and not the server's installer;
   - the signature is from a key built into this release (`release/update-keys.txt`);
   - the signature's trusted comment names this exact file and version, so a renamed or re-versioned file is refused;
   - the whole file matches the signature.

   Old-format (non-prehashed) signatures are refused. A build with no key refuses every file.
2. **What is allowed.** A newer version is allowed. An older version is allowed only if it is exactly the version this PC updated from ("Go back to ..."), so rollback is one release back. The installed version itself is refused.
3. **Install and restart.** The file is copied to `updates\staged` with its downloaded-file mark kept, and then checked again. The companion then closes and a helper copy of it takes over. The helper:
   - copies the program folder to `updates\previous`;
   - exports the uninstall entry;
   - checks the staged installer and its signature again, just before it runs (time has passed since the first check, and the updates folder is the user's own; nothing makes the staged file immutable);
   - runs the installer silently with `/UPDATE`;
   - starts the new version, which must show its pet page within 2 minutes.
4. **Rollback.** If the installer fails, or the new version does not confirm its start, the helper:
   - stops the new version;
   - restores the program folder and the uninstall entry;
   - starts the old version.

   App & updates then says "The update to X did not start, so HealthierYou went back to Y". The personal data is not touched by an update or a rollback.
5. **Data.** Before a schema migration, the store keeps a copy of the database (pre-migration copy, V1-0). If an older version finds a newer database after going back, it offers to restore that copy. The restore is failure-safe:
   - the copy is copied beside the database, flushed and checked (whole, with a readable schema) before anything else changes;
   - the newer database is folded into one file and renamed to `hy.db.newer-<time>.bak` (never deleted);
   - the checked copy is renamed into place.

   If the copy cannot be read, the copy fails part way, or the copy is damaged, nothing changes. If the final rename fails, the newer database is moved back. Only if that fails too is the database left set aside, with a marker beside it; the next start then offers to put it back and never opens (so never creates) an empty database. The message says which of these happened.

**An attempt cut short.** A helper holds a named mutex while it runs. When the companion starts with an attempt recorded and no helper running:
- if it is the attempt's target version and the installer had run (the PC restarted, or the helper was ended, after the install), the update is recorded as done, the previous-version copy is cleared and going back stays possible;
- otherwise the attempt is recorded once as not finished, with the version that is running.

A just-staged attempt is left alone for two minutes (its helper may still be starting). So an orphaned attempt never blocks later updates. The helper's log is `updates\update.log`.

### Releasing (the owner, offline)

1. Once: `npx tauri signer generate -w <offline folder>\healthieryou-update.key` (choose a password). Commit **only** the `.pub` file's single line to `release/update-keys.txt`. The secret key and its password never go in the repository, in CI or in a chat. More than one key can be listed, which allows a key change across a release.
2. Each release: build the installer (`tauri build --config src-tauri/tauri.installer.json`, with `webview2-fixed\` fetched by `scripts/windows/Get-WebView2FixedRuntime.ps1`). Then run `npx tauri signer sign -f <key> --app-version <version> HealthierYou_<version>_x64-setup.exe`. Give staff both files.
3. CI builds with keys made for each run and thrown away. CI installers can never update a real install.

## 3. Uninstall

The uninstall can be started from Windows Settings > Installed apps, or by `uninstall.exe` (silent: `/S`, add `/DELETEDATA` to remove personal data). It removes:

- the uninstall entry;
- the `HKCU\Software\HealthierYou` install-location key. The Tauri template removed this key only when the data box was ticked, which was the registry residue that feasibility batch 1 found;
- the `Run` value and Task Manager's `StartupApproved` record;
- the program, the bundled runtime and `updates\`;
- WebView2's cache (`%LOCALAPPDATA%\app.healthieryou.companion`, no personal data).

`%LOCALAPPDATA%\HealthierYou` itself goes only when nothing is left in it. If the data is kept, only `data\` remains.

Batch 1's residue may also have been partly a measurement race. The NSIS uninstaller copies itself to the temp folder (`Au_.exe`) and returns at once. The CI test waits for that copy to finish before checking.

## 4. Unsigned installer: what staff will see

There is no paid code signing (A5). This is what Windows shows, and it is never bypassed or suppressed by the product:

- **Downloaded through a browser or email** (the file carries the "Mark of the Web"): **"Windows protected your PC. Microsoft Defender SmartScreen prevented an unrecognized app from starting."** Click **More info**: it shows **"Publisher: Unknown publisher"** and a **Run anyway** button. Some browsers first warn that the file "isn't commonly downloaded" (Keep > Keep anyway).
- **Copied from a LAN share or USB**: usually no SmartScreen. Windows may show **"Open File - Security Warning ... The publisher could not be verified"** (Run).
- **Updating from App & updates**: the companion keeps the mark on its staged copy and runs the installer through Windows' shell. Windows applies the same checks as a double-click. A marked file is flagged in the notice: Windows may show "Windows protected your PC" or "Unknown publisher", and HealthierYou does not switch that check off.
- **Smart App Control** (Windows 11, when on) or an office policy can **block** an unsigned installer with no "Run anyway". The product then reports that Windows did not start the installer, that nothing was changed and that the owner decides about signing. The current version keeps running. Under A5 this is a **signing decision** for the owner (buy signing, or have IT allow the file). Never turn off a control.

Recommended distribution: copy both files to the office share. The office PCs then usually see no SmartScreen prompt, and the signature check is the same.

## 5. CI coverage (Windows job)

The CI job does the following:

- **Builds.** It fetches the Microsoft-signed fixed runtime (its Authenticode signature is checked), generates two keys for the run, and builds and signs three installers:
  - A 0.1.0;
  - B 0.1.1;
  - C 0.1.2, whose bundled runtime is a fake, so it cannot start.
- **Runs** `scripts/windows/Test-CompanionLifecycle.ps1` on the runner:
  - **Install A**, silent and per user. It checks the files, that the entry is under HKCU only, and the `Run` value. It then runs the existing self-test on the **installed** app, which must use the bundled runtime and be per-monitor DPI aware.
  - **Update A → B**: the outcome is updated; the entry and the exe are B; the previous version is recorded as A; B confirmed its start.
  - **Update B → C**: rolled back; the exe, the uninstaller and the entry are back to B; B is running again.
  - **Refused**, with nothing changed: one changed byte, a signature from the other key, a renamed file, no `.sig`, the installed version, and an install while the app runs.
  - **Mark of the Web**: a Zone 3 mark on the file is detected and left in place.
  - **Rollback B → A** from its file; nothing older may follow.
  - **Uninstall**: no uninstall entry, `HKCU\Software\HealthierYou`, `Run` or `StartupApproved` value, program, updates or WebView2 cache left; the data is kept.
  - **Reinstall** keeps an "off" start-at-sign-in choice.
  - **Uninstall with `/DELETEDATA`**: nothing left.

## 6. Needs physical testing

A CI runner is an administrator account on Windows Server, with no SmartScreen reputation, no Smart App Control and no real sign-in. These checks need an office PC. Run them with `scripts/windows/Test-HealthierYou-OfficePC.ps1` (guided). It writes a report with no user name, PC name or path.

1. Install on Windows 10 and Windows 11 as a **standard user**: no UAC prompt (I4).
2. SmartScreen with a **browser-downloaded** copy against a **LAN/USB** copy: the exact wording, and that Run anyway works.
3. Smart App Control on (a Windows 11 clean install) or an office AppLocker/WDAC policy: whether the installer is blocked. If it is, that is a signing decision (A5).
4. Display scale at 100, 125, 150 and 200%, and two monitors with different scales: the pet and Settings are sharp and placed correctly.
5. Start at sign-in after a real sign-out and sign-in. It is allowed by office policy (A15), shows as enabled in Task Manager, and works after sleep and wake.
6. Update from file and roll back from file in Settings, with the real UI, on a downloaded (marked) file.
7. Uninstall from Windows Settings: the data box is unticked and shows the folder, and nothing is left in Installed apps or the registry.
8. Antivirus or EDR on the office PCs does not quarantine the unsigned installer, the update helper copy in `updates\helper`, or the bundled runtime.

## 7. Deviations and readings

- **Updates are from a file only in V1-6a. Server-delivered updates remain required, approved work: they are deferred, not dropped.** Architecture section 8 also describes getting the update file from the office server. That needs a server endpoint and is tracked as open work for a later slice. The file format and the check are the same, so the server only has to hand over the same two files.
- **No updateable release exists yet.** `release/update-keys.txt` holds no production public key, so a release build refuses every update file. CI's passing signatures use keys made for each run and thrown away. The owner must generate the key offline and commit its public half (section 2) before any release can be updated.
- **"Rollback one release back"** is read as: the only older version accepted is the exact version this PC last updated from. After a rollback, nothing older is accepted until the next update.
- The previous-version program copy is deleted once the new version confirms its start. Going back after that uses the old installer file and its signature.
- The program folder and the data folder share the parent `%LOCALAPPDATA%\HealthierYou`, which is the Tauri template's install path. The uninstaller removes everything but `data\`.
- The "display scaling" defect had no written record in the repository. It is handled as declaring per-monitor-v2 DPI awareness, with the self-test check and the physical checks above.

## 8. The office server (V1-6b)

One file per release: `HealthierYou-Server_<version>_x64-setup.exe` (NSIS, per machine, built by `scripts/windows/Build-ServerInstaller.ps1`), with its `.sig` made the same way as the companion's. An administrator runs it on the server PC; silent: `setup.exe /S [/MIRROR=D:\HealthierYou-Mirror]` (without `/MIRROR` it uses the installed or remembered mirror).

**What it sets up (V1-5 contract 6.2):**

| Item | Setting |
| --- | --- |
| Program | `%ProgramFiles%\HealthierYou Server\hy-server.exe`, uninstall entry under HKLM |
| Service | `HealthierYouServer`, `hy-server.exe service --mirror <folder>\control-log.mirror`, account `NT SERVICE\HealthierYouServer` (virtual account, no password), automatic (delayed) start, restart after 1 and 1 minutes then 5 minutes; reports *running* only once both ports are open |
| Data | `%ProgramData%\HealthierYou\Server`, exact rights on the whole tree (below) |
| Keys | made by the service itself at its first start (TLS and head keys sealed with machine-scope DPAPI); an administrator can still run `hy-server fingerprint` and `set-passphrase` |
| Mirror | the folder given by `/MIRROR=` (or on the setup page) on another volume, bound to the data (below); same rights where Windows allows (a share's owner sets its rights) |
| Firewall | one inbound allow rule "HealthierYou Server": TCP 8443, `hy-server.exe` only, private and domain profiles only (never public). The admin page (8444) listens on loopback only and gets no rule |
| Registry | `HKLM\Software\HealthierYou\Server`: `Version`, `PreviousVersion` (written only after a good start), `MirrorFolder` |

**Exact data-folder rights, also on a folder that already exists.** Before the program is put in place, with the service stopped, the installer: makes Administrators the owner of every file and folder; drops every entry the folder itself has, stops inheritance and grants only SYSTEM (`S-1-5-18`) and Administrators (`S-1-5-32-544`) full control and the service account modify; resets every file and folder under it so it has no entries of its own and takes the folder's. Then `hy-server check-acl` reads the whole tree back: any owner other than those three accounts, any allow entry for another account (explicit or inherited), a folder list that still inherits, or a file with no list at all fails the install closed (exit 8; on an upgrade nothing is replaced and the previous version is started again). For a moment between "drop" and "stop inheritance" the folder inherits `%ProgramData%`'s rights; the service is stopped then and setup runs as an administrator.

**The data stays bound to its mirror.** The database records which mirror it belongs to (V1-5 contract 7.3); pointed at another (say an empty) mirror, the next start would take the global discard path. So the mirror folder is remembered in `mirror-location.txt` in the data folder (kept on uninstall, so a reinstall over kept data uses it by default), and before the service is pointed at any mirror the trusted server (the installed one on an upgrade, the installer's own on a fresh install) runs `hy-server check-mirror`: it passes only when there is no data yet, the data has never had a mirror, or the mirror file at that location is the data's own (same mirror id, this server's key, intact chain). Anything else (a missing or empty folder, another server's mirror, a damaged copy) is refused with nothing changed (exit 6). **Moving the mirror** is therefore an explicit, verified step: copy `control-log.mirror` to the new folder, then run setup with `/MIRROR=` that folder; the check accepts the copy because it is the data's own.

**Version policy (enforced by `hy-server verify-release`).** The installed server accepts a signed server installer whose version is newer than the installed one, or exactly the version it was last updated from (`PreviousVersion`, a rollback). The same version again, or any other older version, is refused (exit 3). Going back keeps the data. If the newer version changed the database in a way the older one cannot read, the older one does not start, and setup handles it like any release that does not start (steps 6 and 7 below): the newer program and the database copy made just before are put back.

**Upgrade and rollback.**

1. The **installed** server checks the new installer, its `.sig` and the version policy (`hy-server verify-release --current --previous`): same keys, name and file rules as the companion. A refused file changes nothing (exit 3). If copies from an earlier update that did not complete are still in `pre-update\`, setup stops here (exit 7): they may be the only good database.
2. The service is stopped and must be **confirmed stopped**, otherwise nothing changes (exit 7). The mirror binding is checked (exit 6 on refusal; the service is started again).
3. Copies are made and each is compared byte for byte (`fc /b`) before anything is replaced: `hy-server.exe` as `hy-server.previous.exe`, and `server.db` with its `server.db-wal` (if any) in `pre-update\`. If any copy cannot be made or does not match, the previous version is started again and nothing changed (exit 7).
4. The data-folder rights are set and verified (exit 8 on failure, nothing replaced).
5. The new program is put in place (verified) and started. If it is running five seconds later, the copies are removed, `Version`/`PreviousVersion` are written, done.
6. Otherwise: the service is stopped (confirmed), the previous program is put back (verified) and started. If it runs: exit 4, the data as it is.
7. If it does not start (for example the new version changed the database so the old one refuses it): the service is disabled while the database is changed (so nothing can start it and make an empty database), the failed version's `server.db`, `-wal` and `-shm` are moved together into `failed-update-<version>\` (each move checked; a database is never left apart from its WAL), the `pre-update\` copy is put back and verified, the service is re-enabled and started: exit 4.
8. Any step of 6 or 7 that fails stops with **exit 9** and a message that says exactly what is where (which program is installed, where the previous program, the database copy and the failed database are, whether the service was disabled). Nothing is deleted. **Recovery after exit 9:** stop the service; move `server.db*` out of the data folder; copy the files in `pre-update\` into the data folder; move `pre-update\` out of the data folder; set the service to automatic (delayed) and start it. (CI runs exactly this.)

After a good start or a completed rollback the previous copy and the pre-update copy are removed. The keys never change, so no PC re-pairs.

**Bootstrap limitation (first-install provenance).** The first install, and a reinstall after an uninstall, has no installed server to check the signature or the version with: whatever installer the administrator runs is trusted. The administrator must check the file came from the owner (the same file as on the share, with its `.sig`, compared with the SHA-256 the owner publishes). Every later upgrade is checked by the installed server. Production update public keys are not yet built into any release (section 7), so no server release is updateable yet either.

**Uninstall** (Windows Settings or `uninstall.exe`, silent `/S`): stops and deletes the service, removes the firewall rule, the program, the uninstall entry and `HKLM\Software\HealthierYou\Server`. The data (keys, database, backups, `mirror-location.txt`) and the mirror are kept unless "Also delete the server data" is ticked (unticked by default; it warns that without the keys every PC re-pairs) or `/DELETEDATA` is passed. A reinstall over kept data reuses the keys and the remembered mirror.

**Exit codes:** 0 done; 2 bad options, not an administrator, or the program could not be written; 3 refused (signature or version), nothing changed; 4 the new version did not start and the previous version was put back and runs; 5 the service did not start (fresh install); 6 the mirror is not the data's own, nothing changed; 7 not confirmed stopped, copies not made and verified, or an earlier incomplete update's copies present, nothing changed; 8 the data-folder rights could not be set or verified, nothing replaced; 9 the rollback could not complete (the message says what is where; nothing deleted).

**CI** (Windows job, administrator): builds S1 0.1.0, S2 0.1.1, S3 0.1.2 (whose `hy-server.exe` is not a program) and S4 0.1.3 (the real server built with the CI-only `HY_SERVER_TEST_SCHEMA_CHANGE_THEN_FAIL` hook: it applies a Contract migration older servers refuse, leaves it in the WAL and fails to start; the hook is compiled in only when that variable is set at build time and the migration is not one of the server's own), and runs `scripts/windows/Test-ServerLifecycle.ps1` over a data folder and mirror seeded by `hy-sim seed` (5 users, 10 devices, 60 summaries), counting them with `hy-sim count` after every step that keeps data:

| Test | Shows |
| --- | --- |
| install over the seeded folder after giving Users explicit entries, a file owned by Users and a non-inheriting Everyone file | exact rights: `check-acl` and an independent PowerShell walk find no other owner or entry; the kept keys are used |
| service, firewall, ports | as before |
| other key, no `.sig`, same version again | exit 3, nothing changed |
| upgrade with another `/MIRROR` | exit 6, program, registry and service mirror unchanged, data counts unchanged |
| S1 to S2 | data counts unchanged, `Version`/`PreviousVersion`, rights verified |
| `hy-server.previous.exe` locked | exit 7, nothing replaced, service running again |
| S3 | exit 4, previous program back and running, data unchanged |
| S4 with the failed-update folder blocked | exit 9, previous program back, database and both copies kept; a new setup refuses (exit 7); the documented recovery brings the same data back |
| S4 | exit 4, previous program and pre-update database back and running with the same counts; the failed version's change kept with its database in `failed-update-0.1.3\` |
| S2 back to S1 | allowed (the previous version) |
| uninstall, then reinstall with another `/MIRROR` | exit 6, nothing installed, data unchanged |
| reinstall without `/MIRROR` over a folder opened up again | remembered mirror used, rights exact, data unchanged |
| copy the mirror, reinstall with `/MIRROR=` the copy | accepted (verified move), remembered |
| uninstall `/DELETEDATA`, fresh install | data and mirror removed; new keys made by the service; rights exact |

**Needs physical testing (server):**

1. Install on the real office server PC (Windows 10/11 Pro, or Server) as a domain or local administrator; the setup's SmartScreen and "Unknown publisher" warnings as in section 4.
2. A companion on another office PC connects through the firewall rule on the office network (private or domain profile); confirm the network profile is not *Public* (the rule deliberately does not open it).
3. The mirror on the real second disk or share: on a share, give the computer account (`DOMAIN\SERVERPC$`) modify rights, since the virtual account acts as the computer on the network.
4. Power settings: the server PC must not sleep during office hours (Settings > System > Power: sleep "Never" when plugged in); the service restarts after a reboot (delayed start) and after a crash.
5. Upgrade with a real downloaded installer, and uninstall from Windows Settings with the data box left unticked.
6. Moving the mirror to the real share (copy `control-log.mirror`, setup with `/MIRROR=`): `check-mirror` reads it over SMB as the administrator, the service then writes it as the computer account.
7. An install over a data folder an administrator copied or restored by hand on that PC (domain owners, inherited domain groups): `check-acl` passes after setup.

**Deviation:** the server takes update files only from the administrator running the next installer (no automatic channel).
