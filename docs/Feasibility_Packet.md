# HealthierYou Delivery Plan and Feasibility Packet

> Public repository copy of the HealthierYou delivery plan and feasibility packet, revision 2 (18 Oct 2026). Personal billing details are kept out of this copy; budget figures are project allocations only.


Oct 18, 2026 · @Rohit Lodha

**Revision 2, 18 Oct, after Codex's feasibility gate review.** What changed and what is needed from you:

- **Needed now:** approval of feasibility batch 1 (pet performance, offline install, webcam resources) at a $20 cap, plus a documentation-only coordination test at about $1. The request is in section 12.
- **Needed by 20 Oct:** the device checklist (section 6) and a decision on the Apple route (section 3). Nothing is bought until you decide.
- **Verdict:** full V1, V2 and V3 scope with all three wearable pairs acceptance-ready by 1 Nov is possible on paper but not yet supported by evidence. Four named blockers and three options are in section 2. No scope has been removed and the pilot stays 2 to 4 Nov.
- **Budget:** $175 planned plus $25 reserve inside your $200 ceiling. The build-phase estimates exceed their caps at the upper end, so spend is re-planned at the 22 Oct feasibility review (section 10).
- **Separate costs, not requested:** Apple Developer Program USD 99 a year if you choose route A; an optional code-signing certificate later.
- **Gates:** feasibility review, then architecture, then wireframes and prototype, then your prototype approval, then product code. No phase overlaps the gate before it.

## R. Response to Codex's review

All eight corrections from Codex's feasibility gate review (18 Oct) are accepted and applied; none was disputed. Revision 1 is kept in the doc's history and as a Markdown copy beside this one.

| # | Codex correction | Change in this revision | Where |
| --- | --- | --- | --- |
| 1 | Restore the gates | Feasibility review now ends before architecture starts, architecture review before wireframes, your prototype approval before any product code. No phase overlaps the gate before it | Sections 1 and 2 |
| 2 | Reconcile the pilot | Every release item must be acceptance-ready by 1 Nov for the full 2 to 4 Nov pilot. The verdict names the blockers and options; no watch is deferred and the pilot is not shortened | Section 2 |
| 3 | Prove offline installation | The embedded bootstrapper is dropped because it needs internet. Batch 1 tests a bundled fixed WebView2 runtime, installed and run with the network disconnected, on your 4 GB PC | Section 4 |
| 4 | Separate resource tests from functional proof | Memory and CPU are summed across the companion and all its WebView2 child processes and cross-checked in Task Manager. The webcam test is labelled resource-only. Meeting detection has a fixed trial count with false-positive cases. LAN discovery moves to acceptance with the real client | Sections 4 and 5 |
| 5 | Correct wearable tests | Exact models and OS first; same source and time window; freshness, duplicates and provenance recorded; a phone-stationary walk is the physical evidence, not an app name. Apple build, signing, install and testing are assessed separately. Samsung developer mode is diagnostic only | Sections 3, 5 and 6 |
| 6 | Prove coordination once | One documentation-only pull request round trip is proposed; no automatic wake-up is claimed until it passes | Section 7 |
| 7 | Honest hosting assumptions | Supabase's default email only reaches project-team addresses at 2 messages an hour and is not for production. The pilot avoids email: you issue invite codes, devices pair by on-screen code, recovery is a fresh invite. Custom email is a later option | Sections 8 and 9 |
| 8 | Document the execution boundary | Everything that ran in this container is listed, including the compiler install and hello-world build that went beyond read-only research. Nothing further has run | Section A |

## A. Environment, credits and execution record

### Credits

The owner has confirmed the credit source. The project ceiling is $200 (section 10). Credits expire on go-live day, 5 Nov, so any later fixes run on regular plan usage.

### What ran in this container on 18 Oct (observed by Claude, not on any Windows PC)

| Step | What ran | Result | Within read-only scope? |
| --- | --- | --- | --- |
| 1 | Repository attached with push access; shallow clone of main | Clone at aad7d5e, PRD 0.1, 15 docs | Yes |
| 2 | GitHub API read of repository permissions and workflow-run count | push and admin true; 0 runs | Yes |
| 3 | Push of branch claude/access-check pointing at the existing commit aad7d5e | Branch created, no new commit; two delete attempts refused by the session proxy | Write test approved in the brief; branch still present |
| 4 | apt-get install mingw-w64, rustup target add, cargo new hello\_win, cross-compile | 1.17 MB Windows .exe produced, never run, scratch folder deleted | No. A toolchain check, not product code, but beyond read-only research |
| 5 | HTTPS reachability checks to 15 toolchain hosts | Android SDK, WebView2 and .NET hosts denied by network policy | Yes |
| 6 | Files written: shared-folder notes, packet Markdown copies, project memory | No repository writes besides step 3 | Yes |

No spike, build or experiment has run since. Results above are this session's own observations; nothing about Windows behaviour has been observed yet.

### Capability summary

| Capability | Status |
| --- | --- |
| Read files and repository | Verified |
| Push branches | Verified; deleting branches is blocked here |
| Build Linux and cross-compiled Windows binaries | Verified (step 4) |
| Windows installers with a bundled WebView2 runtime | Route proposed: GitHub's free Windows runner on a spike branch, which avoids any network change here. Untested |
| Android builds | Same GitHub runner route (Linux runner with Android SDK). Untested |
| iOS builds | GitHub's macOS runner; signing and install route in section 3 |
| Automatic Codex and Claude coordination | Untested; section 7 |

## 1. Gated delivery plan

The sequence now runs strictly in your order: each phase starts only after the gate before it closes, and every gate needs evidence plus the named approver. Dates are estimates in Asia/Kolkata time, not commitments, and a late gate moves everything after it.

| # | Phase | Earliest window | Gate that must close first | Owner | Needed from you | Exit evidence |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Approved PRD | Done 18 Oct | None | You | Nothing | PRD 0.3 approval recorded |
| 2 | Detailed plan (this packet) | 18 Oct | PRD | Claude; Codex reviews | Approve batch 1 and the coordination test | Your approval message |
| 3a | Feasibility batch 1: pet, offline install, webcam resources | 19 to 20 Oct | Batch 1 approval | Claude builds; you test | About 2 hours on the slowest PC | Logs, Task Manager screenshots, pass or fail per criterion |
| 3b | Feasibility batch 2: three wearable routes, meeting detection | 20 to 22 Oct | Device checklist returned; batch 2 approval | Claude builds; you test | About 3 hours with phones and watches; Apple route decision | Same, per device |
| 4 | Feasibility review | 22 Oct | Batches 1 and 2 reported | Codex reviews; you accept | Read one evidence summary | Your acceptance of the feasibility result |
| 5 | Architecture | 22 to 23 Oct | Feasibility review | Claude | Nothing | Architecture note, contracts, Codex review closed |
| 6 | Wireframes and clickable prototype | 23 to 25 Oct | Architecture review | Claude | Choose one of two pet visual directions | PRD flows 1 to 12, addendum flows, traceability table |
| 7 | Your prototype approval | 26 Oct | Codex design review | You | About 2 hours to click through | Rubric at least 85/100 and your explicit approval in Approvals.md |
| 8 | Build in slices | 26 to 31 Oct | Your prototype approval | Claude coordinator plus at most 2 workers | Installs on office PCs and phones as slices land | One evidence packet per slice |
| 9 | Claude testing | With each slice | Slice built | Claude | Nothing | Automated tests and Windows builds green |
| 10 | Independent testing and security | 29 Oct to 1 Nov | Slice evidence packet | Codex agents | Nothing | Findings fixed and re-run |
| 11 | UAT | 30 Oct to 1 Nov | Independent test of that slice | You | Slowest PC, webcam desk sessions, all three watch pairs | UAT checklist signed |
| 12 | Pilot | 2 to 4 Nov | All pilot scope acceptance-ready on 1 Nov | You and 3 pilot users | Three working days | Pilot log, no unresolved critical defect |
| 13 | Go live | 5 Nov | Pilot review | Your decision | Go or no-go | Release manifest, rollback tested |
| 14 | Post-launch review | 12 Nov | Go live | Codex and Claude | One week of use | Review of reminder burden, reliability, engagement |

### Build slices (start only after your prototype approval)

| Slice | Content | Requirement IDs | Depends on | Credit cap | Your time |
| --- | --- | --- | --- | --- | --- |
| V1-1 | Shared contracts: event schema, reminder rules, local SQLite store, time-zone rules | RHY-02, RHY-03, PRD section 8 | Architecture | Within V1 $50 | None |
| V1-2 | Desktop shell, pet overlay, ten-pet asset pipeline and editing | PET-01 to PET-04 | V1-1, batch 1 result | Within V1 | 30 min |
| V1-3 | Activity tracking, combined reminders, focus, Pomodoro, meeting suppression | RHY-01 to RHY-04, FOC-01 | V1-1, V1-2 | Within V1 | 30 min |
| V1-4 | Water, six-day movement content, recovery day, phone self-report | WAT-01, WAT-02, MOV-01, MOV-02, REC-01, DIS-01 | V1-1, V1-2 | Within V1 | 30 min content check |
| V1-5 | Weekly and monthly review | REV-01, REV-02 | V1-3, V1-4 | Within V1 | 15 min |
| V1-6 | LAN server, discovery with the real client, pairing, sync, sharing controls, admin status | LAN-01, SHR-01, SHR-02, flow 12 | V1-1 | Within V1 | 1 hour, two PCs |
| V1-7 | Offline installer, update, uninstall, backup and restore, rollback | PRD section 9 | V1-2, V1-6 | Within V1 | 30 min |
| V2-1 | Camera calibration, check-in, confidence, conflict, immediate off | CAM-01, CAM-02 | V1-3, batch 1 result | Within V2 $15 | 30 min |
| V2-2 | Labelled desk test harness and raw-count report | V2 quality gate | V2-1 | Within V2 | About 6 hours across 5 setups |
| V2-3 | Simple meditation pause | CAL-01 | V1-3 | Within V2 | 10 min |
| V3-1 | Hosted service: invite-code accounts, device pairing, revoke, isolation | CON-02 | V1-1, hosting answer | Within V3 $35 | 30 min account setup |
| V3-2 | Avatar sync, one-group membership, bounded rankings | COM-01, COM-02 | V3-1 | Within V3 | 20 min |
| V3-3 | Android bridge on Health Connect (Fossil and Galaxy Fit pairs) | CON-01 | V3-1, batch 2 result | Within V3 | 1 hour, two phones |
| V3-4 | iPhone bridge on HealthKit (Apple pair) | CON-01 | V3-1, Apple route (section 3) | Within V3 | 1 hour |
| V3-5 | Per-device acceptance and wrist reminder tests | CON-01 | V3-3, V3-4 | Within V3 | 2 hours, three pairs |
| X-1 | Fixes from independent testing and UAT; release packaging | All | All slices | $15 | Re-test time |

One Claude coordinator owns integration; at most two workers run at once on separate files, after the V1-1 contracts merge. Each slice lands as a pull request with requirement IDs, test evidence and limits, and is reviewed on GitHub.

## 2. Pilot readiness by 1 Nov

**Verdict: not yet supported by evidence.** The plan fits every gate before 1 Nov only if each gate closes on its earliest date, all three wearable routes pass feasibility, and the Apple membership is active by 22 Oct. None of those is proven today. This is reported now so the decision is yours at the 22 Oct feasibility review, not discovered during the build.

### Gated schedule (earliest dates, Asia/Kolkata)

| Dates | Phase | Gate that closes it |
| --- | --- | --- |
| 18 to 19 Oct | Plan review, batch 1 approval | Your approval |
| 19 to 20 Oct | Batch 1: pet, offline install, webcam resources | Results reported |
| 20 Oct | Device checklist and Apple route decision | Your answers |
| 20 to 22 Oct | Batch 2: three wearable routes, meeting detection | Results reported |
| 22 Oct | Feasibility review | Codex review and your acceptance |
| 22 to 23 Oct | Architecture | Codex architecture review |
| 23 to 25 Oct | Wireframes and clickable prototype | Codex design review |
| 26 Oct | Prototype review | Your explicit approval |
| 26 to 31 Oct | Build in slices (V1, V2, V3) | Slice evidence packets |
| 29 Oct to 1 Nov | Independent testing and UAT | UAT checklist signed |
| 1 Nov | Pilot readiness | All release scope acceptance-ready |
| 2 to 4 Nov | Three-day pilot, three people | Pilot log |
| 5 Nov | Go live | Your go or no-go |

### Readiness by version

| Version | Can it be acceptance-ready by 1 Nov? | What it depends on |
| --- | --- | --- |
| V1: pet, rhythm, focus, water, movement, review, LAN | Likely, if batch 1 passes | Batch 1 results by 20 Oct; prototype approval on 26 Oct; two PCs for LAN acceptance |
| V2: camera posture, meditation pause | Possible | Batch 1 webcam result; about 6 hours of labelled desk sessions with colleagues on 30 Oct to 1 Nov |
| V3: hosted accounts, community, three wearable pairs | Not yet supported | Blockers B1, B2 and B4; your hosting answer by 22 Oct; acceptance on all three pairs by 1 Nov |

### Blockers

| ID | Blocker | Latest date it must clear | Effect if it does not |
| --- | --- | --- | --- |
| B1 | Apple route: no Mac and membership unconfirmed; Indian purchases have failed for some users since May 2026 | Membership active by 22 Oct for M1; signed TestFlight build by 30 Oct for slice V3-4 | The Apple pair cannot be verified before the pilot |
| B2 | Fossil Gen 5 runs Wear OS 2; no confirmed path for its steps into Health Connect | M2 result by 22 Oct | The Fossil pair needs an alternative you choose (own Wear OS app, or a labelled phone-origin route) |
| B3 | Build window of six days (26 to 31 Oct) for 16 slices; V1 to V3 estimates of $105 to $205 against $100 of caps | Measured batch 1 spend on 20 Oct | Either the build overruns 1 Nov or the reserve is needed, both your decision |
| B4 | Galaxy Fit steps may not be separable from A25 phone steps | M3 result by 22 Oct | The Fit pair needs a Samsung partner request or a labelled export route, your decision |

### Options for your decision at the 22 Oct review

| Option | What it means | Trade-off |
| --- | --- | --- |
| O1. Keep scope and dates; decide on evidence (recommended) | Run batches 1 and 2 now; at the 22 Oct review you see which blockers cleared and choose for each remaining one | No cost now; the decision moves to the review, with four days less room |
| O2. Keep scope; move the pilot and go-live | Any pair or slice not ready on 1 Nov shifts the pilot and go-live by the same amount | A deadline change; credits expire 5 Nov at 1:29 PM, so later work runs on plan usage |
| O3. Keep scope and dates; accept a labelled fallback for a failed pair | A pair that fails feasibility ships with its fallback route, clearly labelled, and the native route moves to after go-live | A quality change to that integration, needing your explicit approval per pair |

No option is applied without your word. Nothing has been removed from V1, V2 or V3, and the pilot remains three full days.

## 3. Apple route without a Mac

The workable route without a Mac is: build and sign on GitHub's free macOS runner, deliver through TestFlight to you as an internal tester, and test on your own iPhone 13 and Watch. It needs the Apple Developer Program at USD 99 a year (about ₹8,700 per a June 2026 forum post; Apple shows the exact price at purchase). The biggest schedule risk is paying for it from India: three Apple forum threads from May to July 2026 report purchases failing with "not available in your country or region", with no Apple answer. Nothing is being bought; this is the information you asked for before committing.

### Four stages, assessed separately

| Stage | How | Verified by docs | Still unverified |
| --- | --- | --- | --- |
| Build | GitHub Actions macos-latest (macOS 26 on Arm). Default Xcode 26.6 with iOS 26 SDKs, which meets Apple's upload rule (Xcode 26 and iOS 26 SDK since 28 Apr 2026). Free on a public repository | Runner image readme; Apple upcoming requirements | First build of our project |
| Sign | App Store Connect API key used by xcodebuild automatic signing (or fastlane match). You create the key and paste it straight into GitHub repository secrets; it never appears in chat | Apple API key docs; fastlane docs | Apple reviews API access requests "case by case"; a forum thread reports export errors on first CI signing |
| Install | TestFlight app on your iPhone. Internal testers (up to 100, App Store Connect users) do not go through TestFlight App Review, which Apple's glossary applies to external testers | TestFlight overview and glossary | That internal builds skip review is inferred, not stated outright; HealthKit inside TestFlight builds is expected to work but not confirmed |
| Physical test | Run the M1 method from section 5 on your iPhone 13 and Watch Series 8; feedback screenshots and crash reports come back through App Store Connect; the app exports its own log through the share sheet | TestFlight feedback docs | Background step delivery is best-effort and hourly at most; it must be observed over a real day |

The Watch needs no app of its own: its steps reach the iPhone's Health store, and the iPhone app's reminders appear on the Watch when the iPhone is locked and the Watch is on your wrist. A watch app would add on-watch screens and complications, and is not proposed.

### Routes compared

| Route | Cost | Lead time | Main blocker | Verdict |
| --- | --- | --- | --- | --- |
| A. GitHub runner, Developer Program, TestFlight internal testing | USD 99 a year; runner $0 | 1 to 3 days after membership is active | Purchase from India; API access approval | Recommended |
| B. Same membership, ad hoc install of a signed file | USD 99 a year | Same | Needs your iPhone's device ID; no documented Windows method to read it | Fallback if TestFlight processing stalls |
| C. Free Apple ID re-signed on Windows with Sideloadly or AltStore (AltStore is open source, AGPLv3) | $0 | Same day | Unknown whether these tools keep the HealthKit permission; re-sign every 7 days; your Apple ID password goes into third-party software, so a separate Apple ID would be used | Fallback only, and only with your explicit approval |
| D. Rented cloud Mac (AWS 24-hour minimum from about $18.70; Scaleway from about €2.64; MacinCloud prepaid about $25 to $30) | As listed | Hours | Cannot connect your iPhone to a remote Mac; still needs the membership for TestFlight | Only to untangle signing in a graphical Xcode, if A gets stuck |
| E. Xcode Cloud | Included with membership | After membership | Setup starts inside Xcode on a Mac | Not usable without a Mac |

### What you would do yourself, if you choose route A

1. Confirm two-factor authentication on your Apple ID and check whether a membership already exists (section 6 checklist).
2. Enrol as an Individual in the Apple Developer app on your iPhone, paying with your own card or Apple account balance. If it fails, contact Apple Developer Support with the Enrollment ID.
3. In App Store Connect: accept the agreements, request API access, create one API key, and paste the key, Key ID and Issuer ID into the repository's GitHub secrets.
4. Accept the TestFlight invitation on your iPhone and allow the Health permission when the app asks.

To keep M1 inside the feasibility window, enrolment would need to be active by 22 Oct. If it is not, the feasibility review closes with the Apple pair marked unverified and the decision comes back to you (section 2).

Sources: [Apple upcoming requirements](https://developer.apple.com/news/upcoming-requirements/), [macOS 26 runner image](https://github.com/actions/runner-images/blob/main/images/macos/macos-26-arm64-Readme.md), [compare memberships](https://developer.apple.com/support/compare-memberships/), [enrolment support](https://developer.apple.com/support/enrollment/), [TestFlight overview](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview), [internal testers](https://developer.apple.com/help/app-store-connect/test-a-beta-version/add-internal-testers), [TestFlight App Review](https://developer.apple.com/help/glossary/testflight-app-review/), [tester feedback](https://developer.apple.com/help/app-store-connect/test-a-beta-version/view-tester-feedback), [App Store Connect API](https://developer.apple.com/help/app-store-connect/get-started/app-store-connect-api), [supported capabilities](https://developer.apple.com/help/account/reference/supported-capabilities-ios), [background delivery entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.healthkit.background-delivery), [fastlane match](https://docs.fastlane.tools/actions/match/), [AltStore](https://github.com/altstoreio/AltStore), [Sideloadly](https://sideloadly.io/), [AWS EC2 Mac](https://aws.amazon.com/ec2/instance-types/mac/), [Scaleway Apple silicon](https://www.scaleway.com/en/pricing/apple-silicon/), [MacinCloud](https://www.macincloud.com/pages/payg.html), India purchase failures: [thread 835350](https://developer.apple.com/forums/thread/835350), [thread 837697](https://developer.apple.com/forums/thread/837697).

## 4. First feasibility batch, capped at $20

Batch 1 answers three questions on your slowest PC: can the pet run within the memory, CPU and focus limits, can it install and run with no internet, and what does a webcam check-in cost in CPU and memory. Estimated $11 to $19; work stops at $20 whether or not it is finished, and the report states actual spend from your before-and-after balance. It is resource and installation evidence only; it does not prove posture accuracy, LAN sync or any product feature.

### What gets built (disposable)

- One throwaway Windows app with two buttons: a 128-pixel animated pet (12 frames a second, one click action) and a "Start camera test" mode running MediaPipe pose lite at 3 frames a second. No product screens, settings, storage or server.
- It writes a measurement log every 5 seconds: memory and CPU summed across the app and every WebView2 child process, frame time, and camera start and stop times.
- Packaged as a per-user installer with the fixed WebView2 runtime bundled (about +180 MB), so nothing is downloaded at install or run time. The pose model is bundled too.
- Built on GitHub's free Windows runner from a branch named spike/batch1 in the public repository, because this container cannot download the WebView2 runtime. You download the zip from that run's page on GitHub while signed in. The branch is never merged; you can delete it afterwards.

### Pass and fail criteria

| Test | Measure | Pass | Fail leads to |
| --- | --- | --- | --- |
| P1 Idle memory | Summed private working set of the app and WebView2 children, minutes 10 to 30, with Excel and a browser open | At most 150 MB average | Retest with the system WebView2; then the Rust-native fallback (about +$10 to $20, 2 days) |
| P2 Idle CPU | Average share of total machine CPU over 30 quiet minutes | At most 2%, and no one-minute average above 5% | Lower frame rate, pause animation when idle; then fallback |
| P3 No focus stealing | Click the pet 10 times while typing in Notepad | Caret stays in Notepad 10 of 10 times | Win32 window flags fix, or Rust-native window |
| P4 Response | Time from click to the pet's reaction (logged) | At most 500 ms, 10 of 10 | Same as P2 |
| P5 Launch | Double-click to pet visible | At most 10 s | Startup trimming |
| P6 Scaling | Windows display scaling 100%, 150%, 200% | Pet sharp, correctly sized, still clickable | Rendering fix |
| I1 Offline install | Wi-Fi off and cable out, run the installer as a standard user | Completes without an administrator prompt, within 2 minutes | Change runtime packaging |
| I2 Offline run | App opens with the network still disconnected | Log shows the bundled runtime path and version | Same as I1 |
| I3 Uninstall | Uninstall from Windows Settings | Program folder removed; log folder left only if you chose to keep it | Installer fix |
| I4 Warnings | Run the installer once downloaded from GitHub and once copied from a USB stick or LAN share | Warning screens recorded both ways (no pass threshold; informs the signing decision) | None |
| C1 Webcam CPU | Average total CPU during 10 minutes of camera test | At most 10% | 1 frame a second check-ins, results reported to you |
| C2 Webcam memory | Peak summed memory during camera test | At most 350 MB | Smaller model or lower resolution, reported to you |
| C3 Camera release | Press Stop; time until the camera light goes off | At most 2 s, 5 of 5 times | Release-handling fix; a release blocker if unfixable |
| C4 Camera unavailable | Turn camera access off in Windows privacy settings, start camera test | App shows "camera unavailable" and the pet keeps working | Error-handling fix |

A miss on any line is reported with its numbers; changing a target needs your approval, as the PRD requires.

### Your testing instructions (about 2 hours)

Use the slowest 4 GB PC. If it has no webcam, do C1 to C4 on the PC whose webcam would be used and note which PC.

1. Before starting, note the cloud-credit balance shown in claude.ai (for the actual-spend record).
2. Write down the PC's processor name and Windows edition: Settings > System > About.
3. Download the zip from the GitHub link I send and copy it to the desktop. Turn Wi-Fi off and unplug the network cable.
4. Double-click the installer. Note whether any blue administrator prompt appears and how long it takes (I1). Note any warning screen (I4).
5. Open Excel with a normal workbook and a browser with two tabs. Start the test app (P5: time it). Leave everything idle for 30 minutes; do not close anything.
6. At minute 10, open Task Manager > Details, sort by Name, and take a screenshot showing the test app and its msedgewebview2 entries (cross-check for P1).
7. After 30 minutes, open Notepad and type a sentence. While typing, click the pet. Repeat 10 times and count how often the typing cursor stayed in Notepad (P3).
8. Change display scaling to 150% and then 200% (Settings > Display), check the pet each time, then return to your normal setting (P6).
9. Press "Start camera test" and sit normally for 10 minutes (C1, C2). Press Stop and count seconds until the camera light goes off. Repeat Start and Stop 5 times (C3).
10. Turn off camera access in Settings > Privacy > Camera, press Start camera test, and note what appears (C4). Turn access back on.
11. Reconnect the network. Copy the installer from a USB stick or shared folder to another location, run it again and note any warning (I4).
12. Uninstall from Settings > Apps (I3).
13. Zip the folder %LOCALAPPDATA%\\HY-Spike\\logs, attach it in this thread with the two screenshots, and note the credit balance again.

I read the logs and post one result table; you do not need to interpret numbers.

## 5. Later spikes (batch 2, not yet requested)

Batch 2 covers the three wearable routes and meeting detection, estimated $9 to $18 with a proposed $15 cap; it will be requested separately after your device checklist comes back, because the checklist may answer some questions at no cost. LAN discovery is no longer a spike: it is demonstrated with the real client in slice V1-6, with typed address as fallback.

### Common wearable test method (applies to M1, M2, M3)

1. Record exact watch and phone models and OS versions first (section 6); nothing is assumed from a product name.
2. Note the watch's own step count, leave the phone on the desk, walk about 200 steps, note the watch count again, then sync. A stationary phone is supporting evidence of watch origin, not proof: phones can still log steps, and an app's name in a source list proves nothing.
3. Compare records for the same 15-minute window only: watch display, phone health store, and our throwaway reader's output.
4. Record freshness (minutes from walk end to the record appearing), duplicates (raw records summed versus the platform's de-duplicated total, and any overlap with phone-origin records), and provenance (source app, device type, manufacturer and model as written by the source app, labelled "reported by" because Health Connect does not validate it).
5. Pass (revised 18 Oct on the owner's instruction, replacing the earlier 2% and 5% display comparisons): the watch-origin source records exist for the walk's time window and match the watch's own record for that window when compared record by record from the same source; freshness is measured; duplicates are explained by the platform's de-duplication rules; and provenance (source app, device type, manufacturer and model as reported) is recorded and labelled "reported by". Fail: no new watch-source records in the window, only phone-origin records, or unexplained duplicates.

| ID | Question | Experiment | Needs | Est. credits | Pass leads to | Fail leads to |
| --- | --- | --- | --- | --- | --- | --- |
| M1 | Can our own iPhone build read Apple Watch step samples attributed to the watch? | One-screen reader app listing step samples by source device, installed by the route chosen in section 3 | iPhone 13, Watch Series 8, the section 3 route | $2 to $4 | Native SwiftUI bridge | Evidence and the labelled Health-export import alternative, for your decision |
| M2 | Do Fossil watch steps reach Health Connect on the S24 FE? | Section 6 no-code check first; if inconclusive, a throwaway reader app built on GitHub's runner and installed over USB | S24 FE, Fossil watch, USB cable, Developer options enabled | $2 to $4 | Health Connect bridge | Two alternatives for your decision: our own Wear OS 2 step app installed over ADB (more work, battery risk), or a clearly labelled phone-origin route |
| M3 | Can Galaxy Fit steps be separated from A25 phone steps? | Same; Samsung Data SDK developer mode only as a diagnostic, never as release permission | A25, Galaxy Fit, USB cable | $2 to $4 | Health Connect bridge | Samsung partner request (free, outcome uncertain) or labelled Samsung Health export import, for your decision |
| D2 | Can the companion see that a camera or microphone is in use? | Console reader of the Windows in-use registry state; 20 trials: join and leave Teams, Zoom and a browser meeting (12), plus false-positive cases: Camera app, voice recorder, a webcam test page, our own camera test (8) | One Win10 and one Win11 PC | $1 to $2 | Optional hint labelled "camera or microphone in use", never "in a meeting" | Manual meeting toggle plus presentation detection only |

Device-side installs of throwaway Android apps use USB debugging, which is exempt from Google's developer verification. Nothing is published to an app store.

## 6. Device checklist

One pass through this checklist, about 30 minutes across the three pairs, identifies every device and shows whether watch data already reaches each phone. It needs no installs and no codes. Paste the answers back in any form; photos of the screens are fine.

### Pair 2: Fossil watch with Galaxy S24 FE

- [ ] Watch: Settings > System > About. Record Model, and under Versions the Wear OS version, system version and security patch date.
- [ ] Watch: is Google Fit installed on the watch? Is any other fitness app (Fossil's own) installed?
- [ ] Phone: Settings > About phone. Record Model name and Model number; under Software information record One UI version and Android version.
- [ ] Phone: which app pairs the watch (Wear OS, Fossil Smartwatches, or another)? Does it say Connected right now?
- [ ] Phone: open Google Fit. Is it signed in, and does it show today's steps?
- [ ] Phone: Settings > search "Health Connect" > App permissions. Is Google Fit listed, and is it allowed to write Steps?
- [ ] Phone: Health Connect > Data and access > Activity > Steps. Which apps are listed as sources?
- [ ] 200-step check: note the watch's step count, leave the phone on the desk, walk about 200 steps, note the watch count again, then open Google Fit. Did Fit's count rise by roughly the same amount? Within 15 minutes, does Health Connect's Steps total rise too?

### Pair 3: Samsung Galaxy Fit with Galaxy A25

- [ ] Phone: Galaxy Wearable app > About band. Record Model name, Model number (SM-R390 means Fit3) and Software version.
- [ ] Phone: Settings > About phone. Record Model name, Model number, One UI version and Android version.
- [ ] Phone: Samsung Health > Settings > About Samsung Health. Record the version.
- [ ] Phone: Samsung Health > Settings > Health Connect (or Settings > search "Health Connect" > App permissions). Is Samsung Health allowed to write Steps?
- [ ] Phone: Health Connect > Data and access > Activity > Steps. Which apps are listed as sources?
- [ ] 200-step check: note the band's count, leave the phone on the desk, walk about 200 steps, note the band count again, open Samsung Health and pull down to sync. Did its count rise by roughly the same amount, and does Health Connect's Steps total rise within 15 minutes?

### Pair 1: Apple Watch Series 8 with iPhone 13

- [ ] iPhone: Settings > General > About. Record Model Name, Model Number and iOS Version.
- [ ] iPhone: Watch app > General > About. Record Model, watchOS version.
- [ ] iPhone: Health > Browse > Activity > Steps > Data Sources & Access. Is the Apple Watch listed under data sources?
- [ ] 200-step check: leave the iPhone on the desk, walk about 200 steps wearing the watch, then open Health > Steps > Show All Data. Do new entries appear with the Apple Watch as source?
- [ ] Apple Developer membership: sign in at developer.apple.com/account with your Apple ID and note whether it shows an active membership. Looking does not buy anything.
- [ ] Is two-factor authentication on for that Apple ID (Settings > your name > Sign-In & Security)?

### Office machines (for the test plan)

- [ ] Slowest PC: processor name and Windows edition and version (Settings > System > About); does it have a webcam?
- [ ] Server PC: processor, installed RAM, Windows edition; can you sign in to it as an administrator once?
- [ ] Are the office PCs on one network (same router or Wi-Fi), and does any PC use guest or isolated Wi-Fi?

## 7. Repository record and one coordination test

GitHub stays the shared record, and automatic Codex-to-Claude communication is not claimed until one documentation-only round trip passes. Claude's automatic fixing of its own pull requests is a different feature and proves nothing about Codex.

### What goes in the repository once you approve

- docs/Feasibility\_Packet.md (this packet, revision 2), with the revision 1 file kept beside it.
- docs/STATUS.md, Decisions.md, Budget.md and Approvals.md updated with today's decisions, the credit observation and the pending gates.
- docs/Coordination\_Test.md recording the test below.
- Routine decisions go into Decisions.md through pull requests from then on; only blockers, decisions and gates come to you here.

### Coordination test (documentation only, about $1)

1. Claude opens one pull request containing only the documents above, and subscribes this session to that pull request's activity.
2. Codex reviews it on GitHub and leaves one review comment containing the phrase HY-COORD-TEST-1 and one small requested wording change.
3. Pass condition, Claude side: without any message from you, this session wakes on that review, makes the change, pushes it and replies on the pull request quoting HY-COORD-TEST-1 and the new commit. Timestamps go in Coordination\_Test.md.
4. Pass condition, Codex side: Codex confirms it saw the reply and commit. How Codex is triggered to look is Codex's own mechanism; Codex should state it in its review.
5. Fail: if Claude does not wake (for example because the cloud session has ended), the fallback is one line from you in this thread, "Codex review ready on the PR", at gates only. Nothing else depends on the test.

The only thing you would do is ask Codex once to review that pull request, unless Codex already watches the repository.

## 8. Feasibility matrix, updated

"Docs" means vendor documentation or code was read; "observed" means this session saw it run. Nothing in this table has been observed on your hardware yet.

| Area | Status | Evidence type | What proves it | When |
| --- | --- | --- | --- | --- |
| Credits | Confirmed by the owner; project ceiling $200 | Owner confirmation | Spend recorded at each gate | Every gate |
| Desktop pet on a 4 GB PC | Unproven | Docs | Batch 1 P1 to P6 | 20 Oct |
| Offline installation | Unproven; embedded bootstrapper dropped because it needs internet; bundled fixed runtime chosen | Docs (Tauri installer options) | Batch 1 I1 to I4, network disconnected | 20 Oct |
| Webcam resource use | Unproven; resource test only, not posture accuracy | Docs | Batch 1 C1 to C4 | 20 Oct |
| Posture accuracy | Unproven | None yet | V2 labelled desk test | 30 Oct to 1 Nov |
| Idle, lock and presentation signals | Documented Windows APIs | Docs | Slice V1-3 acceptance | Build |
| Meeting detection | Undocumented registry state | Docs and community | D2, 20 trials with false-positive cases | Batch 2 |
| LAN discovery | Moved to acceptance with the real client; typed address fallback | Docs | Slice V1-6 on two PCs | Build |
| Hosted accounts | Feasible without email: invite codes and on-screen pairing | Docs (Supabase limits) | Slice V3-1 | Build |
| Apple Watch pair | Hosted route found; needs the USD 99 membership | Docs | M1 on your iPhone 13 and Watch | Batch 2 |
| Fossil pair | High risk: Wear OS 2 | Secondary sources | Checklist, then M2 | Batch 2 |
| Galaxy Fit pair | Medium risk: attribution unknown | Docs | Checklist, then M3 | Batch 2 |
| Windows builds without network changes | Route proposed via GitHub runner | Docs | First batch 1 build | 19 Oct |
| Agent coordination | Unproven | None | HY-COORD-TEST-1 | 19 Oct |

## 9. Risk and dependency register

The three risks most likely to stop pilot readiness on 1 Nov are the Apple install route without a Mac, the Fossil Gen 5 data path, and the six-day build window between your prototype approval and the pilot.

### Dependencies

| Input | Needed by | From | Blocks |
| --- | --- | --- | --- |
| Approval of batch 1 and the coordination test | 18 to 19 Oct | You | Everything after the plan |
| Device checklist (section 6) | 20 Oct | You | Batch 2, Apple route choice |
| Apple route decision (section 3) | 20 Oct (PRD checkpoint) | You | M1, slice V3-4 |
| Batch 2 approval | 20 Oct | You | Wearable evidence |
| Office machine facts | 20 Oct (inside the checklist) | You | Test plan, server design |
| Feasibility acceptance | 22 Oct | You, after Codex review | Architecture |
| Hosting yes or no | 22 Oct | You | Slice V3-1 |
| Pet visual direction | 24 Oct | You | Prototype |
| Prototype approval | 26 Oct | You | All product code |
| Five webcam desk setups, about 6 hours of labelled sessions | 30 Oct to 1 Nov | You and colleagues | V2 acceptance |
| Three pilot users for 2 to 4 Nov | 1 Nov | You | Pilot |

### Risks

| ID | Risk | Likelihood | Impact | Mitigation or option | Owner | Decide by |
| --- | --- | --- | --- | --- | --- | --- |
| R1 | Any gate slips; the build window is only 26 to 31 Oct | High | High | Batched questions; evidence summaries ready before each gate; a slip is reported the same day with its effect on 1 Nov | You and Claude | Each gate |
| R2 | Fossil Gen 5 on Wear OS 2 gives no watch-origin data path | High | High | Device checklist, then M2; alternatives for your decision | Claude evidence, you decide | 22 Oct |
| R3 | No working Apple build-sign-install route in time | High | High | Section 3 routes; decision by 20 Oct | You | 20 Oct |
| R4 | Galaxy Fit steps not distinguishable from phone steps | Medium | High | M3; Samsung partner request or labelled import, your decision | Claude evidence, you decide | 22 Oct |
| R5 | Cloud credits expire 5 Nov at 1:29 PM, on go-live day | Certain | Medium | Release work planned to finish 4 Nov; later fixes use plan usage | Claude | 4 Nov |
| R6 | Build-phase caps below the low estimate | Medium | Medium | Batch 1 measures real spend; replan at feasibility review | Claude, you approve | 22 Oct |
| R7 | Memory above 150 MB on the 4 GB PC | Medium | High | Batch 1 P1; Rust-native fallback, about +$10 to $20 and 2 days | Claude | 20 Oct |
| R8 | Pet window takes focus or click-through fails on Win10 | Medium | High | Batch 1 P3; Win32 flags or Rust-native | Claude | 20 Oct |
| R9 | Fixed WebView2 runtime gets no automatic security updates | Certain if chosen | Medium | Ship runtime updates through our own updater; record the version in each release | Claude | Architecture |
| R10 | Unsigned installers warn or are blocked by policy | Medium | Medium | Batch 1 I4 records it; signing is a later purchase decision (Azure signing unavailable in India) | You | Before go-live |
| R11 | Server firewall needs an administrator once | High | Low | Admin installs the server; companions stay per-user | You | Install day |
| R12 | Webcam above 10% CPU on old processors | Medium | Medium | Batch 1 C1; lower duty cycle; target changes need your approval | Claude | 20 Oct |
| R13 | V2 labelled desk test needs about 6 hours of colleagues' time inside 30 Oct to 1 Nov | High | Medium | Book the sessions now; the harness records counts | You | 26 Oct |
| R14 | Hosted email limited to team addresses, 2 an hour | Certain | Low | Invite codes and on-screen pairing; no email in pilot | Claude | Architecture |
| R15 | Supply-chain attack, as shown by the open-posture lookalike | Low | High | Verified upstreams, pinned versions, licence register, Codex dependency review | Claude and Codex | Every slice |
| R16 | Spike code and artifacts are public on the repository | Certain | Low | No secrets or employee data in spikes; spike branches never merged | Claude | Batch 1 |
| R17 | Codex wake-up route unproven; session may end between events | Medium | Medium | Coordination test; fallback one line from you at gates | Claude and Codex | Before build |
| R18 | Health Connect device labels are not validated | Certain | Medium | Phone-stationary evidence; data labelled "reported by" | Claude | Batch 2 |
| R19 | GitHub Windows runners are Server 2025, not Windows 10 | Certain | Medium | Runners build only; all acceptance on your PCs | Claude | Build |
| R20 | Pairing crypto libraries are unaudited | Medium | Medium | TLS with a pinned server certificate after code pairing; Codex security review | Claude and Codex | Architecture |

## 10. Budget: $175 planned plus $25 reserve

> Superseded on 18 Oct 2026: the owner decided planning is recorded as its own line, not inside batch 1. Current allocations, estimated spend and the unchanged $200 ceiling are in docs/Budget.md. Session cost meters are estimates, not balance readings.

The project ceiling is $200: $175 of planned caps and a $25 reserve that is touched only with your decision. The caps are envelopes, not forecasts; actual spend is recorded only from the balance you see.

| Phase | Estimate | Cap | Checkpoint |
| --- | --- | --- | --- |
| Planning and this revision | About $21.91, estimated from session meters | Separate line (superseded the earlier "included in batch 1") | See docs/Budget.md |
| Feasibility batch 1 (pet, offline install, webcam) | $11 to $19 | $20 | Stop at $20; report actual spend |
| Feasibility batch 2 (three wearable routes, meeting detection) | $9 to $18 | $15 | Requested separately |
| Architecture | $5 to $10 | $8 | Codex review |
| Wireframes and clickable prototype | $12 to $25 | $17 | Your approval |
| V1 build and developer tests | $54 to $108 | $50 | Replan with you if batch 1 calibration shows V1 will exceed $50 |
| V2 build, desk-test harness, meditation | $16 to $30 | $15 | Same |
| V3 hosted service, community, two phone bridges | $35 to $67 | $35 | Same |
| Fixes after independent testing and UAT; release packaging | $19 to $43 | $15 | Critical and high findings first |
| Total planned | $161 to $320 | $175 | Your protocol checkpoints: $100, $150, $175 |
| Reserve |  | $25 | Your decision only |

The caps for the build phases sit at or below the low end of the revision 1 estimates. Batch 1 gives the first measured figure: if it shows the build will not fit $175, you get a replan at the feasibility review, not a quiet draw on the reserve.

### Separate costs outside the $200 (none requested now)

| Item | When it would be needed | Cost | Status |
| --- | --- | --- | --- |
| Apple route costs | V3 iPhone bridge | See section 3 | For your decision after batch 2 |
| Windows code-signing certificate | Only to remove installer warnings | About $150 to $300 a year (OV) | Not needed for the pilot |
| Custom email for hosted accounts | Only if email sign-in is wanted later | Free tiers exist at some providers; unverified | Pilot uses invite codes, no email |
| Hosting | V3 | $0 on Supabase Free, Mumbai region | Needs your yes (section 12) |
| GitHub Actions | Spikes and builds | $0 on a public repository | No action needed |

## 11. Sources and inspected components

Four specialist research passes (wearables and Apple; desktop, LAN and webcam; hosted, licensing and billing; hosted macOS route) ran on separate topics, and this coordinator reconciled them. Agent suggestions to drop a version or device were rewritten as options for your decision.

### Security finding

The top search result for open-posture, github.com/valuable-familygerreidae648/open-posture, is a malicious lookalike: a different account added a zip containing Application.cmd and utility.exe on 26 Jul 2026 and redirected its download link on 14 Oct 2026. It was listed, never extracted or run. Only the genuine [whatnotbot/open-posture](https://github.com/whatnotbot/open-posture) (Apache-2.0) may be used, and every dependency is pinned to a verified upstream.

### Inspected components

| Component | Licence | Latest commit seen | What was inspected | Use |
| --- | --- | --- | --- | --- |
| [tauri-apps/tauri](https://github.com/tauri-apps/tauri) | MIT or Apache-2.0 | 15 Oct 2026 | set\_focusable, set\_ignore\_cursor\_events, transparent config | Candidate desktop shell |
| [tauri-apps/tao](https://github.com/tauri-apps/tao) | MIT or Apache-2.0 | 14 Oct 2026 | focusable false sets WS\_EX\_NOACTIVATE; click-through flags | Window behaviour |
| [tauri-apps/wry](https://github.com/tauri-apps/wry) | MIT or Apache-2.0 | 12 Oct 2026 | SetMemoryUsageTargetLevel call; MoveFocus on focus | Memory tuning |
| [hovancik/stretchly](https://github.com/hovancik/stretchly) | BSD-2-Clause | 15 Oct 2026 | Natural-break, do-not-disturb, suspend and exclusion managers | Design reference only |
| [whatnotbot/open-posture](https://github.com/whatnotbot/open-posture) | Apache-2.0 | 19 Jul 2026 | MediaPipe worker, adaptive 5 to 1 fps, median and MAD calibration, dwell and cooldown | V2 design reference |
| PCPosture, code2k13/posturedetection | No licence | Not recorded | Licence files absent | Not reusable |
| @mediapipe/tasks-vision 1.1.1 | Apache-2.0 | npm | 42 MB package; pose lite model 5.8 MB | V2 inference |
| spake2, snow | MIT or Apache-2.0 | docs.rs, GitHub | Both state they are unaudited | Pairing candidates; security review |

### Primary documentation

- Windows: [Tauri Windows installer (WebView2 modes)](https://v2.tauri.app/distribute/windows-installer/), [WebView2 performance](https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance), [WebView2 distribution](https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution), [tauri#11461](https://github.com/tauri-apps/tauri/issues/11461), [GetLastInputInfo](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo), [WTSRegisterSessionNotification](https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtsregistersessionnotification), [DnsServiceBrowse](https://learn.microsoft.com/en-us/windows/win32/api/windns/nf-windns-dnsservicebrowse), [Firewall rules](https://learn.microsoft.com/windows/security/operating-system-security/network-security/windows-firewall/rules), [Artifact Signing FAQ](https://learn.microsoft.com/azure/artifact-signing/faq), [code-signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options).
- Webcam: [MediaPipe Pose Landmarker](https://developers.google.com/edge/mediapipe/solutions/vision/pose_landmarker), [Windows camera privacy](https://support.microsoft.com/windows/privacy/windows-camera-microphone-and-privacy).
- Apple: [enableBackgroundDelivery](<https://developer.apple.com/documentation/healthkit/hkhealthstore/enablebackgrounddelivery(for:frequency:withcompletion:)>), [HKSourceRevision.productType](https://developer.apple.com/documentation/healthkit/hksourcerevision/producttype), [supported capabilities](https://developer.apple.com/help/account/reference/supported-capabilities-ios/), [compare memberships](https://developer.apple.com/support/compare-memberships), [enrolment](https://developer.apple.com/support/enrollment/), [Xcode requirements](https://developer.apple.com/xcode/system-requirements), [upcoming requirements](https://developer.apple.com/news/upcoming-requirements/), [Apple Watch notifications](https://support.apple.com/en-us/108369); section 3 lists the macOS-route sources.
- Android, Fossil, Samsung: [Health Connect read data](https://developer.android.com/health-and-fitness/health-connect/read-data), [metadata](https://developer.android.com/health-and-fitness/health-connect/metadata), [Fit migration FAQ](https://developer.android.com/health-and-fitness/health-connect/migration/fit/faq), [Health Services compatibility](https://developer.android.com/health-and-fitness/health-services/compatibility), [developer verification FAQ](https://developer.android.com/developer-verification/guides/faq), [Samsung Health and Health Connect](https://developer.samsung.com/health/blog/en/accessing-samsung-health-data-through-health-connect), [Samsung Data SDK process](https://developer.samsung.com/health/data/process.html), [developer mode](https://developer.samsung.com/health/data/guide/developer-mode.html), [Fit3 specifications](https://www.samsung.com/ie/watches/galaxy-fit/galaxy-fit3-grey-bluetooth-sm-r390nzaaeub). Secondary only: [Engadget on Fossil's exit](https://www.engadget.com/fossil-is-done-making-smartwatches-but-will-keep-releasing-updates-for-a-few-years-161958128.html), [Android Police on Wear OS 3 eligibility](https://www.androidpolice.com/heres-every-smartwatch-guaranteed-to-get-a-wear-os-3-update/).
- Hosted, licensing, CI: [Supabase SMTP limits](https://supabase.com/docs/guides/auth/auth-smtp), [Supabase pricing](https://supabase.com/pricing), [Supabase regions](https://supabase.com/docs/guides/platform/regions), [Postgres partial indexes](https://www.postgresql.org/docs/current/indexes-partial.html), [NHS terms](https://www.nhs.uk/our-policies/terms-and-conditions/), [HSE copyright](https://www.hse.gov.uk/help/copyright.htm), [WHO copyright](https://www.who.int/about/policies/publishing/copyright), [Kenney licence](https://kenney.nl/support), [Lottie licence](https://lottiefiles.com/page/license), [GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions), [GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), [Claude Code on the web](https://code.claude.com/docs/en/claude-code-on-the-web).

## 12. Questions, routine decisions and the approval request

### Questions for you

> Historical: Q1 and the approval request below were answered on 18 Oct 2026; batch 1 and the coordination test are approved (docs/Approvals.md).

| # | Question | Answer format | Needed by |
| --- | --- | --- | --- |
| Q1 | Approve batch 1 and the coordination test as below? | Yes or no | 19 Oct |
| Q2 | Return the device checklist (section 6) | One reply with the filled list | 20 Oct |
| Q3 | Apple route: A (membership plus TestFlight, recommended), B, C, or decide later | One letter. No purchase is requested by this question | 20 Oct |
| Q4 | Use a hosted service (Supabase, Mumbai region, free tier) for V3 accounts? | Yes or no | 22 Oct |

### Routine decisions Claude has taken (recorded, reversible)

| Decision | Reason |
| --- | --- |
| Tauri 2 desktop shell, with a Rust-native fallback if batch 1 fails | Small footprint; fallback costs about $10 to $20 and 2 days |
| Rust office server as a Windows service with SQLite | One binary, no database administration |
| Per-user installer with the bundled fixed WebView2 runtime | No admin rights, works offline |
| Unsigned pilot builds | Signing is a later purchase decision |
| Pairing by on-screen code or QR | No email, no passwords in pilot |
| Manual meeting toggle always present | Detection is only a hint |
| MediaPipe pose lite inside WebView2 | No extra runtime; licence Apache-2.0 |
| Health Connect first for both Android pairs | One bridge for two pairs |
| SwiftUI iPhone app, no watch app | Watch shows phone reminders; less work |
| Wearable data labelled "reported by" | Device labels are not validated |
| Original pets and content only | Avoids licence risk |
| Unfunded requirement DIS-02 kept in the backlog, not removed | Visible for your later decision |
| Repository docs updated by pull request after your approval | Repository stays the shared record |
| Leftover branch claude/access-check left for you or Codex to delete | Deletion is blocked from this session |

### Approval request

**Approve feasibility batch 1 at a $20 maximum, plus the coordination test at about $1?** Claude builds one throwaway test app on GitHub's free Windows runner from branch spike/batch1 and tests nothing itself on your PCs. You spend about 2 hours on the slowest 4 GB PC following section 4, with the network disconnected for the install. Work stops early if the cap is reached, and the report gives actual spend, before and after credit balances, and pass or fail per criterion. Architecture, wireframes and product code stay blocked.
