//! HealthierYou pet shell logic (architecture 4.1, 4.2 and 4.4; slice V1-1).
//!
//! Everything here is pure or store-backed, with no windows, so it is built and tested on
//! every platform. The Windows companion (`apps/companion/src-tauri`) uses it to:
//!
//! - place the pet inside the work area at every size and Windows scaling, and again
//!   after a monitor, resolution, scaling or taskbar change ([`placement`]);
//! - load and check the ten data-only pet packs ([`pack`], [`svg`]);
//! - apply and persist the user's pet edits ([`profile`]) and hotkey ([`hotkey`]);
//! - turn engine requests into window effects ([`shell`], [`state`]), with the shell's
//!   own wording in [`messages`].
//!
//! The coaching engine (slice V1-2) drives the pet through [`shell::PetShell`]; [`link`]
//! connects the two: it draws the engine's view on the shell, routes the user's answers
//! back to the engine, and holds the decisions behind the Windows signals (idle time,
//! presentation and full-screen detection) so they are tested without Windows.

pub mod frame;
pub mod geometry;
pub mod hotkey;
pub mod link;
pub mod messages;
pub mod pack;
pub mod placement;
pub mod profile;
pub mod shell;
pub mod state;
pub mod svg;
