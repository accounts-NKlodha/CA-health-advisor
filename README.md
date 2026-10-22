# HealthierYou — planning workspace

Status: **V1-0 build slice in progress** (21 October 2026). The owner approved the prototype and the reviewer closed the prototype gate on 21 October 2026, authorising the agreed V1/V2 build in reviewed slices. The architecture (`docs/Architecture_V1_V2.md`, PR #3) and PRD 0.3 govern the build.

User target: V1, V2 and V3 by 5 November 2026. Codex plans/reviews; Claude develops; user approves PRD, prototype and go-live.

## Start here

- [Product requirements](docs/PRD.md)
- [Confirmed decisions and open inputs](docs/Scope_Decisions.md)
- [Claude design handoff](docs/Claude_Design_Brief.md)
- [Product discovery](docs/Discovery_Findings.md)
- [Wearable feasibility](docs/Wearable_Feasibility.md)
- [Delivery and coordination](docs/Delivery_and_Claude_Protocol.md)
- [Current status](docs/STATUS.md)

Documents are draft proposals where labelled. Neither a prototype nor a specification is proof of integration feasibility. No existing app, installer or wearable integration is claimed.

Shared repository supplied by the user: [accounts-NKlodha/CA-health-advisor](https://github.com/accounts-NKlodha/CA-health-advisor). Repository visibility is public. This initial publication contains planning documents only; private conversation exports and operational credentials are not published.

Source conversation export is retained as background. `sources/` and `AGENTS.md` are synced project references and must not be modified. Historical suggestions do not override the PRD or current user decisions.

## Code layout (slice V1-0)

| Path | What it is |
| --- | --- |
| `crates/hy-core` | Shared Rust core: schema, local SQLite store, migrations, clock and time zone; the activity tracker and coaching engine (`hy_core::coach`, V1-2) |
| `crates/hy-pet` | Pet shell logic (V1-1): placement and clamping in the work area, pet packs (`crates/hy-pet/packs`), pet edits, the `PetShell` interface for the coaching engine; `hy_pet::link` connects the engine to the shell (view and answer routing, the rules behind the Windows idle and presentation signals) |
| `crates/hy-server` | Office LAN server (skeleton; built in V1-5 after the pairing security gate) |
| `crates/hy-sim` | Load simulator (skeleton; built in V1-5) |
| `apps/companion/src-tauri` | Windows desktop companion, Tauri 2: pet window, tray, keyboard pet panel, settings window, and the coaching engine's thread fed by Windows idle, lock, sleep and presentation signals |
| `apps/companion/ui` | Companion pages: pet and keyboard panel (plain TypeScript), screens (Preact), built with Vite |

Local checks (Rust toolchain pinned in `rust-toolchain.toml`, Node 22):

```
cargo fmt --all --check
cargo clippy --workspace --exclude hy-companion --all-targets --locked -- -D warnings
cargo test --workspace --exclude hy-companion --locked
(cd apps/companion/ui && npm ci && npm run build)
```

The companion builds and is tested on the Windows CI job (`.github/workflows/ci.yml`).
