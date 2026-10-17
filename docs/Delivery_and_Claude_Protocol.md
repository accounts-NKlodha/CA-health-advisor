# HealthierYou delivery and Claude coordination protocol

Status: proposed delivery baseline for PRD review, 18 October 2026. Dates use Asia/Kolkata. Target: all three agreed versions released by 5 November 2026. This target is not yet a validated delivery forecast.

## Authority and development hold

- User: approves PRD, wireframes/clickable prototype, pilot acceptance and go-live; supplies real Windows and wearable testing.
- Codex: project planning, scope control, requirements, architecture review, independent acceptance and security review. Codex does not develop the product.
- Claude: design artifacts, architecture proposal, implementation, developer tests, packaging, deployment instructions and fixes. Its coordinator owns integration of its agents' work.
- Product implementation is BLOCKED until explicit user approval of the wireframes and clickable prototype. Approval of discovery is not build approval. A filename or an agent's claim does not constitute approval.
- Before approval, Claude may research, document, produce wireframes and create an explicitly simulated presentation prototype. No production backend, desktop companion, wearable application or deployment. Experimental executable spikes need a separate explicit authorization while this hold applies; otherwise use documentation/device capability evidence and mark runtime feasibility unverified.
- Preserve synced `sources/` material unchanged. Treat reference documents and old chat suggestions as background, not authority over current requirements.

## The Claude environment must be verified

The ordinary Claude.ai project GitHub integration imports selected repository files as context. Its documentation says sync covers file names/contents on a branch, not commit history or pull-request metadata, and offers a manual refresh. That connection alone is insufficient evidence that Claude can run builds or write changes. [Anthropic: Use the GitHub integration](https://support.claude.com/en/articles/10167454-use-the-github-integration)

Claude Code cloud sessions can clone repositories and push branches with an authorised GitHub connection, run work in the cloud, use subagents and create pull requests. Cloud sessions share account usage limits. Official documentation describes review-triggered auto-fix and cloud session follow-up mechanisms. Availability and permissions must be tested on the user's account rather than inferred from a normal Claude.ai project. [Anthropic: Claude Code in the cloud](https://code.claude.com/docs/en/claude-code-on-the-web)

Recommended arrangement: retain the Claude.ai project for reference/discussion if desired; use an authorised Claude Code cloud workspace for execution. The user has stated $200 of cloud-session credit: record its actual eligibility, expiry and remaining balance before dispatching paid work. Do not assume it is API credit, that it funds every surface, or that $200 guarantees completion.

Setup exit criteria, all documented with links or evidence:

1. The user-supplied public accounts-NKlodha/CA-health-advisor repository is the shared record. Codex connector access is verified; Claude access must be tested. Keep private transcripts, credentials and employee records out.
2. Claude can read the approved document revision and create a documentation-only branch/commit. This is the write-access smoke check before any build permission.
3. The execution surface and available credit are confirmed. No credentials belong in the repository or chat documents.
4. There is a verified way to wake Claude when a review is ready and a verified way for Codex to inspect results. Until tested, coordination is manual at gates; a shared repository does not automatically start an agent.
5. Record repository URL, default branch, Claude session link, current document revision and integration status in `docs/STATUS.md`. A project/session link alone does not grant access.

No external setup or cross-agent message has been performed merely by writing this protocol.

## Work allocation

Codex uses separate review agents for product/acceptance, wearable feasibility, and delivery/coordination. After build approval, independent testers review completed artifacts without being the authors of those artifacts. They report reproducible failures to Claude; they do not quietly fix product code.

Claude uses a coordinator and a small number of bounded workers. Suggested initial split is desktop/pet/LAN, webcam, and mobile/wearable/hosted integration. A separate test pass validates completed slices. These are responsibilities, not a commitment to run four expensive sessions continuously. Start with two concurrent implementation workers after interfaces are agreed; expand only for genuinely independent tasks and within the budget. Avoid duplicate research, overlapping file ownership and repeated full-history prompts.

Each work item states an owner, requirement IDs, dependencies, files/components owned, acceptance evidence, spending allowance and stop condition. Claude's coordinator integrates shared contracts before parallel work and resolves routine merge conflicts. A worker may not change scope or declare another worker's untested integration complete.

## Target calendar and evidence gates

| Dates in 2026 | Work | Exit evidence |
|---|---|---|
| Oct 18–19 | Discovery, PRD, backlog and environment checks | User-approved PRD revision; known assumptions and device uncertainties listed |
| Oct 19–20 | Feasibility assessment and architecture proposal | Separate result for each watch/phone pair; LAN, desktop overlay and low-memory approach; unknowns explicitly marked |
| Oct 20–22 | Claude wireframes and clickable prototype | All three versions, ten pets, settings/sharing, Monday–Saturday movement, weekly/monthly progress and failure states demonstrated |
| Oct 22 | Design review gate | Codex consolidated review resolved; explicit user approval linked to prototype revision; production development unlocked only then |
| Oct 23–28 | Build in slices and Claude testing | Runnable incremental artifacts, automated checks and version-specific evidence; independent reviews begin as slices finish |
| Oct 29–31 | Independent testing, security review, fixes and user UAT | Tested 4 GB Windows machine, LAN server/client, camera cases and all three real wearable pairs; no unresolved release blockers |
| Nov 1–3 | Office pilot | Three confirmed pilot users; actual usage and issues recorded, fixes rechecked, missing coverage acknowledged |
| Nov 4 | Release readiness | Install/upgrade/rollback/backup evidence, release notes and user go-live decision |
| Nov 5 | Go live and initial review | Accepted release deployed; initial operational checks and issues recorded |
| Nov 12, proposed | Longer post-launch review | A week's real use, reminder burden, reliability and habit-engagement trends |

These dates require prompt gate decisions and access to physical devices. A delayed gate does not authorise skipping testing. Escalate the effect on dates and scope; only the user can approve changing either. Early November 5 review is operational, not a claim of long-term health improvement.

### Scope must remain visible

V1: self-contained office server and lightweight Windows employee companion, LAN access, no camera/wearable dependency. Pet-led reminders and logging, exercise routine, settings, sharing choices and personal progress.

V2: V1 plus webcam functions approved in the PRD and supported by evidence; phone-distraction inference must remain labelled as uncertain and user-correctable.

V3: hosted use plus separately validated integrations for Apple Watch Series 8/iPhone 13, probable Fossil Gen 5/Galaxy S24 FE, and Samsung Galaxy Fit of unconfirmed model/Galaxy A25. All three are required targets. A prototype, mock data, manual entry or unsupported status screen is not a completed integration. If any route proves unavailable, escalate immediately rather than silently removing that device. Public app-store approval must be treated as an external dependency if the chosen route requires it.

## Repository as the shared record

The following paths are proposed coordination records; their listing does not mean they already exist:

| Record | Purpose / writer |
|---|---|
| `docs/PRD.md` | Versioned requirements and acceptance criteria; Codex owns baseline |
| `docs/Backlog.md` | Work IDs, owner, dependencies, state and evidence links; coordinator updates |
| `docs/STATUS.md` | One current summary: gate, revision, completed work, next work, risks and credit balance |
| `docs/Decisions.md` | Decision, reason, alternatives, approver and affected requirements |
| `docs/Review_Queue.md` | Batch of completed slices ready for Codex, exact commit and test evidence |
| `docs/Blockers.md` | Only actionable blockers with attempted resolution, requested decision and deadline |
| `docs/Approvals.md` | PRD, prototype and release approvals with user message/date and artifact revision |
| `docs/Budget.md` | Actual credit observations and phase allocations; never invented token/dollar estimates |
| `design/` | Wireframes, prototype exports and design decisions |
| `evidence/` | Sanitised test reports, screenshots and device acceptance records |
| `releases/` | Release manifest and instructions; large installers can be repository release assets |

Before starting a slice, read current approvals, decisions and backlog. At completion, publish one evidence packet containing requirement IDs, exact commit/build, change summary, test environment, actual results, known limitations and next action. Link existing evidence rather than repeat conversations.

Review at stable slice boundaries. Batch nonurgent findings into one review. Claude fixes them and reports the new commit with rerun evidence. A review is stale if relevant code changed after its recorded commit. Codex routine acceptance authority does not include overriding user design or go-live gates.

## Low-noise communication

Once a reliable connection is configured, use repository records and supported execution triggers. Verify any trigger with one documentation-only test. Enable no unbounded polling or paid retry loop. Ordinary record updates should not notify the user; escalate a gate decision, missing access/device, infeasible requirement, material budget/date risk or reproducible blocker.

Use one blocker message containing: affected requirement; evidence; what was tried; two feasible choices if available; recommendation; impact if unanswered. Continue independent authorised work. Do not exchange new prompts for every small fix. No automated connector/trigger is claimed active in this draft, and no ongoing monitoring starts merely because this document describes it.

If only the normal Claude.ai project is available, use a single durable brief and batched handoffs at milestones. State that this fallback needs human transfer; do not pretend the two systems can communicate autonomously. Authentication/setup may need the user once. Do not ask for passwords or tokens in chat.

## Proposed $200 cloud-credit allocation

These amounts are management envelopes, not model pricing, a cost forecast or evidence of funds spent. Actual account credit rules prevail.

| Phase | Proposed cap |
|---|---:|
| Claude feasibility, architecture, wireframes and prototype | $30 |
| V1 implementation and developer tests | $45 |
| V2 implementation and developer tests | $25 |
| V3 implementation and device integration | $45 |
| Claude fixes from independent testing/UAT/pilot, packaging | $30 |
| Unallocated contingency | $25 |
| Total | $200 |

Log the visible credit balance before/after meaningful batches. If exact usage is unavailable, mark it unknown and request a balance check rather than assigning fabricated costs. At $100 spent, review remaining work and burn. At $150, stop optional polish and reconcile remaining acceptance work. At $175, obtain an explicit contingency decision before starting more paid work. Never silently exceed $200 or shift to another paid service. Reallocation inside the total is a documented planning decision; additional expenditure requires user approval. Hosting, Apple signing/membership, app-store costs and any paid services are not assumed funded or authorised by these credits.

## Release decision

Every required version and target device has an acceptance result against the approved scope, with no substitution of simulated evidence. Required checks include actual pet overlay and responsiveness on the slow PC, user-controlled data sharing, reminders under focus/sleep/restart, LAN disconnect/reconnect, camera opt-out, wearable sync delays/denial, upgrade and recovery. No critical unresolved security or data-loss defect may be hidden as a limitation. The user approves pilot and go-live with the remaining nonblocking limitations visible.

If evidence cannot be obtained by the deadline, report the affected acceptance item as incomplete. Completion means accepted working versions, not just merged code or a passing cloud test suite.


