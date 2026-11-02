//! W1: companion updates from the office server (architecture 8; V1-6 section 7).
//!
//! The real server, the companion's real client and download flow
//! ([`hy_core::link::flow::fetch_update`]) and the file route's whole check
//! ([`hy_core::update::check_update_file`]), with the CI test release key from
//! `crates/hy-core/tests/fixtures/update` (no production key exists or is added).
//!
//! - Only a paired, confirmed PC's token is served; no token, an unknown token, a pending
//!   PC and a revoked PC are refused on every update endpoint.
//! - The server hands over the administrator's two files byte for byte.
//! - A tampered installer, an older version and a missing signature are refused before
//!   anything could be installed, with exactly the file route's messages.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use common::{Harness, admin_login, admin_request, error_of, http, opts, test_config, test_limits};
use hy_core::link::flow::{FlowError, UpdateFetch, UpdateFetchError, fetch_update};
use hy_core::protocol::wire::{UPDATE_CHUNK, UpdateManifest, UpdateSignature, WireError};
use hy_core::secret::Secret;
use hy_core::store::ServerLink;
use hy_core::update::{
    Plan, Product, TrustedKeys, UpdateError, Version, check_update_file, sig_path_for,
};
use reqwest::Method;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../hy-core/tests/fixtures/update"
);
/// The one fully signed companion release among the fixtures.
const V011: &str = "HealthierYou_0.1.1_x64-setup.exe";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

/// The CI test key the fixtures are signed with.
fn keys() -> TrustedKeys {
    TrustedKeys::parse(&std::fs::read_to_string(fixture("test.pub")).unwrap()).unwrap()
}

fn v(s: &str) -> Version {
    s.parse().unwrap()
}

/// A server with a drop folder (kept alive by the returned guard).
async fn harness() -> (Harness, tempfile::TempDir) {
    let drop = tempfile::tempdir().unwrap();
    let config = hy_server::Config {
        updates_dir: Some(drop.path().to_path_buf()),
        ..test_config()
    };
    (Harness::start_with(config, test_limits()).await, drop)
}

/// The administrator copies the release `fixture_name` into the drop folder as `as_name`,
/// with or without its `.sig`; `change` may alter the installer's bytes.
fn place(drop: &Path, fixture_name: &str, as_name: &str, with_sig: bool, change: fn(&mut [u8])) {
    let mut bytes = std::fs::read(fixture(&format!("{fixture_name}.payload"))).unwrap();
    change(&mut bytes);
    std::fs::write(drop.join(as_name), &bytes).unwrap();
    if with_sig {
        std::fs::copy(
            fixture(&format!("{fixture_name}.sig")),
            drop.join(format!("{as_name}.sig")),
        )
        .unwrap();
    }
}

fn unchanged(_: &mut [u8]) {}

fn flip_a_byte(b: &mut [u8]) {
    b[5] ^= 0x01;
}

/// The stored link of a paired device, as the companion holds it.
fn link_of(h: &Harness, d: &common::Device) -> ServerLink {
    d.store(h.clock.clone()).server_link().unwrap().unwrap()
}

/// The companion's server route: download, then the file route's whole check.
async fn server_route(
    h: &Harness,
    d: &common::Device,
    current: Version,
    previous: Option<Version>,
    dest: &Path,
) -> Result<Option<Plan>, String> {
    let link = link_of(h, d);
    match fetch_update(&link, &*d.vault, opts(), &keys(), current, previous, dest).await {
        Ok(UpdateFetch::Downloaded(path)) => {
            check_update_file(&keys(), Product::Companion, current, previous, &path)
                .map(|(_, p)| Some(p))
                .map_err(|e| e.to_string())
        }
        Ok(UpdateFetch::NoneOffered | UpdateFetch::UpToDate(_)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// The file route on a copy of the drop folder's files (chosen from a share).
fn file_route(drop: &Path, name: &str, current: Version, previous: Option<Version>) -> String {
    let share = tempfile::tempdir().unwrap();
    let installer = share.path().join(name);
    std::fs::copy(drop.join(name), &installer).unwrap();
    let sig = drop.join(format!("{name}.sig"));
    if sig.exists() {
        std::fs::copy(sig, sig_path_for(&installer)).unwrap();
    }
    check_update_file(&keys(), Product::Companion, current, previous, &installer)
        .expect_err("the file route refuses it")
        .to_string()
}

fn bearer(t: &Secret<[u8; 32]>) -> Option<&Secret<[u8; 32]>> {
    Some(t)
}

const ENDPOINTS: [&str; 3] = [
    "/v1/update/manifest",
    "/v1/update/signature?file=HealthierYou_0.1.1_x64-setup.exe",
    "/v1/update/installer?file=HealthierYou_0.1.1_x64-setup.exe&offset=0",
];

#[tokio::test(flavor = "multi_thread")]
async fn only_a_confirmed_pc_is_served() {
    let (h, drop) = harness().await;
    place(drop.path(), V011, V011, true, unchanged);
    let c = h.client(1);
    // No token, and a token the server does not know: refused, nothing served.
    for path in ENDPOINTS {
        let raw = c.raw(Method::GET, path, None, None).await.unwrap();
        assert_eq!(
            (raw.status, error_of(&raw).as_str()),
            (401, "unknown_token"),
            "{path}"
        );
        let raw = c
            .raw(Method::GET, path, bearer(&Secret::new([9; 32])), None)
            .await
            .unwrap();
        assert_eq!(
            (raw.status, error_of(&raw).as_str()),
            (401, "unknown_token"),
            "{path}"
        );
    }
    // A paired PC the administrator has not confirmed (a stolen slip, say).
    let u = h.user("u1");
    let (_, pending) = h.pair(&h.slip(&u), "PC-1").await.unwrap();
    for path in ENDPOINTS {
        let raw = c
            .raw(Method::GET, path, bearer(&pending.token), None)
            .await
            .unwrap();
        assert_eq!(
            (raw.status, error_of(&raw).as_str()),
            (403, "device_unconfirmed"),
            "{path}"
        );
    }
    let dest = tempfile::tempdir().unwrap();
    let err = fetch_update(
        &link_of(&h, &pending),
        &*pending.vault,
        opts(),
        &keys(),
        v("0.1.0"),
        None,
        &dest.path().join("downloaded"),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            UpdateFetchError::Link(FlowError::Server(WireError::DeviceUnconfirmed))
        ),
        "{err:?}"
    );
    assert!(
        !dest.path().join("downloaded").exists(),
        "nothing downloaded"
    );
    // Confirmed: served.
    h.confirm(&pending);
    let raw = c
        .raw(Method::GET, ENDPOINTS[0], bearer(&pending.token), None)
        .await
        .unwrap();
    assert_eq!(raw.status, 200);
    // Revoked: refused again.
    h.server.revoke_device(&pending.device_ref).unwrap();
    for path in ENDPOINTS {
        let raw = c
            .raw(Method::GET, path, bearer(&pending.token), None)
            .await
            .unwrap();
        assert_eq!(
            (raw.status, error_of(&raw).as_str()),
            (401, "device_revoked"),
            "{path}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_hands_over_the_same_two_files() {
    let (h, drop) = harness().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let c = h.client(1);
    // Nothing placed: nothing offered.
    let m: UpdateManifest = serde_json::from_slice(
        &c.raw(Method::GET, ENDPOINTS[0], bearer(&d.token), None)
            .await
            .unwrap()
            .body,
    )
    .unwrap();
    assert_eq!(m, UpdateManifest { offer: None });
    let dest = tempfile::tempdir().unwrap();
    let downloaded = dest.path().join("downloaded");
    assert_eq!(
        fetch_update(
            &link_of(&h, &d),
            &*d.vault,
            opts(),
            &keys(),
            v("0.1.0"),
            None,
            &downloaded
        )
        .await
        .unwrap(),
        UpdateFetch::NoneOffered
    );

    // The administrator places the release (and an older one, and a server installer).
    place(drop.path(), V011, V011, true, unchanged);
    std::fs::write(drop.path().join("HealthierYou_0.0.9_x64-setup.exe"), b"old").unwrap();
    std::fs::write(
        drop.path().join("HealthierYou-Server_9.0.0_x64-setup.exe"),
        b"srv",
    )
    .unwrap();
    let m = c.update_manifest(&d.token).await.unwrap();
    let offer = m.offer.unwrap();
    assert_eq!(offer.file_name, V011, "the newest companion installer");
    let got = fetch_update(
        &link_of(&h, &d),
        &*d.vault,
        opts(),
        &keys(),
        v("0.1.0"),
        None,
        &downloaded,
    )
    .await
    .unwrap();
    let UpdateFetch::Downloaded(path) = got else {
        panic!("{got:?}")
    };
    assert_eq!(path, downloaded.join(V011));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(drop.path().join(V011)).unwrap()
    );
    assert_eq!(
        std::fs::read(sig_path_for(&path)).unwrap(),
        std::fs::read(drop.path().join(format!("{V011}.sig"))).unwrap()
    );
    assert_eq!(
        std::fs::read_dir(&downloaded).unwrap().count(),
        2,
        "exactly the two files"
    );
    let (release, plan) =
        check_update_file(&keys(), Product::Companion, v("0.1.0"), None, &path).unwrap();
    assert_eq!((release.version, plan), (v("0.1.1"), Plan::Upgrade));
    // The PC already on that version is told so, and downloads nothing.
    let again = tempfile::tempdir().unwrap();
    assert_eq!(
        fetch_update(
            &link_of(&h, &d),
            &*d.vault,
            opts(),
            &keys(),
            v("0.1.1"),
            None,
            again.path()
        )
        .await
        .unwrap(),
        UpdateFetch::UpToDate(v("0.1.1"))
    );
    // Going back to exactly the version this PC came from is allowed, as from a file.
    let back = tempfile::tempdir().unwrap();
    assert_eq!(
        server_route(&h, &d, v("0.2.0"), Some(v("0.1.1")), back.path()).await,
        Ok(Some(Plan::Rollback))
    );

    // Only the offered file is served, and only from inside it.
    for path in [
        "/v1/update/signature?file=HealthierYou_0.0.9_x64-setup.exe",
        "/v1/update/installer?file=HealthierYou_0.0.9_x64-setup.exe&offset=0",
        "/v1/update/installer?file=..%2Fserver.db&offset=0",
        "/v1/update/signature?file=HealthierYou-Server_9.0.0_x64-setup.exe",
    ] {
        let raw = c
            .raw(Method::GET, path, bearer(&d.token), None)
            .await
            .unwrap();
        assert_eq!(
            (raw.status, error_of(&raw).as_str()),
            (404, "protocol_unsupported"),
            "{path}"
        );
    }
    for path in [
        "/v1/update/signature",
        "/v1/update/installer?file=HealthierYou_0.1.1_x64-setup.exe",
        "/v1/update/installer?file=HealthierYou_0.1.1_x64-setup.exe&offset=x",
        "/v1/update/installer?file=HealthierYou_0.1.1_x64-setup.exe&offset=66",
    ] {
        let raw = c
            .raw(Method::GET, path, bearer(&d.token), None)
            .await
            .unwrap();
        assert_eq!(
            (raw.status, error_of(&raw).as_str()),
            (400, "schema_invalid"),
            "{path}"
        );
    }
    let sig: UpdateSignature = serde_json::from_slice(
        &c.raw(Method::GET, ENDPOINTS[1], bearer(&d.token), None)
            .await
            .unwrap()
            .body,
    )
    .unwrap();
    assert_eq!(
        sig.signature,
        std::fs::read_to_string(drop.path().join(format!("{V011}.sig"))).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_installer_comes_in_pieces_each_within_the_limits() {
    let (h, drop) = harness().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let name = "HealthierYou_2.0.0_x64-setup.exe";
    let bytes: Vec<u8> = (0..2 * UPDATE_CHUNK + 123)
        .map(|i| (i % 253) as u8)
        .collect();
    std::fs::write(drop.path().join(name), &bytes).unwrap();
    let c = h.client(1);
    let offer = c.update_manifest(&d.token).await.unwrap().offer.unwrap();
    assert_eq!(offer.size, bytes.len() as u64);
    let mut got = Vec::new();
    while (got.len() as u64) < offer.size {
        let piece = c
            .update_chunk(&d.token, name, got.len() as u64)
            .await
            .unwrap();
        assert!(!piece.is_empty() && piece.len() <= UPDATE_CHUNK);
        got.extend_from_slice(&piece);
    }
    assert_eq!(got, bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tampered_older_or_unsigned_file_is_refused_with_the_file_routes_message() {
    // Tampered: the installer changed after it was signed. Downloaded (the signature is
    // fine on its own), then refused by the whole check, before anything is staged.
    let (h, drop) = harness().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    place(drop.path(), V011, V011, true, flip_a_byte);
    let dest = tempfile::tempdir().unwrap();
    let got = server_route(&h, &d, v("0.1.0"), None, dest.path()).await;
    let expected = file_route(drop.path(), V011, v("0.1.0"), None);
    assert_eq!(expected, UpdateError::Tampered.to_string());
    assert_eq!(got, Err(expected));

    // Older: a version below this PC's that is not the one it came from. Refused from the
    // offer, before anything is downloaded.
    place(drop.path(), V011, V011, true, unchanged);
    let dest = tempfile::tempdir().unwrap();
    let downloaded = dest.path().join("downloaded");
    let got = server_route(&h, &d, v("0.2.0"), Some(v("0.1.9")), &downloaded).await;
    let expected = file_route(drop.path(), V011, v("0.2.0"), Some(v("0.1.9")));
    assert!(
        expected.contains("older than the installed 0.2.0"),
        "{expected}"
    );
    assert_eq!(got, Err(expected));
    assert!(!downloaded.exists(), "nothing downloaded");

    // Missing signature: the administrator copied only the installer. Refused as a file
    // without its .sig is, and the installer is not downloaded.
    std::fs::remove_file(drop.path().join(format!("{V011}.sig"))).unwrap();
    let got = server_route(&h, &d, v("0.1.0"), None, &downloaded).await;
    let expected = file_route(drop.path(), V011, v("0.1.0"), None);
    assert!(
        expected.starts_with("The signature file is missing"),
        "{expected}"
    );
    assert_eq!(got, Err(expected));
    assert!(
        !downloaded.join(V011).exists(),
        "the installer was not downloaded"
    );
    assert!(!downloaded.join(format!("{V011}.part")).exists());

    // Signed with a key this build does not trust: refused before the download too.
    place(drop.path(), V011, V011, true, unchanged);
    let other =
        TrustedKeys::parse(&std::fs::read_to_string(fixture("other.pub")).unwrap()).unwrap();
    let err = fetch_update(
        &link_of(&h, &d),
        &*d.vault,
        opts(),
        &other,
        v("0.1.0"),
        None,
        &downloaded,
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), UpdateError::UnknownKey.to_string());
    assert!(!downloaded.join(V011).exists());

    // A renamed release (0.1.1 placed as 0.1.3): the signed name differs.
    std::fs::remove_file(drop.path().join(V011)).unwrap();
    std::fs::remove_file(drop.path().join(format!("{V011}.sig"))).unwrap();
    let renamed = "HealthierYou_0.1.3_x64-setup.exe";
    place(drop.path(), V011, renamed, true, unchanged);
    let got = server_route(&h, &d, v("0.1.0"), None, &downloaded).await;
    assert_eq!(got, Err(file_route(drop.path(), renamed, v("0.1.0"), None)));
    assert!(!downloaded.join(renamed).exists());

    // A build with no key (every build today: no production key exists) refuses it all.
    let err = fetch_update(
        &link_of(&h, &d),
        &*d.vault,
        opts(),
        &TrustedKeys::default(),
        v("0.1.0"),
        None,
        &downloaded,
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), UpdateError::NoKey.to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_admin_page_shows_what_is_offered() {
    let (h, drop) = harness().await;
    h.server
        .set_admin_passphrase(&Secret::new("correct horse battery".to_owned()))
        .unwrap();
    let cookie = admin_login(&h, "correct horse battery").await;
    let host = format!("127.0.0.1:{}", h.admin.port());
    let page = || async {
        http(
            h.admin,
            &admin_request("GET", "/", &host, None, Some(&cookie), None),
        )
        .await
        .2
    };
    assert!(page().await.contains("Offered now: nothing."));
    place(drop.path(), V011, V011, false, unchanged);
    let p = page().await;
    assert!(p.contains(V011) && p.contains("version 0.1.1"), "{p}");
    assert!(p.contains("every PC will refuse this"));
    place(drop.path(), V011, V011, true, unchanged);
    assert!(!page().await.contains("every PC will refuse this"));
}
