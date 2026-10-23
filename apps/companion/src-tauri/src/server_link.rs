//! Pairing with the office server, on the companion (V1-5 contract 4.2, 4.5, 6.3 and 12;
//! wireframe W8).
//!
//! - **Pair:** the user pastes or types the slip. It is checked locally (a mistyped slip
//!   never reaches the network), then redeemed over pinned TLS 1.3 with
//!   [`hy_core::link::flow::redeem_slip`]. The token is sealed at once with user-scope
//!   DPAPI (entropy `HealthierYou/server_link/v1`), and the link is stored pending in one
//!   transaction that also switches every category off.
//! - **Confirmation:** the settings page shows the six words of the pin and this PC's
//!   check code for the user to read out to the administrator. "Check again" asks the
//!   server; the PC counts as confirmed only after a signed head and a verified chain
//!   extension containing its own confirmation ([`hy_core::store::Store::apply_confirmed_state`]).
//! - **Server address:** if the server's IP changed and its name cannot be found, the
//!   user can enter its address. The remembered and manual addresses are tried before
//!   any name lookup; every address must present the pinned certificate.
//! - **Unpair:** the server is told when it can be reached, and the local unpair
//!   transaction always runs. A revoked PC unpairs locally on its next check.
//!
//! Nothing here holds the store across a network wait. The token is opened only for the
//! duration of one request and never logged or shown.

use std::sync::Arc;

use hy_core::link::LinkOptions;
use hy_core::link::flow::{
    DeviceFacts, FlowError, StoreAccess, SyncOutcome, redeem_slip, restore_from_server, sync_once,
    unpair_remote,
};
use hy_core::protocol::slip::Slip;
use hy_core::protocol::wire::ControlOp;
use hy_core::protocol::words::{device_check_code, fingerprint_words};
use hy_core::schema::Category;
use hy_core::secret::{Secret, TokenVault};
use hy_core::store::{NoticeKind, ServerLink, ServerNotice, Store};
use serde::Serialize;
use uuid::Uuid;

use crate::shell::Shell;

/// The notice the pairing screen shows (recorded in the consent log).
pub const PAIRING_NOTICE: &str = "v1-5-pairing-1";

/// The notice shown before a category is switched on (recorded in the consent log).
pub const SHARING_NOTICE: &str = "v1-5-sharing-1";

/// How often the companion syncs on its own while it runs.
pub const SYNC_EVERY: std::time::Duration = std::time::Duration::from_secs(15 * 60);

impl StoreAccess for Shell {
    fn with<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        self.with_store(f)
    }
}

/// The token vault: user-scope DPAPI on Windows. Elsewhere pairing is refused (the
/// companion is a Windows product; tests use `hy_core::secret::EphemeralVault`).
#[derive(Debug, Default, Clone, Copy)]
pub struct DpapiUserVault;

impl TokenVault for DpapiUserVault {
    fn seal(&self, token: &Secret<[u8; 32]>) -> hy_core::Result<Vec<u8>> {
        #[cfg(windows)]
        {
            crate::win32::protect_user(token.expose(), hy_core::protocol::DPAPI_ENTROPY)
                .map_err(|_| hy_core::Error::Invalid("the token could not be protected".into()))
        }
        #[cfg(not(windows))]
        {
            let _ = token;
            Err(hy_core::Error::Invalid("pairing needs Windows".into()))
        }
    }

    fn open(&self, blob: &[u8]) -> hy_core::Result<Secret<[u8; 32]>> {
        #[cfg(windows)]
        {
            let plain = crate::win32::unprotect_user(blob, hy_core::protocol::DPAPI_ENTROPY)
                .map_err(|_| hy_core::Error::Invalid("token blob cannot be opened here".into()))?;
            let token: [u8; 32] =
                plain.expose().as_slice().try_into().map_err(|_| {
                    hy_core::Error::Invalid("token blob cannot be opened here".into())
                })?;
            Ok(Secret::new(token))
        }
        #[cfg(not(windows))]
        {
            let _ = blob;
            Err(hy_core::Error::Invalid("pairing needs Windows".into()))
        }
    }
}

/// One of the user's PCs, for "PCs paired to you".
#[derive(Debug, Clone, Serialize)]
pub struct PairedPc {
    /// The device reference (for "seen").
    pub device_ref: String,
    /// Its label.
    pub label: String,
    /// Confirmed by the administrator.
    pub confirmed: bool,
    /// This PC.
    pub this_pc: bool,
    /// Newly paired since the user last looked: show the new-device notice.
    pub new: bool,
}

/// One optional category on the sharing page.
#[derive(Debug, Clone, Serialize)]
pub struct ShareState {
    /// The stored name.
    pub category: &'static str,
    /// Its name in words.
    pub label: &'static str,
    /// On (only after the office server accepted it).
    pub enabled: bool,
    /// Shared from this date (`YYYY-MM-DD`) while on.
    pub share_from: Option<String>,
    /// Switched on here, waiting for the office server.
    pub waiting: bool,
    /// This version builds records for it. When false it is shown as "not yet sharing"
    /// and cannot be switched on.
    pub available: bool,
}

/// A one-line notice about sharing (never blocks anything).
#[derive(Debug, Clone, Serialize)]
pub struct NoticeView {
    /// For dismissing.
    pub id: u64,
    /// The words.
    pub text: String,
}

/// The pairing section of Settings > Sharing & server.
#[derive(Debug, Clone, Serialize)]
pub struct LinkView {
    /// `none`, `pending` or `confirmed`.
    pub state: &'static str,
    /// The user's alias on the server.
    pub alias: Option<String>,
    /// The server's name or address from the slip.
    pub host: Option<String>,
    /// The six words of the pin, to read out.
    pub words: Vec<&'static str>,
    /// This PC's check code, to read out.
    pub check_code: Option<String>,
    /// The confirmation date.
    pub confirmed_on: Option<String>,
    /// The address the user entered for when the server's IP changed, if any.
    pub manual_address: Option<String>,
    /// The user's PCs (after confirmation).
    pub devices: Vec<PairedPc>,
    /// The optional categories (switchable once confirmed).
    pub sharing: Vec<ShareState>,
    /// Sharing notices not yet dismissed.
    pub notices: Vec<NoticeView>,
    /// The server is recovering: sharing paused.
    pub hold: bool,
    /// The server's history did not verify: uploads paused.
    pub mismatch: bool,
    /// Last successful sync (`YYYY-MM-DD HH:MM`, local time).
    pub last_sync: Option<String>,
    /// A message for the user about the last action, if any.
    pub message: Option<String>,
}

impl LinkView {
    fn of(link: Option<&ServerLink>, devices: Vec<PairedPc>, message: Option<String>) -> Self {
        match link {
            None => Self {
                state: "none",
                alias: None,
                host: None,
                words: Vec::new(),
                check_code: None,
                confirmed_on: None,
                manual_address: None,
                devices: Vec::new(),
                sharing: Vec::new(),
                notices: Vec::new(),
                hold: false,
                mismatch: false,
                last_sync: None,
                message,
            },
            Some(l) => Self {
                state: if l.confirmed { "confirmed" } else { "pending" },
                alias: Some(l.alias.clone()),
                host: Some(l.host.clone()),
                words: fingerprint_words(&l.pin).to_vec(),
                check_code: Some(device_check_code(l.device_ref.as_bytes())),
                confirmed_on: l.confirmed_on.map(|d| d.to_string()),
                manual_address: l.manual_address.clone(),
                devices,
                sharing: Vec::new(),
                notices: Vec::new(),
                hold: false,
                mismatch: false,
                last_sync: None,
                message,
            },
        }
    }
}

/// The words of a notice.
fn notice_text(n: &ServerNotice) -> String {
    let category = n
        .category
        .map_or("A category", crate::shell::category_label);
    let who = n
        .from_label
        .clone()
        .unwrap_or_else(|| "another of your PCs".to_owned());
    match n.kind {
        NoticeKind::RemoteEnable => format!(
            "{category} sharing was switched on from {who}{}. You can switch it off here at any time.",
            n.date.map(|d| format!(", from {d}")).unwrap_or_default()
        ),
        NoticeKind::RemoteDisable => format!("{category} sharing was switched off from {who}."),
        NoticeKind::StaleEnable => format!(
            "{category} was not switched on: it had been switched off on another of your PCs since. Switch it on again if you want it."
        ),
        NoticeKind::Regressed => "The office server saw an older copy of this PC's data (a restored or copied PC?). Every category is off; switch on again the ones you want.".to_owned(),
    }
}

fn store_error(_: hy_core::Error) -> String {
    "HealthierYou could not read or save its data on this PC.".to_owned()
}

/// The current view.
///
/// # Errors
/// A store failure, worded for the user.
pub fn view(shell: &Shell, message: Option<String>) -> Result<LinkView, String> {
    shell.with_store(|store| {
        let link = store.server_link().map_err(store_error)?;
        let devices = match &link {
            Some(l) if l.confirmed => store
                .paired_devices()
                .map_err(store_error)?
                .into_iter()
                .map(|d| PairedPc {
                    device_ref: d.device_ref.to_string(),
                    this_pc: d.device_ref == l.device_ref,
                    new: !d.notice_shown,
                    label: d.label,
                    confirmed: d.confirmed,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut v = LinkView::of(link.as_ref(), devices, message);
        if link.as_ref().is_some_and(|l| l.confirmed) {
            let waiting: Vec<Category> = store
                .pending_control_ops()
                .map_err(store_error)?
                .into_iter()
                .filter_map(|op| match op {
                    ControlOp::Enable { category, .. } => Some(category),
                    _ => None,
                })
                .collect();
            for c in Category::ALL.iter().copied() {
                if c == Category::PostureSummary {
                    continue;
                }
                let p = store.sharing_policy(c).map_err(store_error)?;
                v.sharing.push(ShareState {
                    category: c.as_str(),
                    label: crate::shell::category_label(c),
                    enabled: p.enabled,
                    share_from: p.share_from.filter(|_| p.enabled).map(|d| d.to_string()),
                    waiting: waiting.contains(&c),
                    available: hy_core::protocol::share::produced(c),
                });
            }
            let status = store.server_status().map_err(store_error)?;
            v.hold = status.hold;
            v.mismatch = status.mismatch;
            let zone = store.user_zone().map_err(store_error)?;
            v.last_sync = status
                .last_sync_ms
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|t| zone.local_datetime(t).format("%Y-%m-%d %H:%M").to_string());
            v.notices = store
                .server_notices()
                .map_err(store_error)?
                .iter()
                .map(|n| NoticeView {
                    id: n.id,
                    text: notice_text(n),
                })
                .collect();
        }
        Ok(v)
    })
}

fn computer_label() -> String {
    let name = std::env::var("COMPUTERNAME").unwrap_or_default();
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(hy_core::protocol::wire::MAX_LABEL)
        .collect();
    if name.is_empty() {
        "This PC".to_owned()
    } else {
        name
    }
}

/// "Pair": check the slip, redeem it over pinned TLS, store the pending link.
///
/// # Errors
/// The reason, worded for the user; nothing was stored.
pub async fn pair(shell: Arc<Shell>, slip_text: String) -> Result<LinkView, String> {
    let slip = Slip::parse(&slip_text).map_err(|e| FlowError::Slip(e).to_string())?;
    let origin: Uuid = shell.with_store(|s| s.device_id());
    if shell
        .with_store(|s| s.server_link())
        .map_err(store_error)?
        .is_some()
    {
        return Err("This PC is already paired. Unpair it first.".to_owned());
    }
    let facts = DeviceFacts {
        origin_device: origin,
        label: computer_label(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        // The build number is shown to the administrator only; V1-6 reads it from the OS.
        windows_build: "windows".to_owned(),
    };
    let redeemed = redeem_slip(&slip, &facts, &DpapiUserVault, LinkOptions::default())
        .await
        .map_err(|e| e.to_string())?;
    shell
        .with_store(|s| s.save_pairing(&redeemed.link, PAIRING_NOTICE))
        .map_err(store_error)?;
    view(&shell, None)
}

/// "Check again" and the periodic sync: one whole sync with the server
/// ([`hy_core::link::flow::sync_once`]).
///
/// # Errors
/// A store failure, worded for the user. Network and server problems come back as the
/// view's message, with the link unchanged.
pub async fn refresh(shell: Arc<Shell>) -> Result<LinkView, String> {
    let today = shell.with_store(|s| s.today()).map_err(store_error)?;
    let message = match sync_once(&*shell, &DpapiUserVault, LinkOptions::default(), today).await {
        Ok(SyncOutcome::NotPaired | SyncOutcome::Synced { .. }) => None,
        Ok(SyncOutcome::Pending) => {
            Some("Waiting for your administrator to confirm this PC.".to_owned())
        }
        Ok(SyncOutcome::HistoryMismatch) => {
            Some("Server history mismatch: sharing is paused. Tell your administrator.".to_owned())
        }
        Ok(SyncOutcome::Hold { .. }) => {
            Some(FlowError::Server(hy_core::protocol::wire::WireError::RecoveryHold).to_string())
        }
        Err(e) => Some(e.to_string()),
    };
    view(&shell, message)
}

/// Sync every [`SYNC_EVERY`] while the app runs (on its own thread; each sync is a
/// short async task, and failures only wait for the next round).
pub fn start_periodic_sync(shell: &Arc<Shell>) {
    let weak = Arc::downgrade(shell);
    let spawned = std::thread::Builder::new()
        .name("hy-sync".into())
        .spawn(move || {
            loop {
                std::thread::sleep(SYNC_EVERY);
                let Some(shell) = weak.upgrade() else { return };
                let paired = shell
                    .with_store(|s| s.server_link())
                    .ok()
                    .flatten()
                    .is_some();
                if paired {
                    let _ = tauri::async_runtime::block_on(refresh(shell));
                }
            }
        });
    if spawned.is_err() {
        eprintln!("HealthierYou: the periodic sync could not start");
    }
}

/// Switch a category on (after the notice) or off. Off takes effect at once, even
/// offline; on waits for the office server. A sync follows.
///
/// # Errors
/// An unknown category or a store failure, worded for the user.
pub async fn set_sharing(
    shell: Arc<Shell>,
    category: String,
    enabled: bool,
) -> Result<LinkView, String> {
    let c = Category::ALL
        .iter()
        .copied()
        .find(|c| c.as_str() == category)
        .ok_or_else(|| "Unknown category.".to_owned())?;
    if enabled && !hy_core::protocol::share::produced(c) {
        return Err("This category is not yet shared by this version of HealthierYou.".to_owned());
    }
    let confirmed = shell
        .with_store(|s| s.server_link())
        .map_err(store_error)?
        .is_some_and(|l| l.confirmed);
    if !confirmed {
        return Err(
            "Sharing can be switched on only after your administrator confirms this PC.".to_owned(),
        );
    }
    shell
        .with_store(|s| {
            if enabled {
                s.request_enable(c, SHARING_NOTICE).map(|_| ())
            } else {
                s.disable_sharing(c).map(|_| ())
            }
        })
        .map_err(store_error)?;
    refresh(shell).await
}

/// Dismiss a sharing notice.
///
/// # Errors
/// A store failure, worded for the user.
pub fn dismiss_notice(shell: &Shell, id: u64) -> Result<LinkView, String> {
    shell
        .with_store(|s| s.dismiss_server_notice(id))
        .map_err(store_error)?;
    view(shell, None)
}

/// "Preview what is sent": today's record for a category, exactly as it would be sent
/// (the same serializer as the upload), or `None` when there is nothing to send.
///
/// # Errors
/// An unknown category or a store failure, worded for the user.
pub fn preview(shell: &Shell, category: &str) -> Result<Option<String>, String> {
    let c = Category::ALL
        .iter()
        .copied()
        .find(|c| c.as_str() == category)
        .ok_or_else(|| "Unknown category.".to_owned())?;
    shell
        .with_store(|s| -> hy_core::Result<_> {
            let today = s.today()?;
            Ok(s.share_record(c, today)?
                .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default()))
        })
        .map_err(store_error)
}

/// "Restore from server": the last 90 days of the user's shared summaries, kept
/// read-only and apart from this PC's own data.
///
/// # Errors
/// A store failure, worded for the user.
pub async fn restore(shell: Arc<Shell>) -> Result<LinkView, String> {
    let today = shell.with_store(|s| s.today()).map_err(store_error)?;
    let from = today
        .checked_sub_days(chrono::Days::new(89))
        .unwrap_or(today);
    let message = match restore_from_server(
        &*shell,
        &DpapiUserVault,
        LinkOptions::default(),
        from,
        today,
    )
    .await
    {
        Ok(0) => "Nothing new to restore from the office server.".to_owned(),
        Ok(n) => format!("Restored {n} daily summaries from the office server (read-only)."),
        Err(e) => e.to_string(),
    };
    view(&shell, Some(message))
}

/// "Unpair this PC": tell the server if it can be reached, then always unpair locally.
///
/// # Errors
/// A store failure, worded for the user.
pub async fn unpair(shell: Arc<Shell>) -> Result<LinkView, String> {
    let link = shell.with_store(|s| s.server_link()).map_err(store_error)?;
    let mut message = None;
    if let Some(l) = &link {
        // Already revoked on the server is as good as told.
        let told = match unpair_remote(l, &DpapiUserVault, LinkOptions::default()).await {
            Ok(()) | Err(FlowError::Revoked) => true,
            Err(_) => false,
        };
        if !told {
            message = Some(
                "Unpaired on this PC. The office server could not be told; your administrator can remove it there."
                    .to_owned(),
            );
        }
        shell
            .with_store(|s| s.unpair_locally())
            .map_err(store_error)?;
    }
    view(&shell, message)
}

/// "Server address": an IP address (optionally with a port) to try when the server's
/// address changed and its name cannot be found. Empty clears it. The address is only
/// a place to look: the server must still present the pinned certificate there.
///
/// # Errors
/// An address that is not an IP address, or a store failure, worded for the user.
pub fn set_address(shell: &Shell, address: &str) -> Result<LinkView, String> {
    let a = address.trim();
    let value = if a.is_empty() {
        None
    } else if a.parse::<std::net::SocketAddr>().is_ok() || a.parse::<std::net::IpAddr>().is_ok() {
        Some(a)
    } else {
        return Err("Enter an IP address such as 192.168.1.20 or 192.168.1.20:8443.".to_owned());
    };
    let paired = shell
        .with_store(|s| s.server_link())
        .map_err(store_error)?
        .is_some();
    if !paired {
        return Err("Pair this PC first.".to_owned());
    }
    shell
        .with_store(|s| s.set_link_manual_address(value))
        .map_err(store_error)?;
    view(shell, None)
}

/// The user saw the new-device notice for `device_ref`.
///
/// # Errors
/// A malformed id or a store failure, worded for the user.
pub fn notice_seen(shell: &Shell, device_ref: &str) -> Result<LinkView, String> {
    let id = Uuid::parse_str(device_ref).map_err(|_| "Unknown PC.".to_owned())?;
    shell
        .with_store(|s| s.mark_device_notice_shown(id))
        .map_err(store_error)?;
    view(shell, None)
}
