//! The companion's pairing and status steps (V1-5 contract 4.2 steps 3 to 10, 6.3 and
//! 12), shared with the simulator and tested on Linux against the real server.
//!
//! The network steps are async and never hold the store; the store steps are
//! synchronous single transactions. A caller runs, for example:
//!
//! ```text
//! let link = redeem_slip(&slip, ...).await?;      // steps 4-6: pinned TLS, POST /v1/pair
//! store.save_pairing(&link, notice_version)?;     // step 7: one local transaction
//! let state = fetch_state(&store_link, ...).await; // step 10, every sync
//! store.apply_confirmed_state(&state)?;           // verified chain extension, receipt
//! ```
//!
//! [`sync_once`] runs one whole sync (V1-5b, contract 7.4 and 8): pull and verify, send
//! pending control operations, pull again if any was accepted, queue this PC's records
//! and upload them. During a recovery hold it presents the stored receipt as evidence
//! and sends control operations (disables and deletions are taken, enables wait), but
//! uploads nothing. On `device_regressed` it runs [`Store::regression_reset`] and pulls
//! again; on `device_revoked` it unpairs locally.

use std::net::SocketAddr;
use std::sync::{Mutex, PoisonError};

use chrono::NaiveDate;
use uuid::Uuid;

use super::{LinkClient, LinkError, LinkOptions, resolve};
use crate::protocol::slip::{Slip, SlipError, encode_secret};
use crate::protocol::wire::{
    ControlOutcome, ControlRequest, MAX_UPLOAD_ROWS, PairRequest, RowOutcome, StateResponse,
    UploadRequest, UploadRow, WireEntry, WireError,
};
use crate::protocol::words::{device_check_code, fingerprint_words};
use crate::protocol::{PROTOCOL, ct_eq32, unb64_exact};
use crate::secret::{Secret, TokenVault};
use crate::store::{NewLink, ServerLink, StateOutcome, Store};

/// Why pairing or a status check failed, in the terms the companion shows (contract 12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowError {
    /// The slip text is mistyped or not a slip (caught locally).
    Slip(SlipError),
    /// The server is not the one on the slip, or its identity changed. Nothing was sent.
    PinMismatch {
        /// Prefix of the pin.
        expected: String,
        /// Prefix of what was presented.
        seen: String,
    },
    /// No address for the server answered.
    Unreachable,
    /// The TLS handshake failed (old TLS, wrong protocol, bad signature).
    Tls,
    /// The server refused (`slip_invalid`, `rate_limited`, `origin_conflict`,
    /// `recovery_hold`, `device_unconfirmed`...).
    Server(WireError),
    /// This PC was revoked: run the local unpair transaction and offer to re-pair.
    Revoked,
    /// The token cannot be opened here (another Windows user or PC, or a reinstall):
    /// "re-pair needed". Local data is untouched.
    RepairNeeded,
    /// The server's answer was malformed or inconsistent.
    Protocol,
}

impl From<LinkError> for FlowError {
    fn from(e: LinkError) -> Self {
        match e {
            LinkError::PinMismatch { expected, seen } => Self::PinMismatch { expected, seen },
            LinkError::Tls => Self::Tls,
            LinkError::Unreachable => Self::Unreachable,
            LinkError::Server {
                error: WireError::DeviceRevoked,
                ..
            } => Self::Revoked,
            LinkError::Server { error, .. } => Self::Server(error),
            LinkError::Protocol => Self::Protocol,
        }
    }
}

impl std::fmt::Display for FlowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Slip(e) => return write!(f, "{e}"),
            Self::PinMismatch { .. } => {
                "This is not the office server named on your slip. Do not continue; tell your administrator."
            }
            Self::Unreachable => "The office server can't be reached right now.",
            Self::Tls => "The office server's connection was refused for security reasons.",
            Self::Server(WireError::SlipInvalid) => "This slip can't be used. Ask for a new one.",
            Self::Server(WireError::RateLimited) => "Too many attempts. Try again in a few minutes.",
            Self::Server(WireError::RecoveryHold) => "Server recovering: sharing paused.",
            Self::Server(WireError::OriginConflict) => {
                "This PC is paired to someone else on the office server. Ask your administrator."
            }
            Self::Server(WireError::DeviceUnconfirmed) => {
                "Waiting for your administrator to confirm this PC."
            }
            Self::Server(_) | Self::Protocol => "The office server gave an unexpected answer.",
            Self::Revoked => "This PC was removed from the office server. You can pair again.",
            Self::RepairNeeded => "Re-pair needed: this PC's server key can't be opened here.",
        })
    }
}

impl std::error::Error for FlowError {}

/// What a successful redemption returns, before the caller stores it.
#[derive(Debug, Clone)]
pub struct Redeemed {
    /// The link to store with [`crate::store::Store::save_pairing`].
    pub link: NewLink,
    /// The six words of the pin this PC holds (shown for the confirmation).
    pub words: [&'static str; 6],
    /// This device's check code (shown for the confirmation).
    pub check_code: String,
}

/// The device facts sent at pairing.
#[derive(Debug, Clone)]
pub struct DeviceFacts {
    /// This PC's origin-device id.
    pub origin_device: Uuid,
    /// A label the user recognises.
    pub label: String,
    /// The companion version.
    pub app_version: String,
    /// The Windows build.
    pub windows_build: String,
}

/// Steps 4 to 6 of the ceremony: resolve the slip's host, open pinned TLS 1.3 to each
/// address in turn, send `POST /v1/pair` once, and check the server's fingerprint equals
/// the pin. A pin mismatch stops at once (it never tries another address). The token is
/// sealed by `vault` before it leaves this function.
///
/// # Errors
/// [`FlowError`]; nothing is stored by this function.
pub async fn redeem_slip(
    slip: &Slip,
    facts: &DeviceFacts,
    vault: &dyn TokenVault,
    opts: LinkOptions,
) -> Result<Redeemed, FlowError> {
    let addrs = resolve(&slip.host, slip.port).await;
    let req = PairRequest {
        protocol: PROTOCOL,
        secret: encode_secret(slip.secret.expose()).expose().clone(),
        origin_device: facts.origin_device,
        device_label: facts.label.clone(),
        app_version: facts.app_version.clone(),
        windows_build: facts.windows_build.clone(),
    };
    let mut last = FlowError::Unreachable;
    for addr in addrs {
        let client = LinkClient::new(slip.fingerprint, addr, opts)?;
        match client.pair(&req).await {
            Ok(resp) => {
                let fp: [u8; 32] =
                    unb64_exact(&resp.server_fingerprint).ok_or(FlowError::Protocol)?;
                if !ct_eq32(&fp, &slip.fingerprint) {
                    return Err(FlowError::Protocol);
                }
                let head_key: [u8; 32] = unb64_exact(&resp.head_key).ok_or(FlowError::Protocol)?;
                let token = Secret::new(unb64_exact::<32>(&resp.token).ok_or(FlowError::Protocol)?);
                let token_blob = vault.seal(&token).map_err(|_| FlowError::RepairNeeded)?;
                return Ok(Redeemed {
                    words: fingerprint_words(&slip.fingerprint),
                    check_code: device_check_code(resp.device_ref.as_bytes()),
                    link: NewLink {
                        pin: slip.fingerprint,
                        head_key,
                        host: slip.host.clone(),
                        port: slip.port,
                        last_ip: Some(addr.to_string()),
                        user_id: resp.user_id,
                        device_ref: resp.device_ref,
                        alias: resp.alias,
                        token_blob,
                    },
                });
            }
            Err(LinkError::Unreachable) => last = FlowError::Unreachable,
            Err(e) => return Err(e.into()),
        }
    }
    Err(last)
}

/// The addresses known without a name lookup, in order: the last address that passed
/// the pin, then the manual address. They are tried before the OS resolver is asked, so
/// a failing or slow name lookup never delays recovery after the server's IP changed.
#[must_use]
pub fn known_addresses(link: &ServerLink) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    if let Some(a) = link.last_ip.as_deref().and_then(|s| s.parse().ok()) {
        out.push(a);
    }
    if let Some(m) = link.manual_address.as_deref() {
        let parsed: Option<SocketAddr> = m
            .parse()
            .ok()
            .or_else(|| m.parse().ok().map(|ip| SocketAddr::new(ip, link.port)));
        if let Some(a) = parsed
            && !out.contains(&a)
        {
            out.push(a);
        }
    }
    out
}

/// Every address to try for a stored link, in order: [`known_addresses`], then the
/// slip's host through the OS resolver (contract 5).
pub async fn candidate_addresses(link: &ServerLink) -> Vec<SocketAddr> {
    let mut out = known_addresses(link);
    for a in resolve(&link.host, link.port).await {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

/// Run `attempt` against each address in turn until one is reachable: the known
/// addresses first, and only if none answers, the resolved ones (the lookup is bounded
/// by the connect timeout).
async fn try_addresses<T, F, Fut>(
    link: &ServerLink,
    opts: LinkOptions,
    mut attempt: F,
) -> Result<(T, SocketAddr), FlowError>
where
    F: FnMut(SocketAddr) -> Fut,
    Fut: std::future::Future<Output = Result<T, FlowError>>,
{
    let known = known_addresses(link);
    for &addr in &known {
        match attempt(addr).await {
            Err(FlowError::Unreachable) => {}
            Ok(v) => return Ok((v, addr)),
            Err(e) => return Err(e),
        }
    }
    let resolved = tokio::time::timeout(opts.connect_timeout, resolve(&link.host, link.port))
        .await
        .unwrap_or_default();
    for addr in resolved {
        if known.contains(&addr) {
            continue;
        }
        match attempt(addr).await {
            Err(FlowError::Unreachable) => {}
            Ok(v) => return Ok((v, addr)),
            Err(e) => return Err(e),
        }
    }
    Err(FlowError::Unreachable)
}

/// What a status check found.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// The state, with every page of entries joined.
    pub state: StateResponse,
    /// The address that answered (store it as the last address).
    pub addr: SocketAddr,
}

/// `GET /v1/state` for a stored link, trying each candidate address, and joining pages
/// of chain entries after `after_seq` (the receipt's seq) until the last one.
///
/// # Errors
/// [`FlowError::RepairNeeded`] if the token cannot be opened; [`FlowError::Revoked`] for
/// `device_revoked`; other [`FlowError`]s as for pairing.
pub async fn fetch_state(
    link: &ServerLink,
    after_seq: u64,
    vault: &dyn TokenVault,
    opts: LinkOptions,
) -> Result<Fetched, FlowError> {
    let token = vault
        .open(&link.token_blob)
        .map_err(|_| FlowError::RepairNeeded)?;
    let (state, addr) = try_addresses(link, opts, |addr| {
        let token = &token;
        async move {
            let client = LinkClient::new(link.pin, addr, opts)?;
            fetch_pages(&client, token, after_seq).await
        }
    })
    .await?;
    Ok(Fetched { state, addr })
}

async fn fetch_pages(
    client: &LinkClient,
    token: &Secret<[u8; 32]>,
    after_seq: u64,
) -> Result<StateResponse, FlowError> {
    let mut after = after_seq;
    let mut joined: Vec<WireEntry> = Vec::new();
    loop {
        let page = client.state(token, after).await?;
        match page {
            StateResponse::Pending { .. } => return Ok(page),
            StateResponse::Confirmed {
                alias,
                confirmed_on,
                devices,
                entries,
                more,
                head,
                hold,
            } => {
                let next = entries.last().map(|e| e.seq);
                joined.extend(entries);
                if more {
                    after = next.ok_or(FlowError::Protocol)?;
                    if joined.len() > 1_000_000 {
                        return Err(FlowError::Protocol);
                    }
                    continue;
                }
                return Ok(StateResponse::Confirmed {
                    alias,
                    confirmed_on,
                    devices,
                    entries: joined,
                    more: false,
                    head,
                    hold,
                });
            }
        }
    }
}

/// `POST /v1/unpair` for a stored link (best effort: the caller unpairs locally whether
/// or not this succeeds, contract 6.3).
///
/// # Errors
/// [`FlowError`] if the server could not be told.
pub async fn unpair_remote(
    link: &ServerLink,
    vault: &dyn TokenVault,
    opts: LinkOptions,
) -> Result<(), FlowError> {
    let token = vault
        .open(&link.token_blob)
        .map_err(|_| FlowError::RepairNeeded)?;
    try_addresses(link, opts, |addr| {
        let token = &token;
        async move {
            let client = LinkClient::new(link.pin, addr, opts)?;
            client.unpair(token).await.map_err(FlowError::from)
        }
    })
    .await?;
    Ok(())
}

/// Short synchronous access to the store, never held across a network wait.
pub trait StoreAccess {
    /// Run `f` with the store.
    fn with<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R;
}

impl StoreAccess for Mutex<Store> {
    fn with<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        f(&mut self.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// What one [`sync_once`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// This PC is not paired.
    NotPaired,
    /// Waiting for the administrator's confirmation.
    Pending,
    /// The server's history did not verify: nothing was applied or sent.
    HistoryMismatch,
    /// The server holds this user in recovery: sharing paused.
    Hold {
        /// The stored receipt was counted as evidence.
        evidence: bool,
        /// Control operations sent.
        control_sent: usize,
    },
    /// A normal sync.
    Synced {
        /// The receipt's seq afterwards.
        seq: u64,
        /// Control operations sent.
        control_sent: usize,
        /// Rows the server stored.
        uploaded: usize,
        /// Rows the server refused (discarded, never re-stamped).
        rejected: usize,
        /// The server answered `device_regressed` and this PC was reset first.
        regressed: bool,
    },
}

fn store_err(_: crate::error::Error) -> FlowError {
    FlowError::Protocol
}

/// The link, the stored receipt (wire form) and its seq.
fn link_and_receipt(
    store: &impl StoreAccess,
) -> Result<Option<(ServerLink, Option<String>, u64)>, FlowError> {
    store
        .with(|s| -> crate::error::Result<_> {
            let Some(link) = s.server_link()? else {
                return Ok(None);
            };
            let receipt = s.head_receipt()?;
            let seq = receipt.as_ref().map_or(0, |r| r.head.seq);
            Ok(Some((link, receipt.map(|r| r.to_wire()), seq)))
        })
        .map_err(store_err)
}

/// Pull the state from the first candidate address that answers.
async fn pull(
    link: &ServerLink,
    receipt: Option<String>,
    after: u64,
    token: &Secret<[u8; 32]>,
    opts: LinkOptions,
) -> Result<(LinkClient, StateResponse, SocketAddr), FlowError> {
    let ((client, state), addr) = try_addresses(link, opts, |addr| {
        let receipt = receipt.clone();
        async move {
            let client = LinkClient::new(link.pin, addr, opts)?.with_receipt(receipt);
            let state = fetch_pages(&client, token, after).await?;
            Ok((client, state))
        }
    })
    .await?;
    Ok((client, state, addr))
}

/// Send one batch of pending control operations; returns (sent, any accepted).
async fn send_control(
    store: &impl StoreAccess,
    client: &LinkClient,
    token: &Secret<[u8; 32]>,
) -> Result<(usize, bool), FlowError> {
    let ops = store.with(|s| s.pending_control_ops()).map_err(store_err)?;
    if ops.is_empty() {
        return Ok((0, false));
    }
    let resp = client
        .control(token, &ControlRequest { ops: ops.clone() })
        .await?;
    if resp.results.len() != ops.len() {
        return Err(FlowError::Protocol);
    }
    store
        .with(|s| s.apply_control_results(&resp.results))
        .map_err(store_err)?;
    let accepted = resp
        .results
        .iter()
        .any(|r| matches!(r.outcome, ControlOutcome::Accepted { .. }));
    Ok((ops.len(), accepted))
}

/// One whole sync (see the module documentation). `today` is the user's local date.
///
/// # Errors
/// [`FlowError::Revoked`] after the local unpair; [`FlowError::RepairNeeded`] if the
/// token cannot be opened; network and server errors otherwise. Whatever was applied
/// before an error stays applied (each store step is its own transaction).
pub async fn sync_once(
    store: &impl StoreAccess,
    vault: &dyn TokenVault,
    opts: LinkOptions,
    today: NaiveDate,
) -> Result<SyncOutcome, FlowError> {
    let Some((link, receipt, after)) = link_and_receipt(store)? else {
        return Ok(SyncOutcome::NotPaired);
    };
    let token = vault
        .open(&link.token_blob)
        .map_err(|_| FlowError::RepairNeeded)?;
    let mut regressed = false;
    let pulled = match pull(&link, receipt.clone(), after, &token, opts).await {
        Err(FlowError::Server(WireError::DeviceRegressed)) => {
            // The server reset this device's baseline; this PC resets its sharing.
            store.with(Store::regression_reset).map_err(store_err)?;
            regressed = true;
            pull(&link, receipt.clone(), after, &token, opts).await
        }
        other => other,
    };
    let (client, state, addr) = match pulled {
        Err(FlowError::Revoked) => {
            store.with(Store::unpair_locally).map_err(store_err)?;
            return Err(FlowError::Revoked);
        }
        other => other?,
    };
    let applied = store
        .with(|s| -> crate::error::Result<_> {
            s.set_link_last_ip(&addr.to_string())?;
            s.apply_confirmed_state(&state)
        })
        .map_err(store_err)?;
    let seq = match applied {
        None => return Ok(SyncOutcome::Pending),
        Some(StateOutcome::HistoryMismatch) => {
            // Report the stored receipt (contract 7.4): a server that rolled back or
            // forked learns of it. Nothing else is sent while the mismatch lasts.
            if let Some(r) = &receipt {
                let _ = client.receipt(&token, r).await;
            }
            return Ok(SyncOutcome::HistoryMismatch);
        }
        Some(StateOutcome::Hold) => {
            let evidence = match &receipt {
                Some(r) => client.receipt(&token, r).await?.recorded,
                None => false,
            };
            let (control_sent, _) = send_control(store, &client, &token).await?;
            return Ok(SyncOutcome::Hold {
                evidence,
                control_sent,
            });
        }
        Some(StateOutcome::Applied { seq, .. }) => seq,
    };
    // From here every request echoes the new receipt.
    let (_, receipt, _) = link_and_receipt(store)?.ok_or(FlowError::Protocol)?;
    let client = client.with_receipt(receipt.clone());
    let (control_sent, accepted) = send_control(store, &client, &token).await?;
    let mut seq = seq;
    if accepted {
        // Accepted operations settle (and take effect) only through the verified chain.
        // This second verification is as strict as the first: anything but a verified
        // extension stops the sync before a single row is uploaded.
        let state = fetch_pages(&client, &token, seq).await?;
        match store
            .with(|s| s.apply_confirmed_state(&state))
            .map_err(store_err)?
        {
            Some(StateOutcome::Applied { seq: s, .. }) => seq = s,
            None => return Ok(SyncOutcome::Pending),
            Some(StateOutcome::HistoryMismatch) => {
                if let Some(r) = &receipt {
                    let _ = client.receipt(&token, r).await;
                }
                return Ok(SyncOutcome::HistoryMismatch);
            }
            Some(StateOutcome::Hold) => {
                return Ok(SyncOutcome::Hold {
                    evidence: false,
                    control_sent,
                });
            }
        }
    }
    let (_, receipt, _) = link_and_receipt(store)?.ok_or(FlowError::Protocol)?;
    let client = client.with_receipt(receipt);
    store
        .with(|s| s.queue_share_records(today))
        .map_err(store_err)?;
    let (mut uploaded, mut rejected) = (0, 0);
    loop {
        // The upload boundary: nothing leaves this PC unless the link is confirmed and
        // its last state verified (no mismatch, no hold), checked before every batch.
        if !store.with(|s| s.uploads_allowed()).map_err(store_err)? {
            break;
        }
        let batch: Vec<_> = store
            .with(|s| s.outbox())
            .map_err(store_err)?
            .into_iter()
            .take(MAX_UPLOAD_ROWS)
            .collect();
        if batch.is_empty() {
            break;
        }
        let ids: Vec<i64> = batch.iter().map(|r| r.id).collect();
        let rows = batch
            .into_iter()
            .map(|r| UploadRow {
                category: r.category,
                origin_device: r.origin_device,
                local_date: r.local_date.format("%Y-%m-%d").to_string(),
                revision: r.revision,
                payload: r.payload,
            })
            .collect();
        let resp = client.upload(&token, &UploadRequest { rows }).await?;
        if resp.results.len() != ids.len() {
            return Err(FlowError::Protocol);
        }
        for r in &resp.results {
            match r {
                RowOutcome::Accepted => uploaded += 1,
                RowOutcome::Rejected { .. } => rejected += 1,
            }
        }
        store
            .with(|s| s.apply_upload_results(&ids, &resp.results))
            .map_err(store_err)?;
    }
    store.with(|s| s.note_sync()).map_err(store_err)?;
    Ok(SyncOutcome::Synced {
        seq,
        control_sent,
        uploaded,
        rejected,
        regressed,
    })
}

/// "Restore from server" (architecture 5.4): read the user's shared history for
/// `from..=to` (at most 90 days) and store it as read-only imported summaries. Returns
/// the rows stored.
///
/// # Errors
/// [`FlowError::Server`] with `recovery_hold` during a hold; others as for a sync.
pub async fn restore_from_server(
    store: &impl StoreAccess,
    vault: &dyn TokenVault,
    opts: LinkOptions,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<usize, FlowError> {
    let Some((link, receipt, _)) = link_and_receipt(store)? else {
        return Err(FlowError::Protocol);
    };
    let token = vault
        .open(&link.token_blob)
        .map_err(|_| FlowError::RepairNeeded)?;
    let (f, t) = (
        from.format("%Y-%m-%d").to_string(),
        to.format("%Y-%m-%d").to_string(),
    );
    let (resp, _) = try_addresses(&link, opts, |addr| {
        let (receipt, token, f, t) = (receipt.clone(), &token, &f, &t);
        let pin = link.pin;
        async move {
            let client = LinkClient::new(pin, addr, opts)?.with_receipt(receipt);
            client.history(token, f, t).await.map_err(FlowError::from)
        }
    })
    .await?;
    store
        .with(|s| s.import_server_history(&resp.rows))
        .map_err(store_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::EphemeralVault;

    fn send<T: Send>(_: &T) {}

    #[test]
    fn the_flows_are_send_so_any_runtime_thread_can_run_them() {
        // The companion runs them as Tauri async commands, which need Send futures.
        let slip = Slip::new("office-pc", 8443, [1; 32], [2; 10]).unwrap();
        let vault = EphemeralVault::new();
        let facts = DeviceFacts {
            origin_device: Uuid::from_bytes([3; 16]),
            label: "PC".into(),
            app_version: "0.5.0".into(),
            windows_build: "26100".into(),
        };
        send(&redeem_slip(&slip, &facts, &vault, LinkOptions::default()));
        let link = ServerLink {
            server_key_id: [0; 32],
            pin: [1; 32],
            head_key: [0; 32],
            host: "office-pc".into(),
            port: 8443,
            last_ip: None,
            manual_address: Some("192.168.1.20".into()),
            user_id: Uuid::from_bytes([4; 16]),
            device_ref: Uuid::from_bytes([5; 16]),
            alias: "u1".into(),
            token_blob: Vec::new(),
            confirmed: false,
            confirmed_on: None,
            paired_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        };
        send(&fetch_state(&link, 0, &vault, LinkOptions::default()));
        send(&unpair_remote(&link, &vault, LinkOptions::default()));
    }

    #[tokio::test]
    async fn known_addresses_come_first_and_never_wait_for_the_name_lookup() {
        let link = ServerLink {
            server_key_id: [0; 32],
            pin: [1; 32],
            head_key: [0; 32],
            host: "127.0.0.1".into(),
            port: 8443,
            last_ip: Some("10.0.0.7:8443".into()),
            manual_address: Some("192.168.1.20".into()),
            user_id: Uuid::from_bytes([4; 16]),
            device_ref: Uuid::from_bytes([5; 16]),
            alias: "u1".into(),
            token_blob: Vec::new(),
            confirmed: false,
            confirmed_on: None,
            paired_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        };
        let got: Vec<String> = candidate_addresses(&link)
            .await
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            got,
            ["10.0.0.7:8443", "192.168.1.20:8443", "127.0.0.1:8443"]
        );
        // The remembered and manual addresses need no lookup at all.
        let known: Vec<String> = known_addresses(&ServerLink {
            host: "no-such-host.invalid".into(),
            ..link
        })
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(known, ["10.0.0.7:8443", "192.168.1.20:8443"]);
    }

    #[test]
    fn a_token_that_cannot_be_opened_is_re_pair_needed_and_revocation_is_its_own_case() {
        assert_eq!(
            FlowError::from(LinkError::Server {
                status: 401,
                error: WireError::DeviceRevoked
            }),
            FlowError::Revoked
        );
        assert!(
            FlowError::RepairNeeded
                .to_string()
                .contains("Re-pair needed")
        );
        assert!(
            FlowError::PinMismatch {
                expected: String::new(),
                seen: String::new()
            }
            .to_string()
            .contains("not the office server")
        );
    }
}
