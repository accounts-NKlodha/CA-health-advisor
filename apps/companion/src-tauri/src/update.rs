//! Install an update from a file, keep the previous version and put it back if the new
//! one does not start (architecture 8; slice V1-6). The rules are in `hy_core::update`.
//!
//! The flow, from Settings > App & updates > "Install update from file":
//!
//! 1. The user picks the installer in the standard Open dialog; its `.sig` must sit next
//!    to it. The signature is checked against the keys built into this app, and the
//!    version against [`hy_core::update::plan`] (newer, or the version this PC came
//!    from). Nothing has changed yet; the page shows what would happen.
//! 2. "Install and restart": the installer and its signature are copied into the updates
//!    folder and the **copy** is checked again (so a file swapped after the first check is
//!    refused). Windows' downloaded-file mark (Mark of the Web) travels with the copy. The
//!    update record gets the attempt, a copy of this program becomes the update helper,
//!    the helper starts, and the app closes normally (the coaching engine saves first).
//! 3. The helper waits for the app to close, copies the program files aside, exports the
//!    uninstall entry, and runs the installer silently through the shell, so Windows
//!    checks it as it would on a double-click (SmartScreen is never bypassed). It then
//!    starts the new version and waits for it to confirm (its pet page loaded, which
//!    proves the bundled WebView2 runtime works).
//! 4. No confirmation within [`START_WAIT`], an early exit or a failed installer: the
//!    helper stops the new version, puts the previous program files and uninstall entry
//!    back and starts the previous version. Settings then says what happened. If Windows
//!    or the user stopped the installer before it ran, nothing was changed.
//!
//! The same check and install run from the command line for scripted installs and the CI
//! lifecycle test: `hy-companion.exe --hy-update-check <installer> [--report <file>]` and
//! `--hy-update-install <installer> [--report <file>]` (refused while the app runs).
//!
//! Personal data is never part of the program copy: the store stays where it is, and
//! migrations keep the previous release able to open it (expand, then contract).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use hy_core::update::{
    self, Attempt, Confirmation, OutcomeKind, Phase, Plan, Product, TrustedKeys, UpdateState,
    VerifiedRelease, Version,
};
use serde::Serialize;

/// The update public keys built into this program (`build.rs`: the `HY_UPDATE_PUBKEY`
/// environment variable at build time, else `release/update-keys.txt`). Empty in a build
/// with no key, which then refuses every update file.
pub const BUILT_IN_KEYS: &str = env!("HY_UPDATE_KEYS");

/// The program's entries in the install folder: what is copied aside before an update and
/// put back by a rollback. The install folder also holds the user's data folder, which is
/// never touched here.
pub const PROGRAM_ENTRIES: &[&str] = &["hy-companion.exe", "uninstall.exe", "webview2-fixed"];

/// The name of the uninstall entry and the Run value (the product name).
pub const PRODUCT_NAME: &str = "HealthierYou";

/// How long the new version has to confirm it started.
pub const START_WAIT: Duration = Duration::from_secs(120);
/// How long the helper waits for the app to close.
const CLOSE_WAIT: Duration = Duration::from_secs(60);
/// A staged attempt younger than this with no helper is left alone: its helper may still
/// be starting.
const STAGED_GRACE: Duration = Duration::from_secs(2 * 60);
/// The marker file in the data folder that says the user switched start at sign-in off,
/// so a reinstall does not switch it on again (read by the installer hooks).
pub const AUTOSTART_OFF_MARKER: &str = "autostart-off";

/// Command-line switches.
const ARG_CHECK: &str = "--hy-update-check";
const ARG_INSTALL: &str = "--hy-update-install";
const ARG_HELPER: &str = "--hy-update-helper";
const ARG_REPORT: &str = "--report";

/// The per-user folders.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// `%LOCALAPPDATA%\HealthierYou\updates`: the update record, staged installers, the program copy
    /// and the helper. Program state, removed by the uninstaller.
    pub updates: PathBuf,
    /// `...\HealthierYou\data`: personal data.
    pub data: PathBuf,
}

impl Dirs {
    /// The folders under `local_data` (`%LOCALAPPDATA%`).
    #[must_use]
    pub fn under(local_data: &Path) -> Self {
        let root = local_data.join("HealthierYou");
        Self {
            updates: root.join("updates"),
            data: root.join("data"),
        }
    }

    /// The folders for the signed-in user, from `%LOCALAPPDATA%`.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        std::env::var_os("LOCALAPPDATA").map(|d| Self::under(Path::new(&d)))
    }

    fn staged(&self) -> PathBuf {
        self.updates.join("staged")
    }

    fn previous(&self) -> PathBuf {
        self.updates.join("previous")
    }

    fn helper(&self) -> PathBuf {
        self.updates.join("helper").join("hy-update-helper.exe")
    }

    fn uninstall_entry(&self) -> PathBuf {
        self.updates.join("uninstall-entry.reg")
    }

    fn log(&self) -> PathBuf {
        self.updates.join("update.log")
    }
}

/// The folder this program runs from, if it is an installed copy (the uninstaller sits
/// next to it). A development build never updates itself.
#[must_use]
pub fn install_dir() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    dir.join("uninstall.exe").is_file().then_some(dir)
}

/// The Windows zone from a file's `Zone.Identifier` stream (Mark of the Web): 3 is the
/// internet, 4 restricted sites.
#[must_use]
pub fn zone_from_identifier(text: &str) -> Option<u32> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("ZoneId="))
        .and_then(|z| z.trim().parse().ok())
}

/// Whether Windows marked `file` as coming from the internet (zone 3 or above).
#[must_use]
pub fn marked_from_internet(file: &Path) -> bool {
    let mut stream = file.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::read_to_string(PathBuf::from(stream))
        .ok()
        .and_then(|t| zone_from_identifier(&t))
        .is_some_and(|z| z >= 3)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Append a line to the update log. It holds versions, phases and outcomes only: no
/// paths and no user names (architecture 10).
fn log(dirs: &Dirs, line: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(&dirs.updates);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dirs.log())
    {
        let _ = writeln!(f, "{} {line}", chrono::Utc::now().to_rfc3339());
    }
}

/// A checked update file, as the page shows it.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheck {
    /// The installed version.
    pub current: String,
    /// The offered version.
    pub version: String,
    /// Upgrade or rollback.
    pub plan: Plan,
    /// The installer's size, MB (one decimal).
    pub size_mb: f64,
    /// Whether Windows marked the file as downloaded (it may show its own warning).
    pub marked_from_internet: bool,
}

/// The last update's outcome, as the page shows it.
#[derive(Debug, Clone, Serialize)]
pub struct OutcomeView {
    /// What happened.
    pub kind: OutcomeKind,
    /// The version before.
    pub from: String,
    /// The version offered.
    pub to: String,
    /// When, local time, `YYYY-MM-DD HH:MM`.
    pub at: String,
    /// A short explanation.
    pub detail: String,
}

/// Start at sign-in.
#[derive(Debug, Clone, Serialize)]
pub struct AutostartView {
    /// The Run value is there.
    pub enabled: bool,
    /// The user switched it off in Windows (Task Manager or Settings), which wins.
    pub switched_off_in_windows: bool,
}

/// Settings > App & updates.
#[derive(Debug, Clone, Serialize)]
pub struct AppPage {
    /// This version.
    pub version: String,
    /// Where the program is installed (None for a development build).
    pub install_dir: Option<String>,
    /// Where personal data lives.
    pub data_dir: String,
    /// Why update files cannot be installed, if they cannot.
    pub update_blocked: Option<String>,
    /// The version that may be put back, if any.
    pub previous_version: Option<String>,
    /// An update is in progress.
    pub in_progress: bool,
    /// The last update's outcome.
    pub last: Option<OutcomeView>,
    /// Start at sign-in (None outside an installed copy).
    pub autostart: Option<AutostartView>,
}

#[derive(Debug, Clone)]
struct Chosen {
    path: PathBuf,
    release: VerifiedRelease,
    plan: Plan,
    marked: bool,
}

/// Whether an update helper is running in this session.
fn helper_running() -> bool {
    #[cfg(windows)]
    {
        crate::win32::update_helper_running()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The update manager, kept by the app (`app.manage`).
#[derive(Debug)]
pub struct Updater {
    version: Version,
    keys: Result<TrustedKeys, String>,
    dirs: Dirs,
    install: Option<PathBuf>,
    chosen: Mutex<Option<Chosen>>,
    confirmed: Mutex<bool>,
}

impl Updater {
    /// The manager for this program at `version`, with the built-in keys.
    #[must_use]
    pub fn new(version: Version, dirs: Dirs) -> Self {
        let me = Self {
            version,
            keys: TrustedKeys::parse(BUILT_IN_KEYS).map_err(|e| e.to_string()),
            dirs,
            install: install_dir(),
            chosen: Mutex::new(None),
            confirmed: Mutex::new(false),
        };
        me.settle_stale_attempt();
        me
    }

    /// The per-user folders.
    #[must_use]
    pub const fn dirs(&self) -> &Dirs {
        &self.dirs
    }

    fn state(&self) -> UpdateState {
        UpdateState::load(&self.dirs.updates).unwrap_or_default()
    }

    /// An attempt left with no helper (the PC restarted mid-update, or the helper was
    /// ended) is settled once, so it never blocks later updates:
    /// - while a helper runs, it is the helper's to finish;
    /// - if the version now running is the attempt's target and the installer had run
    ///   (installing or waiting for the start), the update took effect: it is recorded as
    ///   updated and the previous-version copy is cleared;
    /// - otherwise it is recorded as not finished, with the version that is running.
    ///
    /// A just-staged attempt is left alone for a short grace period (its helper may still
    /// be starting).
    fn settle_stale_attempt(&self) {
        self.settle_attempt(helper_running());
    }

    fn settle_attempt(&self, helper_alive: bool) {
        let mut st = self.state();
        let Some(a) = st.attempt.clone() else { return };
        if helper_alive {
            return;
        }
        let age = Duration::from_millis(
            u64::try_from(now_ms().saturating_sub(a.started_at_ms)).unwrap_or(0),
        );
        if a.phase == Phase::Staged && age < STAGED_GRACE {
            return;
        }
        let took_effect =
            a.to == self.version && matches!(a.phase, Phase::Installing | Phase::AwaitingStart);
        if took_effect {
            st.finish(
                OutcomeKind::Updated,
                format!(
                    "Version {} started. The update was finished when HealthierYou next started.",
                    a.to
                ),
                now_ms(),
            );
            if st.save(&self.dirs.updates).is_ok() {
                let _ = std::fs::remove_dir_all(self.dirs.previous());
                let _ = std::fs::remove_dir_all(self.dirs.staged());
                let _ = std::fs::remove_file(self.dirs.updates.join(update::CONFIRM_FILE));
                log(
                    &self.dirs,
                    &format!("settled: updated to {} without the helper", a.to),
                );
            }
        } else {
            // A rollback cut short keeps its previous-version copy for the next attempt.
            let detail = format!(
                "The update to {} did not finish. Version {} is running.",
                a.to, self.version
            );
            st.finish(OutcomeKind::Failed, detail, now_ms());
            if st.save(&self.dirs.updates).is_ok() {
                log(
                    &self.dirs,
                    &format!("settled: the update to {} did not finish", a.to),
                );
            }
        }
    }

    /// The pet page loaded: if the helper is waiting for this version, confirm it.
    pub fn page_ready(&self) {
        let mut done = self
            .confirmed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *done {
            return;
        }
        *done = true;
        let st = self.state();
        if let Some(a) = &st.attempt
            && a.phase == Phase::AwaitingStart
            && a.to == self.version
        {
            let c = Confirmation {
                version: self.version,
                token: a.token.clone(),
            };
            if c.save(&self.dirs.updates).is_ok() {
                log(&self.dirs, &format!("started {}", self.version));
            }
        }
    }

    /// Why update files cannot be installed here, if they cannot.
    fn blocked(&self) -> Option<String> {
        match &self.keys {
            Err(e) => return Some(e.clone()),
            Ok(k) if k.is_empty() => return Some(update::UpdateError::NoKey.to_string()),
            Ok(_) => {}
        }
        if self.install.is_none() {
            return Some("This copy of HealthierYou was not installed with its installer, so it cannot update itself.".into());
        }
        None
    }

    /// Settings > App & updates.
    #[must_use]
    pub fn page(&self) -> AppPage {
        let st = self.state();
        AppPage {
            version: self.version.to_string(),
            install_dir: self.install.as_ref().map(|d| d.display().to_string()),
            data_dir: self.dirs.data.display().to_string(),
            update_blocked: self.blocked(),
            previous_version: st.previous_version.map(|v| v.to_string()),
            in_progress: st.attempt.is_some(),
            last: st.last.map(|o| OutcomeView {
                kind: o.kind,
                from: o.from.to_string(),
                to: o.to.to_string(),
                at: chrono::DateTime::from_timestamp_millis(o.at_ms)
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default(),
                detail: o.detail,
            }),
            autostart: self.install.as_ref().map(|_| autostart_view()),
        }
    }

    /// Check an installer and its `.sig` (next to it) and remember it for "Install".
    ///
    /// # Errors
    /// Why it is refused, worded for the page; nothing changed.
    pub fn check(&self, path: &Path) -> Result<UpdateCheck, String> {
        if let Some(why) = self.blocked() {
            return Err(why);
        }
        let keys = self.keys.as_ref().map_err(Clone::clone)?;
        self.settle_stale_attempt();
        let st = self.state();
        if st.attempt.is_some() {
            return Err("An update is already in progress.".into());
        }
        let sig =
            update::read_signature_file(&update::sig_path_for(path)).map_err(|e| e.to_string())?;
        let release = update::verify_release(keys, path, &sig).map_err(|e| e.to_string())?;
        let plan = update::plan(
            Product::Companion,
            self.version,
            st.previous_version,
            &release,
        )
        .map_err(|e| e.to_string())?;
        let marked = marked_from_internet(path);
        let view = UpdateCheck {
            current: self.version.to_string(),
            version: release.version.to_string(),
            plan,
            #[allow(clippy::cast_precision_loss)]
            size_mb: (release.size as f64 / 104_857.6).round() / 10.0,
            marked_from_internet: marked,
        };
        *self.chosen.lock().unwrap_or_else(PoisonError::into_inner) = Some(Chosen {
            path: path.to_path_buf(),
            release,
            plan,
            marked,
        });
        Ok(view)
    }

    /// Stage the checked installer and start the helper. The caller then closes the app.
    ///
    /// # Errors
    /// Nothing checked, the copy failed its check again, or the helper did not start;
    /// worded for the page. Nothing was installed.
    pub fn start_install(&self) -> Result<(), String> {
        let chosen = self
            .chosen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| "Choose the update file first.".to_owned())?;
        let install = self
            .install
            .clone()
            .ok_or_else(|| self.blocked().unwrap_or_default())?;
        let keys = self.keys.as_ref().map_err(Clone::clone)?;
        let fail =
            |what: &str, e: &dyn std::fmt::Display| format!("Nothing was installed: {what} ({e}).");
        // Stage a copy and check the copy: the file that runs is the file that was checked.
        let staged_dir = self.dirs.staged();
        if staged_dir.exists() {
            std::fs::remove_dir_all(&staged_dir)
                .map_err(|e| fail("the updates folder could not be cleared", &e))?;
        }
        std::fs::create_dir_all(&staged_dir)
            .map_err(|e| fail("the updates folder could not be made", &e))?;
        let staged = staged_dir.join(&chosen.release.file_name);
        std::fs::copy(&chosen.path, &staged)
            .map_err(|e| fail("the installer could not be copied", &e))?;
        std::fs::copy(
            update::sig_path_for(&chosen.path),
            update::sig_path_for(&staged),
        )
        .map_err(|e| fail("the signature could not be copied", &e))?;
        let sig = update::read_signature_file(&update::sig_path_for(&staged))
            .map_err(|e| e.to_string())?;
        let again = update::verify_release(keys, &staged, &sig).map_err(|e| e.to_string())?;
        if again != chosen.release {
            return Err("Nothing was installed: the file changed after it was checked.".into());
        }
        let mut token = [0u8; 16];
        getrandom::fill(&mut token).map_err(|e| fail("no random token", &e))?;
        let mut st = self.state();
        st.attempt = Some(Attempt {
            from: self.version,
            to: chosen.release.version,
            plan: chosen.plan,
            installer: chosen.release.file_name.clone(),
            started_at_ms: now_ms(),
            phase: Phase::Staged,
            token: hy_core::protocol::hex(&token),
            marked_from_internet: chosen.marked || marked_from_internet(&staged),
        });
        let _ = std::fs::remove_file(self.dirs.updates.join(update::CONFIRM_FILE));
        st.save(&self.dirs.updates)
            .map_err(|e| fail("the update record could not be written", &e))?;
        // The helper is a copy of this program, so the installer can replace the original.
        let helper = self.dirs.helper();
        if let Some(d) = helper.parent() {
            std::fs::create_dir_all(d)
                .map_err(|e| fail("the helper folder could not be made", &e))?;
        }
        let me = std::env::current_exe().map_err(|e| fail("this program was not found", &e))?;
        std::fs::copy(me, &helper).map_err(|e| fail("the helper could not be copied", &e))?;
        log(
            &self.dirs,
            &format!(
                "staged {} -> {} ({:?}, marked {})",
                self.version, chosen.release.version, chosen.plan, chosen.marked
            ),
        );
        let started = std::process::Command::new(&helper)
            .arg(ARG_HELPER)
            .arg(std::process::id().to_string())
            .arg(&install)
            .current_dir(&self.dirs.updates)
            .spawn();
        if let Err(e) = started {
            let mut st = self.state();
            st.attempt = None;
            let _ = st.save(&self.dirs.updates);
            return Err(fail("the update helper did not start", &e));
        }
        Ok(())
    }
}

fn autostart_view() -> AutostartView {
    #[cfg(windows)]
    {
        AutostartView {
            enabled: crate::win32::run_value(PRODUCT_NAME).is_some(),
            switched_off_in_windows: crate::win32::startup_switched_off(PRODUCT_NAME),
        }
    }
    #[cfg(not(windows))]
    {
        AutostartView {
            enabled: false,
            switched_off_in_windows: false,
        }
    }
}

/// Switch start at sign-in on or off. Off also leaves a marker in the data folder so a
/// reinstall does not switch it back on.
///
/// # Errors
/// Not an installed copy, or a registry or file error, worded for the page.
pub fn set_autostart(dirs: &Dirs, on: bool) -> Result<(), String> {
    let install =
        install_dir().ok_or_else(|| "Only an installed copy can start at sign-in.".to_owned())?;
    let marker = dirs.data.join(AUTOSTART_OFF_MARKER);
    #[cfg(windows)]
    {
        if on {
            let exe = install.join("hy-companion.exe");
            crate::win32::set_run_value(PRODUCT_NAME, &format!("\"{}\"", exe.display()))
                .map_err(|e| format!("Start at sign-in could not be switched on: {e}"))?;
            let _ = std::fs::remove_file(&marker);
        } else {
            crate::win32::delete_run_value(PRODUCT_NAME)
                .map_err(|e| format!("Start at sign-in could not be switched off: {e}"))?;
            let _ = std::fs::create_dir_all(&dirs.data);
            std::fs::write(
                &marker,
                b"Start at sign-in was switched off in HealthierYou.\n",
            )
            .map_err(|e| format!("The choice could not be saved: {e}"))?;
        }
    }
    #[cfg(not(windows))]
    let _ = (install, marker, on);
    Ok(())
}

// --------------------------------------------------------------- command line and helper

/// If this run is a command-line update step, do it and return the process exit code.
///
/// - `--hy-update-check <installer> [--report <file>]`: check only (0 accepted, 2 refused).
/// - `--hy-update-install <installer> [--report <file>]`: check, stage and start the
///   helper (0 started, 2 refused, 3 the app is running).
/// - `--hy-update-helper <pid> <install folder>`: the helper itself.
#[must_use]
pub fn command_line(version: Version) -> Option<i32> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let first = args.first()?.to_str()?;
    if ![ARG_CHECK, ARG_INSTALL, ARG_HELPER].contains(&first) {
        return None;
    }
    let Some(dirs) = Dirs::from_env() else {
        return Some(4);
    };
    let report = args
        .iter()
        .position(|a| a == ARG_REPORT)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    Some(match first {
        ARG_HELPER => {
            let pid = args
                .get(1)
                .and_then(|a| a.to_str()?.parse().ok())
                .unwrap_or(0);
            match args.get(2) {
                Some(install) => run_helper(&dirs, pid, Path::new(install)),
                None => 4,
            }
        }
        _ => {
            let Some(file) = args.get(1).map(PathBuf::from) else {
                return Some(4);
            };
            cli_check_install(
                version,
                &dirs,
                &file,
                first == ARG_INSTALL,
                report.as_deref(),
            )
        }
    })
}

#[derive(Debug, Serialize)]
struct CliReport {
    ok: bool,
    check: Option<UpdateCheck>,
    error: Option<String>,
}

fn cli_check_install(
    version: Version,
    dirs: &Dirs,
    file: &Path,
    install: bool,
    report: Option<&Path>,
) -> i32 {
    let updater = Updater::new(version, dirs.clone());
    let mut code = 0;
    let result = updater.check(file).and_then(|check| {
        if install {
            #[cfg(windows)]
            if crate::win32::companion_running() {
                code = 3;
                return Err("Close HealthierYou first (tray > Quit).".into());
            }
            updater.start_install()?;
        }
        Ok(check)
    });
    let out = match result {
        Ok(check) => CliReport {
            ok: true,
            check: Some(check),
            error: None,
        },
        Err(e) => {
            if code == 0 {
                code = 2;
            }
            log(dirs, &format!("refused: {e}"));
            CliReport {
                ok: false,
                check: None,
                error: Some(e),
            }
        }
    };
    if let Some(path) = report {
        let _ = std::fs::write(path, serde_json::to_vec_pretty(&out).unwrap_or_default());
    }
    code
}

/// Verify the staged installer with the built-in keys, for the attempt's version.
fn reverify_staged(installer: &Path, to: Version) -> Result<(), String> {
    let keys = TrustedKeys::parse(BUILT_IN_KEYS).map_err(|e| e.to_string())?;
    let sig =
        update::read_signature_file(&update::sig_path_for(installer)).map_err(|e| e.to_string())?;
    let release = update::verify_release(&keys, installer, &sig).map_err(|e| e.to_string())?;
    if release.product != Product::Companion || release.version != to {
        return Err(format!("it is now version {}", release.version));
    }
    Ok(())
}

/// What the helper's installer run did.
enum InstallRun {
    /// The installer finished with exit code 0.
    Done,
    /// The installer ran and failed (files may have changed).
    Failed(String),
    /// The shell did not start it: nothing changed.
    NotStarted(String),
}

fn set_phase(dirs: &Dirs, st: &mut UpdateState, phase: Phase) {
    if let Some(a) = &mut st.attempt {
        a.phase = phase;
    }
    let _ = st.save(&dirs.updates);
}

/// The update helper (a copy of this program in the updates folder).
fn run_helper(dirs: &Dirs, parent: u32, install: &Path) -> i32 {
    let mut st = match UpdateState::load(&dirs.updates) {
        Ok(s) => s,
        Err(_) => return 5,
    };
    let Some(attempt) = st.attempt.clone() else {
        return 5;
    };
    #[cfg(windows)]
    crate::win32::mark_update_helper();
    log(dirs, &format!("helper: {} -> {}", attempt.from, attempt.to));
    if !wait_for_exit(parent, CLOSE_WAIT) {
        st.finish(
            OutcomeKind::Failed,
            "HealthierYou did not close, so nothing was installed.",
            now_ms(),
        );
        let _ = st.save(&dirs.updates);
        log(dirs, "helper: the app did not close");
        return 6;
    }
    // The app's WebView2 processes end a moment after it does; the installer can only
    // replace the runtime once they have.
    wait_unlocked(install, CLOSE_WAIT);
    // Keep the previous version: program files and the uninstall entry.
    if let Err(e) = update::backup_program_files(install, &dirs.previous(), PROGRAM_ENTRIES) {
        st.finish(
            OutcomeKind::Failed,
            format!("The previous version could not be kept ({e}), so nothing was installed."),
            now_ms(),
        );
        let _ = st.save(&dirs.updates);
        start_program(install);
        return 7;
    }
    let entry_saved = export_uninstall_entry(dirs);
    let installer = dirs.staged().join(&attempt.installer);
    // Check the staged file again just before it runs: time has passed since the app
    // checked it (closing, the backup), and the updates folder is the user's own.
    if let Err(why) = reverify_staged(&installer, attempt.to) {
        st.finish(
            OutcomeKind::Failed,
            format!(
                "The staged installer no longer passed its check ({why}), so nothing was installed."
            ),
            now_ms(),
        );
        let _ = st.save(&dirs.updates);
        let _ = std::fs::remove_dir_all(dirs.previous());
        let _ = std::fs::remove_dir_all(dirs.staged());
        log(dirs, "helper: staged installer refused before running");
        start_program(install);
        return 9;
    }
    set_phase(dirs, &mut st, Phase::Installing);
    let outcome = match run_installer(&installer, &dirs.staged()) {
        InstallRun::NotStarted(why) => {
            st.finish(OutcomeKind::Blocked, why, now_ms());
            let _ = st.save(&dirs.updates);
            log(dirs, "helper: installer not started");
            start_program(install);
            return 0;
        }
        InstallRun::Failed(why) => Err(why),
        InstallRun::Done => {
            set_phase(dirs, &mut st, Phase::AwaitingStart);
            wait_for_start(dirs, install, &attempt)
        }
    };
    match outcome {
        Ok(()) => {
            st.finish(
                OutcomeKind::Updated,
                format!("Version {} started.", attempt.to),
                now_ms(),
            );
            let _ = st.save(&dirs.updates);
            let _ = std::fs::remove_dir_all(dirs.previous());
            let _ = std::fs::remove_dir_all(dirs.staged());
            log(dirs, &format!("helper: updated to {}", attempt.to));
            0
        }
        Err(why) => {
            set_phase(dirs, &mut st, Phase::RollingBack);
            log(dirs, &format!("helper: rolling back ({why})"));
            wait_unlocked(install, CLOSE_WAIT);
            let mut restored =
                update::restore_program_files(&dirs.previous(), install, PROGRAM_ENTRIES);
            for _ in 0..10 {
                if restored.is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
                restored =
                    update::restore_program_files(&dirs.previous(), install, PROGRAM_ENTRIES);
            }
            if entry_saved {
                import_uninstall_entry(dirs);
            }
            let (kind, detail) = match restored {
                Ok(_) => (
                    OutcomeKind::RolledBack,
                    format!("{why} Version {} was put back.", attempt.from),
                ),
                Err(e) => (
                    OutcomeKind::Failed,
                    format!(
                        "{why} The previous version could not be put back ({e}); install it again from its file."
                    ),
                ),
            };
            st.finish(kind, detail, now_ms());
            let _ = st.save(&dirs.updates);
            let _ = std::fs::remove_file(dirs.updates.join(update::CONFIRM_FILE));
            start_program(install);
            8
        }
    }
}

/// Wait for the new version to confirm it started. It runs as the user's app afterwards.
fn wait_for_start(dirs: &Dirs, install: &Path, attempt: &Attempt) -> Result<(), String> {
    let exe = install.join("hy-companion.exe");
    let mut child = std::process::Command::new(&exe)
        .current_dir(install)
        .spawn()
        .map_err(|e| format!("Version {} could not be started ({e}).", attempt.to))?;
    let start = Instant::now();
    loop {
        if Confirmation::load(&dirs.updates).is_some_and(|c| c.confirms(attempt)) {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "Version {} closed before it finished starting ({status}).",
                attempt.to
            ));
        }
        if start.elapsed() > START_WAIT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "Version {} did not finish starting within {} seconds.",
                attempt.to,
                START_WAIT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Wait until no program in the install folder is running: a running image cannot be
/// opened for writing (the open changes nothing and is closed at once).
fn wait_unlocked(install: &Path, timeout: Duration) {
    let images = [
        install.join("hy-companion.exe"),
        install.join("webview2-fixed").join("msedgewebview2.exe"),
    ];
    let start = Instant::now();
    while start.elapsed() < timeout {
        let busy = images
            .iter()
            .any(|p| p.exists() && std::fs::OpenOptions::new().write(true).open(p).is_err());
        if !busy {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn start_program(install: &Path) {
    let _ = std::process::Command::new(install.join("hy-companion.exe"))
        .current_dir(install)
        .spawn();
}

#[cfg(windows)]
fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    crate::win32::wait_for_exit(pid, timeout)
}

#[cfg(not(windows))]
fn wait_for_exit(_pid: u32, _timeout: Duration) -> bool {
    true
}

#[cfg(windows)]
fn run_installer(installer: &Path, dir: &Path) -> InstallRun {
    use crate::win32::{ShellRun, shell_run_and_wait};
    // `/S` silent; `/UPDATE` keeps start at sign-in as the user left it.
    match shell_run_and_wait(installer, "/S /UPDATE", dir) {
        ShellRun::Exited(0) => InstallRun::Done,
        ShellRun::Exited(code) => {
            InstallRun::Failed(format!("The installer stopped with code {code}."))
        }
        ShellRun::NotStarted(1223) => InstallRun::NotStarted(
            "The installer was cancelled at a Windows prompt, so nothing was changed.".into(),
        ),
        ShellRun::NotStarted(code) => InstallRun::NotStarted(format!(
            "Windows did not start the installer (error {code}): SmartScreen, Smart App Control, antivirus or a policy may have blocked it. Nothing was changed; tell the owner, who decides about signing."
        )),
    }
}

#[cfg(not(windows))]
fn run_installer(_installer: &Path, _dir: &Path) -> InstallRun {
    InstallRun::NotStarted("Updates install on Windows only.".into())
}

/// The uninstall entry's registry key (per user).
const UNINSTALL_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\HealthierYou";

fn reg_exe() -> PathBuf {
    std::env::var_os("SystemRoot").map_or_else(
        || PathBuf::from("reg.exe"),
        |r| Path::new(&r).join("System32").join("reg.exe"),
    )
}

/// Save the uninstall entry, so a rollback shows the previous version in Windows' list.
fn export_uninstall_entry(dirs: &Dirs) -> bool {
    std::process::Command::new(reg_exe())
        .args(["export", UNINSTALL_KEY])
        .arg(dirs.uninstall_entry())
        .arg("/y")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn import_uninstall_entry(dirs: &Dirs) {
    let _ = std::process::Command::new(reg_exe())
        .arg("import")
        .arg(dirs.uninstall_entry())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// The store could not open because a newer version changed the database in a way this
/// version cannot read (a rollback). Offer the copy taken before that upgrade
/// (architecture 8): events since then are lost, and the newer database is set aside, not
/// deleted. True if the copy was put back and the store should be opened again.
pub fn offer_data_rollback(db: &Path, version: Version) -> bool {
    let Some((schema, copy)) =
        update::usable_pre_migration_copy(db, hy_core::migrations::MIGRATIONS)
    else {
        tell_user(
            "HealthierYou",
            &format!(
                "HealthierYou {version} cannot read the data a newer version wrote, and there is no earlier copy it can use.\n\nInstall the newer version again to keep using your data. HealthierYou will close; nothing was changed."
            ),
        );
        return false;
    };
    let taken = std::fs::metadata(&copy)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .format(" on %Y-%m-%d at %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let question = format!(
        "HealthierYou {version} cannot read the data that a newer version wrote.\n\nIt can go back to the copy of your data taken before that update{taken} (schema v{schema}). Anything recorded since then (reminders, water, notes, settings) will be lost. The newer data stays in the same folder; it is not deleted.\n\nGo back to that copy now? Choose No to close without changing anything."
    );
    if !ask_user("HealthierYou: older version", &question) {
        return false;
    }
    match update::restore_pre_migration_copy(db, &copy, now_ms()) {
        Ok(_) => true,
        Err(e) => {
            let text = match &e.state {
                update::RestoreState::Unchanged => format!(
                    "The earlier copy could not be put back: {}. Your current data was not changed. HealthierYou will close.",
                    e.reason
                ),
                update::RestoreState::SetAside(aside) => format!(
                    "The earlier copy could not be put back: {}.\n\nYour current data is safe in the data folder as \"{}\". HealthierYou will not start with empty data: next time it starts it offers to put that file back.",
                    e.reason,
                    aside
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default()
                ),
            };
            tell_user("HealthierYou", &text);
            false
        }
    }
}

/// Before the store opens: if a restore last time set the database aside and could not
/// move it back, offer to put it back. Returns false if the app must close (it then
/// never opens, and so never creates, an empty database).
pub fn settle_incomplete_restore(db: &Path) -> bool {
    let Some(aside) = update::incomplete_restore(db) else {
        return true;
    };
    let name = aside
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if db.exists() {
        // A database was put in place by hand: the marker is out of date.
        let _ = std::fs::remove_file(PathBuf::from(format!(
            "{}.restore-incomplete",
            db.display()
        )));
        return true;
    }
    let question = format!(
        "Last time HealthierYou could not finish going back to an earlier copy of your data. Your data is safe in the data folder as \"{name}\".\n\nPut it back now? Choose No to close without changing anything."
    );
    if !ask_user("HealthierYou: your data", &question) {
        return false;
    }
    match update::recover_incomplete_restore(db) {
        Ok(()) => true,
        Err(e) => {
            tell_user(
                "HealthierYou",
                &format!(
                    "Your data could not be put back ({e}). It is still safe as \"{name}\". HealthierYou will close."
                ),
            );
            false
        }
    }
}

#[cfg(windows)]
fn ask_user(title: &str, text: &str) -> bool {
    crate::win32::ask_yes_no(title, text)
}

#[cfg(not(windows))]
fn ask_user(_title: &str, _text: &str) -> bool {
    false
}

#[cfg(windows)]
fn tell_user(title: &str, text: &str) {
    crate::win32::tell(title, text);
}

#[cfg(not(windows))]
fn tell_user(title: &str, text: &str) {
    eprintln!("{title}: {text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zone_is_read_from_the_mark_of_the_web() {
        let downloaded = "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://example.invalid/\r\n";
        assert_eq!(zone_from_identifier(downloaded), Some(3));
        assert_eq!(zone_from_identifier("[ZoneTransfer]\nZoneId=1\n"), Some(1));
        assert_eq!(zone_from_identifier("[ZoneTransfer]\n"), None);
        assert_eq!(zone_from_identifier(""), None);
    }

    #[test]
    fn folders_sit_under_local_app_data() {
        let d = Dirs::under(Path::new("base"));
        assert!(d.data.ends_with("HealthierYou/data") || d.data.ends_with("HealthierYou\\data"));
        assert_eq!(d.updates.parent(), d.data.parent());
        // The updates folder never holds personal data, and the program copy never
        // includes the data folder.
        assert!(!d.updates.starts_with(&d.data));
        assert!(!PROGRAM_ENTRIES.contains(&"data"));
        assert!(!PROGRAM_ENTRIES.contains(&"updates"));
    }

    #[test]
    fn a_build_without_keys_or_installer_says_why_it_cannot_update() {
        let dir = std::env::temp_dir().join(format!("hy-update-test-{}", std::process::id()));
        let u = Updater::new(Version::new(0, 1, 0), Dirs::under(&dir));
        let page = u.page();
        // Tests run from the build folder, which has no uninstaller next to it.
        assert!(page.install_dir.is_none());
        assert!(page.update_blocked.is_some());
        assert!(
            u.check(Path::new("HealthierYou_0.2.0_x64-setup.exe"))
                .is_err()
        );
        assert!(u.start_install().is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp_dirs(name: &str) -> (PathBuf, Dirs) {
        let root = std::env::temp_dir().join(format!("hy-settle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = Dirs::under(&root);
        std::fs::create_dir_all(&dirs.updates).unwrap();
        (root, dirs)
    }

    fn seed(dirs: &Dirs, phase: Phase, from: Version, to: Version, age: Duration) {
        let started = now_ms() - i64::try_from(age.as_millis()).unwrap();
        let st = UpdateState {
            previous_version: None,
            attempt: Some(Attempt {
                from,
                to,
                plan: Plan::Upgrade,
                installer: format!("HealthierYou_{to}_x64-setup.exe"),
                started_at_ms: started,
                phase,
                token: "t0k3n".into(),
                marked_from_internet: false,
            }),
            last: None,
        };
        st.save(&dirs.updates).unwrap();
    }

    const V0: Version = Version::new(0, 1, 0);
    const V1: Version = Version::new(0, 1, 1);

    #[test]
    fn a_restart_after_the_install_but_before_the_helper_finished_is_settled_as_updated() {
        // The installer ran, the new version started and confirmed, then the PC restarted
        // (or the helper was ended) before the helper recorded the outcome.
        let (root, dirs) = temp_dirs("restart");
        seed(&dirs, Phase::AwaitingStart, V0, V1, Duration::from_secs(90));
        Confirmation {
            version: V1,
            token: "t0k3n".into(),
        }
        .save(&dirs.updates)
        .unwrap();
        std::fs::create_dir_all(dirs.previous().join("program")).unwrap();
        let u = Updater::new(V1, dirs.clone());
        u.settle_attempt(false);
        let st = UpdateState::load(&dirs.updates).unwrap();
        assert!(st.attempt.is_none(), "the attempt no longer blocks updates");
        let last = st.last.unwrap();
        assert_eq!(last.kind, OutcomeKind::Updated);
        assert_eq!((last.from, last.to), (V0, V1));
        assert_eq!(
            st.previous_version,
            Some(V0),
            "going back to the old version stays possible"
        );
        assert!(!dirs.previous().exists());
        assert!(!dirs.updates.join(update::CONFIRM_FILE).exists());
        // The next check is not refused as "already in progress" (it is refused here only
        // because tests do not run from an installed copy).
        let err = u
            .check(Path::new("HealthierYou_0.1.2_x64-setup.exe"))
            .unwrap_err();
        assert!(!err.contains("already in progress"), "{err}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_install_cut_short_after_the_installer_ran_is_settled_by_the_running_version() {
        let (root, dirs) = temp_dirs("installing");
        seed(&dirs, Phase::Installing, V0, V1, Duration::from_secs(10));
        Updater::new(V1, dirs.clone()).settle_attempt(false);
        let st = UpdateState::load(&dirs.updates).unwrap();
        assert!(st.attempt.is_none());
        assert_eq!(st.last.unwrap().kind, OutcomeKind::Updated);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_attempt_whose_target_is_not_running_is_settled_as_not_finished() {
        let (root, dirs) = temp_dirs("old");
        seed(&dirs, Phase::AwaitingStart, V0, V1, Duration::from_secs(10));
        std::fs::create_dir_all(dirs.previous().join("program")).unwrap();
        Updater::new(V0, dirs.clone()).settle_attempt(false);
        let st = UpdateState::load(&dirs.updates).unwrap();
        assert!(st.attempt.is_none());
        let last = st.last.unwrap();
        assert_eq!(last.kind, OutcomeKind::Failed);
        assert!(last.detail.contains("did not finish"));
        assert_eq!(st.previous_version, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_live_helper_or_a_just_staged_attempt_is_left_alone() {
        let (root, dirs) = temp_dirs("live");
        let u = Updater::new(V1, dirs.clone());
        seed(&dirs, Phase::AwaitingStart, V0, V1, Duration::from_secs(10));
        u.settle_attempt(true);
        assert!(UpdateState::load(&dirs.updates).unwrap().attempt.is_some());
        seed(&dirs, Phase::Staged, V0, V1, Duration::from_secs(5));
        u.settle_attempt(false);
        assert!(UpdateState::load(&dirs.updates).unwrap().attempt.is_some());
        // An old staged attempt whose helper never ran is settled.
        seed(
            &dirs,
            Phase::Staged,
            V0,
            V1,
            STAGED_GRACE + Duration::from_secs(1),
        );
        u.settle_attempt(false);
        let st = UpdateState::load(&dirs.updates).unwrap();
        assert!(st.attempt.is_none());
        assert_eq!(st.last.unwrap().kind, OutcomeKind::Failed);
        let _ = std::fs::remove_dir_all(root);
    }
}
