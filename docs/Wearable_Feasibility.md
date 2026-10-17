# HealthierYou V3 — wearable feasibility discovery

Status: desk research only; no device tests or implementation. Official primary documentation accessed 18 October 2026. Deadline requested: 5 November 2026. Product development remains blocked pending the user's prototype approval.

## Decision proposed for the PRD

V3 targets exactly three user-owned combinations: Apple Watch Series 8 + iPhone 13; likely Fossil Gen 5 + Samsung Galaxy S24 FE; Samsung Galaxy Fit (possibly Fit3, model unconfirmed) + Galaxy A25. Do not advertise universal wearable support or identical features across them. Each must pass a real-device end-to-end acceptance test. A manual import or mock demonstration must not be relabelled a completed wearable integration.

Prefer phone-mediated daily activity summaries to continuous raw watch sensing. A hosted website needs a mobile bridge to these on-device data stores; no direct browser-to-watch integration has been established by this research. Upload only categories selected by the user to the hosted account. Desktop/LAN V1 and webcam V2 must remain independent of this path.

## Apple Watch Series 8 + iPhone 13

**Verified platform capability:** HealthKit supports authorised health-data access in native apps. Background delivery needs the entitlement; step-count background notifications on iOS have an hourly maximum frequency. This is not a live streaming feed. [HealthKit setup](https://developer.apple.com/documentation/Xcode/configuring-healthkit-access), [background delivery](https://developer.apple.com/documentation/healthkit/hkhealthstore/enablebackgrounddelivery(for:frequency:withcompletion:)).

**Proposed route, still to test:** watch activity synchronises into the iPhone health store; a small iOS companion reads permitted steps/activity summaries and sends selected summaries to the hosted service. A dedicated watch app is optional unless direct watch interactions are needed. Verify actual installed iOS/watchOS versions, available source data, duplicate-source handling and sync timing on the user's pair.

**Notifications:** mirrored iPhone notifications can reach the Watch, but normal routing depends on phone lock state and watch settings; do not promise simultaneous phone and watch alerts. [Apple notification behaviour](https://support.apple.com/en-ie/108369).

**Delivery dependency:** a compatible macOS/Xcode build environment and Apple signing/distribution route must be arranged. Windows-only testing does not validate iOS/watchOS builds. [Xcode system requirements](https://developer.apple.com/xcode/system-requirements). App-store acceptance is not established or guaranteed by the target date.

## Fossil (likely Gen 5) + Galaxy S24 FE

**Verified generic routes:** Google's Wear OS help describes watch/phone Google Fit synchronisation. Fit's Android app documents Health Connect integration. Health Connect provides an on-device Android data store, accessible through authorised app integration. [Wear OS fitness help](https://support.google.com/wearos/answer/6056874?hl=en), [Fit Health Connect settings](https://support.google.com/fit/faq/6108483?hl=en-GB), [Health Connect availability](https://developer.android.com/health-and-fitness/health-connect/availability).

**Candidate route, not verified for this particular watch:** Fossil → existing watch/phone fitness sync → Health Connect → HealthierYou Android bridge → hosted account. Check the watch model and installed OS, functioning pairing, which app owns its data, and whether actual watch-origin records arrive in Health Connect. Phone-generated steps alone do not prove watch support.

**Important limitation:** modern Health Services requires Wear OS 3 or later. Do not assume it works on a Gen 5; record the installed OS before choosing APIs. If the existing sync path fails, assess an OS-compatible watch sensor app as a separate high-risk route, with battery and background-operation tests. [Health Services requirements](https://developer.android.com/health-and-fitness/health-services).

Avoid starting a new integration around legacy Google Fit cloud APIs: the current migration guide says support ends at the end of 2026. Existing Fit app synchronisation and choosing Fit APIs for our own product are different decisions. [Official migration guide](https://developer.android.com/health-and-fitness/health-connect/migration/fit).

**Notifications:** Wear OS supports bridged phone notifications; actual delivery/actions must be tested on this older device. [Wear OS notifications](https://developer.android.com/training/wearables/notifications).

## Samsung Galaxy Fit (model uncertain) + Galaxy A25

**Model gate:** obtain Settings/About model number before committing. Fit3 findings below do not establish support for a different model or a presumed Fit4.

**Verified Fit3 capabilities:** Samsung documents health/activity tracking and phone notification mirroring. Its product specifications list FreeRTOS; it must not be treated as a Wear OS Galaxy Watch. No supported third-party installable Fit3 app route was established in this research. [Fit3 specifications](https://www.samsung.com/latin_en/watches/galaxy-fit/galaxy-fit3-gray-bluetooth-v5-3-sm-r390nzaalta/), [features](https://www.samsung.com/us/support/answer/ANS10004550/), [notification controls](https://www.samsung.com/ae/support/mobile-devices/how-to-manage-notifications-on-the-galaxy-fit3/).

**Candidate route:** Fit → Samsung Health on A25 → Health Connect → Android bridge → hosted account. Samsung confirms Health Connect synchronisation of activity, heart rate and sleep, but its FAQ's watch-specific examples do not prove every Fit model/data-type combination. Verify Fit-origin records and their freshness on the actual A25. [Samsung Health Connect FAQ](https://developer.samsung.com/health/health-connect-faq.html).

**Alternative:** Samsung Health Data SDK can read stored data and identify source devices including Galaxy Fit. Production distribution requires Samsung partnership registration; developer mode is only for testing, not a release workaround. Prefer evaluating Health Connect first to avoid adding this approval dependency. [SDK FAQ](https://developer.samsung.com/health/data/faq.html), [distribution process](https://developer.samsung.com/health/data/process.html).

## Acceptance evidence required before declaring V3 complete

1. Record exact hardware, OS and bridge/fitness-app versions for all three pairs.
2. For each pair, demonstrate actual watch-origin daily steps in the hosted account; preserve source and last-sync timestamp. Never silently substitute phone-only values.
3. Add other activity/heart-rate data only where the tested data source exposes it. Show unavailable or stale data explicitly. Do not manufacture a common metric across incompatible devices.
4. Demonstrate permission denial, selective sharing, revocation, disconnect/reconnect, retry without duplicates, time-zone/day-boundary handling and deletion semantics.
5. Assess reminders on each wrist under documented settings; distinguish notification receipt from acknowledgement or task completion. This is an additional capability to test, not the PRD's minimum wearable-data acceptance contract unless explicitly added at the feasibility gate.
6. Measure latency and battery impact over normal use. Background access requires platform permissions and scheduling: [Health Connect reading](https://developer.android.com/health-and-fitness/health-connect/read-data).
7. Document install/distribution instructions and tests on the user's physical devices. Store submission is a separate state from approved public availability.

## Deadline risks and decision ownership

Fossil legacy compatibility, uncertain Samsung model, iOS build/signing access and public distribution approvals are unresolved. The three-device requirement is retained. If a device route fails, Claude must provide evidence and alternatives to the planner/user; it must not quietly reduce scope, claim a mock as a completed integration, or promise unsupported hardware behaviour. The user approves any change to the November 5 acceptance scope.
