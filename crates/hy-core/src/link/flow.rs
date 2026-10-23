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

use std::net::SocketAddr;

use uuid::Uuid;

use super::{LinkClient, LinkError, LinkOptions, resolve};
use crate::protocol::slip::{Slip, SlipError, encode_secret};
use crate::protocol::wire::{PairRequest, StateResponse, WireEntry, WireError};
use crate::protocol::words::{device_check_code, fingerprint_words};
use crate::protocol::{PROTOCOL, ct_eq32, unb64_exact};
use crate::secret::{Secret, TokenVault};
use crate::store::{NewLink, ServerLink};

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

/// The addresses to try for a stored link, in order: the last address that passed the
/// pin, the slip's host through the OS resolver, then a manual address (contract 5).
pub async fn candidate_addresses(link: &ServerLink) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    if let Some(a) = link.last_ip.as_deref().and_then(|s| s.parse().ok()) {
        out.push(a);
    }
    for a in resolve(&link.host, link.port).await {
        if !out.contains(&a) {
            out.push(a);
        }
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
    let mut last = FlowError::Unreachable;
    for addr in candidate_addresses(link).await {
        let client = LinkClient::new(link.pin, addr, opts)?;
        match fetch_pages(&client, &token, after_seq).await {
            Ok(state) => return Ok(Fetched { state, addr }),
            Err(FlowError::Unreachable) => last = FlowError::Unreachable,
            Err(e) => return Err(e),
        }
    }
    Err(last)
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
    let mut last = FlowError::Unreachable;
    for addr in candidate_addresses(link).await {
        let client = LinkClient::new(link.pin, addr, opts)?;
        match client.unpair(&token).await {
            Ok(()) => return Ok(()),
            Err(LinkError::Unreachable) => last = FlowError::Unreachable,
            Err(e) => return Err(e.into()),
        }
    }
    Err(last)
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
    async fn addresses_are_tried_last_good_first_then_the_name_then_the_manual_one() {
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
            ["10.0.0.7:8443", "127.0.0.1:8443", "192.168.1.20:8443"]
        );
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
