# Licence register and supply chain (tracked work W3)

Architecture 9 and V1-5 contract C18 ask for `cargo-deny`, `npm audit`, a licence register
and an SBOM. This page is the register: what may be shipped, what is shipped, and how CI
holds it. The CI job **Supply chain - cargo-deny, npm audit, licences, SBOM** checks it on
every pull request and every push to `main`.

## What CI checks

| Check | Tool (pinned) | Fails on |
| --- | --- | --- |
| Rust advisories | `cargo-deny` 0.20.2, RustSec database | Any vulnerability, unsound or yanked crate; an unmaintained crate unless ignored in `deny.toml` with its reason |
| Rust licences | `cargo-deny`, `deny.toml` `[licenses]` | A crate whose licence (or every alternative of its licence expression) is not in the allowed list below |
| Rust bans | `cargo-deny`, `deny.toml` `[bans]` | OpenSSL or native-tls in the graph (TLS is rustls only, V1-5 contract 5); a wildcard version. Duplicate versions are reported, not failed (Tauri brings several majors of a few crates) |
| Rust sources | `cargo-deny`, `deny.toml` `[sources]` | Any crate not from crates.io (no git dependencies, no other registry) |
| npm advisories | `npm audit --audit-level=high` in `apps/companion/ui` | A high or critical advisory in any package, build tools included |
| npm licences | `scripts/ci/check-sbom-licences.mjs` over a CycloneDX SBOM of the whole UI tree | A package with no licence, or none of Apache-2.0, MIT, ISC, BSD-3-Clause |
| SBOM | `cargo-cyclonedx` 0.5.9 and `@cyclonedx/cyclonedx-npm` 6.0.1, CycloneDX 1.5 JSON | Kept as the run's artifact `sbom` (90 days): one file per Rust crate of the workspace for the shipped target `x86_64-pc-windows-msvc`, and `hy-companion-ui.cdx.json` for the UI's runtime packages |

`cargo-deny` checks the graph for `x86_64-pc-windows-msvc` (what ships) and
`x86_64-unknown-linux-gnu` (what CI builds). Run it locally with `cargo deny --locked check`.

## Allowed licences

| Licence | Why it is acceptable |
| --- | --- |
| MIT, MIT-0, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-3-Clause, ISC, 0BSD, Zlib, BSL-1.0, Unicode-3.0 | Permissive: use and redistribution in a proprietary binary with the notice kept |
| CC0-1.0, Unlicense | Public-domain dedications |
| MPL-2.0 | File-level copyleft: the MPL files must stay available under MPL, which they are (unmodified, from crates.io). Nothing of ours becomes MPL. Used by `cssparser`, `cssparser-macros`, `dtoa-short`, `selectors` (Tauri's HTML handling) and `option-ext` (through `dirs`) |

Not allowed: GPL, LGPL and AGPL in any form, and anything unlisted. Where a crate offers a
choice (for example `r-efi`: MIT or Apache-2.0 or LGPL-2.1-or-later), a permissive
alternative applies. The workspace's own crates are proprietary (`license = "UNLICENSED"`,
never published) and are not checked against the list.

## What is shipped (as of this register's last change)

Rust, about 375 third-party crates in the checked graph (`cargo deny list -l license` for
the exact list; the SBOM artifact for each build):

| Licence | Crates | Examples |
| --- | --- | --- |
| MIT (often with Apache-2.0) | 339 | tauri, tokio, serde, axum, rusqlite (SQLite itself is public domain), uuid |
| Apache-2.0 (often with MIT) | 233 | rustls, aws-lc-rs, chrono, reqwest, windows |
| Unicode-3.0 | 19 | icu_* (URL handling) |
| ISC | 8 | rustls, rustls-webpki, aws-lc-rs, untrusted |
| BSD-3-Clause | 7 | subtle, brotli, matchit, aws-lc-sys |
| Unlicense (with MIT) | 6 | memchr, aho-corasick, walkdir |
| MPL-2.0 | 5 | cssparser, selectors, dtoa-short, option-ext |
| Zlib | 5 | miniz_oxide, foldhash, raw-window-handle |
| MIT-0, 0BSD, BSL-1.0, CC0-1.0, Apache-2.0 WITH LLVM-exception | 1 or 2 each | aws-lc-sys, dunce, adler2, ryu, target-lexicon |

npm (`apps/companion/ui`): shipped in the app are `@mediapipe/tasks-vision` (Apache-2.0),
`@tauri-apps/api` (Apache-2.0 or MIT) and `preact` (MIT). Build tools only (not shipped):
`vite`, `rollup`, `esbuild`, `postcss` and their helpers (MIT, ISC, BSD-3-Clause),
`typescript` (Apache-2.0) and `@tauri-apps/cli` (Apache-2.0 or MIT).

Other bundled material (not seen by the tools above):

| Component | Licence | Where |
| --- | --- | --- |
| Microsoft Edge WebView2 Runtime, Fixed Version | Microsoft software licence terms for the WebView2 Runtime (redistribution with an app is the Fixed Version distribution mode) | Fetched and signature-checked at build time by `scripts/windows/Get-WebView2FixedRuntime.ps1`; inside the companion installer |
| MediaPipe Pose Landmarker lite model | Apache-2.0 | `apps/companion/ui/public/mediapipe/` with its `LICENSE` (details and the open model-card item in `docs/V2_Camera.md`) |
| BIP-39 English word list | MIT (bitcoin/bips) | `crates/hy-core/data/bip39-english.txt`, hash checked by a test (`protocol::words`) |
| NSIS installer stub and plugins (companion and server installers) | zlib/libpng (NSIS); Tauri's NSIS plugins MIT or Apache-2.0 | Downloaded by the Tauri CLI at build time |

## Exceptions in force

| Item | Exception | Reason | Review when |
| --- | --- | --- | --- |
| RUSTSEC-2024-0370 | `proc-macro-error` 1.0.4 is unmaintained (no vulnerability) | A build-time proc macro of `glib-macros` and `gtk3-macros`, the GTK stack Tauri uses on Linux only. The companion ships on Windows, where it is not in the graph | Tauri moves to GTK 4 bindings, or a vulnerability is filed |

## Dependency updates

Versions are pinned in `Cargo.lock` and `package-lock.json`. A new version is taken only
once it is at least 14 days old (V1-5 section 15), with one exception: a fix for a
security advisory reported by these checks is taken as soon as it is released. W3's first
run found seven advisories and they were fixed that way: `rustls` 0.23.45 to 0.23.46
(RUSTSEC-2026-0346 to -0349), `rustls-webpki` 0.103.15 to 0.103.16 (RUSTSEC-2026-0382),
`webpki-root-certs` 1.0.9 to 1.0.10 (RUSTSEC-2026-0383) and `tokio` 1.54.0 (yanked) to
1.54.1 (RUSTSEC-2026-0384, a use-after-free in `Interval`).

A new advisory against a pinned version fails the check on the next run, even with no
code change: fix it (update, or remove the dependency) or, only when it cannot affect the
product, add it to `deny.toml` and the table above with its reason.
