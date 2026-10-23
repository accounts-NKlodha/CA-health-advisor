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
//! - **Unpair:** the server is told when it can be reached, and the local unpair
//!   transaction always runs. A revoked PC unpairs locally on its next check.
//!
//! Nothing here holds the store across a network wait. The token is opened only for the
//! duration of one request and never logged or shown.

use std::sync::Arc;

use hy_core::link::LinkOptions;
use hy_core::link::flow::{DeviceFacts, FlowError, fetch_state, redeem_slip, unpair_remote};
use hy_core::protocol::slip::Slip;
use hy_core::protocol::words::{device_check_code, fingerprint_words};
use hy_core::secret::{Secret, TokenVault};
use hy_core::store::{ServerLink, StateOutcome};
use serde::Serialize;
use uuid::Uuid;

use crate::shell::Shell;

/// The notice the pairing screen shows (recorded in the consent log).
pub const PAIRING_NOTICE: &str = "v1-5-pairing-1";

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
    /// The user's PCs (after confirmation).
    pub devices: Vec<PairedPc>,
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
                devices: Vec::new(),
                message,
            },
            Some(l) => Self {
                state: if l.confirmed { "confirmed" } else { "pending" },
                alias: Some(l.alias.clone()),
                host: Some(l.host.clone()),
                words: fingerprint_words(&l.pin).to_vec(),
                check_code: Some(device_check_code(l.device_ref.as_bytes())),
                confirmed_on: l.confirmed_on.map(|d| d.to_string()),
                devices,
                message,
            },
        }
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
        Ok(LinkView::of(link.as_ref(), devices, message))
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

/// "Check again" (and the periodic check): ask the server for this PC's state.
///
/// # Errors
/// A store failure, worded for the user. Network and server problems come back as the
/// view's message, with the link unchanged.
pub async fn refresh(shell: Arc<Shell>) -> Result<LinkView, String> {
    let Some((link, after)) = shell
        .with_store(|s| -> hy_core::Result<_> {
            let link = s.server_link()?;
            let after = s.head_receipt()?.map_or(0, |r| r.head.seq);
            Ok(link.map(|l| (l, after)))
        })
        .map_err(store_error)?
    else {
        return view(&shell, None);
    };
    let message = match fetch_state(&link, after, &DpapiUserVault, LinkOptions::default()).await {
        Ok(fetched) => shell
            .with_store(|s| -> hy_core::Result<Option<String>> {
                s.set_link_last_ip(&fetched.addr.to_string())?;
                Ok(match s.apply_confirmed_state(&fetched.state)? {
                    None => Some("Waiting for your administrator to confirm this PC.".to_owned()),
                    Some(StateOutcome::Applied { .. }) => None,
                    Some(StateOutcome::HistoryMismatch) => Some(
                        "Server history mismatch: sharing is paused. Tell your administrator."
                            .to_owned(),
                    ),
                })
            })
            .map_err(store_error)?,
        Err(FlowError::Revoked) => {
            shell
                .with_store(|s| s.unpair_locally())
                .map_err(store_error)?;
            Some(FlowError::Revoked.to_string())
        }
        Err(e) => Some(e.to_string()),
    };
    view(&shell, message)
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
