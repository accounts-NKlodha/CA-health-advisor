//! HealthierYou office LAN server (architecture 5; V1-5 contract
//! `docs/V1-5_Pairing_Security.md`).
//!
//! Slice V1-5a provides the service core:
//!
//! - [`keys`]: the TLS key and certificate (the pin) and the separate Ed25519
//!   head-signing key, protected with machine-scope DPAPI on Windows;
//! - [`tls`] and [`serve`]: the TLS 1.3-only listener with ALPN `hy/1`, no 0-RTT, no
//!   resumption, and connection limits; the loopback-only admin listener;
//! - [`server`]: users, slips (atomic single use, 24 h, at most three outstanding, rate
//!   limits), pending devices, mandatory administrator confirmation, tokens (only their
//!   hash is stored), revocation and self-unpair, and the pending-device expiry;
//! - [`chain`]: the per-user control log written in the same transaction as each effect,
//!   and signed heads;
//! - [`api`]: `/v1/pair`, `/v1/state`, `/v1/unpair`, and the pending-device gate on every
//!   other endpoint;
//! - [`admin`]: the hardened admin page;
//! - [`logging`] and [`audit`]: allowlisted logs and the audit log.
//!
//! Sync (history, control, upload, receipts), the off-volume mirror, recovery hold and
//! server backup are V1-5b.

pub mod admin;
pub mod api;
pub mod audit;
pub mod chain;
pub mod db;
#[cfg(windows)]
pub mod dpapi;
pub mod keys;
pub mod logging;
pub mod serve;
pub mod server;
pub mod tls;

pub use server::{Config, Server};
