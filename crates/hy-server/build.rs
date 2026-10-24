//! Build script: embeds the release update keys (slice V1-6b).
//!
//! `HY_UPDATE_KEYS`: the `HY_UPDATE_PUBKEY` environment variable if it is set (the CI
//! job's keys made for the run), otherwise the public keys in `release/update-keys.txt`.
//! A build with no key refuses every update file (`hy-server verify-release`). The same
//! keys sign the companion's and the server's installers.

fn update_keys() -> String {
    println!("cargo:rerun-if-env-changed=HY_UPDATE_PUBKEY");
    if let Ok(k) = std::env::var("HY_UPDATE_PUBKEY") {
        return k.split_whitespace().collect::<Vec<_>>().join(";");
    }
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/update-keys.txt");
    println!("cargo:rerun-if-changed={}", file.display());
    std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join(";")
}

fn main() {
    println!("cargo:rustc-env=HY_UPDATE_KEYS={}", update_keys());
}
