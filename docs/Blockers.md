# Blockers and decisions needed

18 October 2026. Continue independent discovery/design while access is resolved.

| ID | Affected work | Evidence / issue | Next action | Latest useful time |
|---|---|---|---|---|
| B01 resolved | Public planning publication | User explicitly requested upload after the public-disclosure question naming payload and public repository | Publish through the normal approved connector route; record commit in repository history | Authorised publication |
| B02 | Claude execution and coordination | User named Claude.ai project; official docs distinguish context sync from Code cloud execution/write | Confirm Claude Code cloud access and $200 credit eligibility; perform docs-only write check | Before prototype dispatch / paid work |
| B03 | Apple integration | Native bridge build/signing/test route not arranged | Claude identifies authorised macOS/Xcode/signing/test distribution route and requests any missing user access | Feasibility gate |
| B04 | Fossil and Samsung integrations | Exact models/OS and existing watch-origin sync unknown | Inspect actual models, app versions and sample source records on user's phones | Feasibility gate |
| B05 | Pilot environment | Three pilot people confirmed; server spec, total rollout count and tester schedule unknown | User supplies remaining inventory; PRD uses three real and ten simulated clients as test planning default | Before UAT scheduling |

No approval to purchase services or store memberships is implied. No automated Codex–Claude relay is active.

## Update 20 October 2026: batch 1 open items

| ID | Affected work | Issue | Next action | Latest useful time |
|---|---|---|---|---|
| B06 | Installer | Uninstall leaves a Windows uninstall registry entry (unresolved defect) | Fix in the product installer | Before UAT/pilot |
| B07 | Display support | Display scaling checked only at 100% and 125% | Test other scalings on available hardware | Before UAT/pilot |
| B08 | Distribution | Downloaded-installer SmartScreen / Mark-of-the-Web warning behaviour not tested | Test with a downloaded installer | Before UAT/pilot |
| B09 | Camera reliability | No multi-hour camera soak test (C2 headroom 301 of 350 MB over 10 min) | Run soak test | Before UAT/pilot; need not block provisional architecture. The pilot must not be the first reliability test |
| B10 | Camera compatibility | Only one working webcam proven; Windows 10 camera operation unproven | Second working webcam check, ideally on Windows 10 | Before UAT/pilot |

B04 (Fossil and Samsung integrations) and B03 (Apple integration) are deferred with V3 and no longer gate V1/V2 architecture.
