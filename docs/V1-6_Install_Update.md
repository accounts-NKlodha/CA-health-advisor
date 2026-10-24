# V1-6 install, update and uninstall

Status: **slice V1-6a, the companion** (24 Oct 2026). This covers the per-user installer, start at sign-in, updating from a signed file with rollback, and a clean uninstall. The office server's installer and Windows service are slice V1-6b (a stacked PR). Baseline: architecture section 8 (`docs/Architecture_V1_V2.md` on the architecture branch), assumptions A5, A15 and A16.

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
5. **Data.** Before a schema migration, the store keeps a copy of the database (pre-migration copy, V1-0). If an older version finds a newer database after going back, it offers to restore that copy. The restore is failure- and crash-safe:
   - the copy is copied beside the database, flushed and checked (whole, with a readable schema) before anything else changes;
   - the newer database is folded into one file;
   - a recovery record (`hy.db.restore-journal`, naming both files) is written, flushed and its folder flushed **before anything is moved**; if it cannot be saved, nothing changes;
   - the newer database is renamed to `hy.db.newer-<time>.bak` (never deleted), then the checked copy is renamed into place, then the record is removed.

   If the copy cannot be read, fails part way or is damaged, nothing changes. If the final rename fails, the newer database is moved back. **At every start, before the database is opened,** a recovery record left by a crash or power loss at any point, or by an undo that failed, is reconciled from the state of the files: not yet moved (nothing changed), set aside but not swapped (the checked copy is put in place if still whole, otherwise the newer database is put back), or completed (the record is cleared). A record that cannot be read, or files in a state no restore leaves, stop the companion with a message **without opening, and so without creating, a database**. The message says which of these happened.

**An attempt cut short.** A helper holds a named mutex while it runs. When the companion starts with an attempt recorded and no helper running:
- if it is the attempt's target version and the installer had run (the PC restarted, or the helper was ended, after the install), the update is recorded as done only once that version **confirms its start** (its pet page loads, the same confirmation the helper waits for); only then is the previous-version copy cleared, and going back stays possible. Merely starting as the target settles nothing: if its window fails before the page loads, the attempt and the copy to go back to are kept;
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
