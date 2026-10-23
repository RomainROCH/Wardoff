# Competitive research

Reviewed: 2026-10-23. Wardoff baseline: `39aadd2`.
Purpose: inform the [product vision](PRODUCT_VISION.md) and
[reference roadmap](planning/reference/index.md), not declare a winner.
This replaces the older 0.1.0-era comparison.

## Method and confidence

Use publisher documentation and official project repositories for capability
claims. "Documented" means the publisher describes a feature; it does not mean
we installed or independently tested that version. No competitor binaries or
live power scenarios were exercised in this research. Current performance,
reliability and accessibility therefore remain unmeasured.

Absence from a page is not proof a feature is absent. No telemetry/security
audit of competitors was performed. Do not infer poor privacy from freeware,
or better performance merely from Wardoff's implementation language.

## The actual field

| Product and primary source | Documented strength | Implication for Wardoff |
| --- | --- | --- |
| [Microsoft PowerToys Awake](https://learn.microsoft.com/en-us/windows/powertoys/awake) | Interval/end-time modes, optional display-on, standalone executable and CLI process/parent attachment | Timers, CLI and PID attachment are parity work. Compare the standalone tool fairly; do not charge it the whole PowerToys suite's overhead |
| [Don't Sleep](https://www.softwareok.com/?seite=Freeware/DontSleep) | Portable freeware with timers, tray power actions and CPU/network-based rules; publisher also documents shutdown/logoff protection | A formidable functional baseline. Wardoff cannot win simply by adding timers or more toggles |
| [Caffeine, Zhorn Software](https://www.zhornsoftware.co.uk/caffeine/) | Timed operation, window/time/AC/CPU triggers; default simulated input and optional Windows API modes | Task/context automation already exists. Wardoff should retain its no-synthetic-input policy without falsely claiming Caffeine has no API alternative |
| [PreventTurnOff](https://www.softwareok.com/?seite=Microsoft/PreventTurnOff) | Portable timer and system/display prevention; publisher positions it as a simpler Don't Sleep derivative | Simplicity is already a competing value. A large settings panel is not automatically progress |
| [Insomnia, Stanley Projects](https://github.com/stanley-projects/Insomnia) | MIT Windows app using Electron powerSaveBlocker, manual/app triggers and AI-tool integrations; installer and package-manager paths | "Open source", "for developers" and "AI-aware" are not unique. Native implementation and content-minimal integration are hypotheses to measure, not automatic wins |
| [ShutdownGuard](https://github.com/stefansundin/shutdownguard) | Official repository explicitly marks the project archived and unsupported | Historical shutdown-blocking reference, not a current maintained-product benchmark |
| ShutdownBlocker, Cresstone | Publisher page could not be retrieved during this review | Current capabilities, release/support status and safety are unverified. Exclude from scored comparisons until its primary source can be checked |

The Don't Sleep page displayed 10.22 dated June 2026; PreventTurnOff displayed
3.63 dated May 2026; Caffeine displayed 1.98 dated November 2024. These are the
publisher-page snapshots reviewed, not a certification of the latest binaries.
Avoid the old assumption that every alternative is obsolete or narrowly manual.

Insomnia's [publisher site](https://stanley-projects.github.io/Insomnia/) also
describes automatic app discovery and activity-based protection. That directly
weakens an AI-only positioning for Wardoff. We have not verified its integrations
or measured its runtime footprint.

## What Wardoff can currently bring

The local source combines shutdown-layer handling, idle system/display requests,
tray control, read-only status/log CLI, rotating JSONL events and autostart. See
[PLAN.md](../PLAN.md) for exact limits. Those ingredients support the proposed
workflows but do not establish a comparative advantage on their own.

Today's gaps relative to documented alternatives include timers, user-selectable
display intent, app/command lifetime workflows, richer controls and easy adoption.
The [historical resource benchmark](benchmarks/2026-09-29-v0.2.0/README.md) is
useful methodology, not a comparison of all these current products.

## Opportunities worth testing

These are strategic inferences from the reviewed field, not assertions that no
competitor has ever implemented them.

| Opportunity | Why it matters | Proof required | Roadmap |
| --- | --- | --- | --- |
| Explain intent, activation, limitations and evidence separately | A green-looking state can hide unsupported conditions or a failed layer | Inactive/degraded/normal cases understandable through UI and read-only CLI | 002, 005 |
| Independently owned sessions with a clear end | One task ending must not cancel another; forgotten timers should release safely | Overlap, expiry, cancel-all, reconnect and failure tests | 004, 006 |
| Background work with display optional and battery policy explicit | Less unintended screen-on time without guessing that low CPU means completion | System/display isolation and documented AC/DC/resume results | 003, 007 |
| A human workflow and automation that expose the same truth | Reduces setup friction and makes support reproducible | Five common tasks, normal terminal behavior and consistent state | 005, 006, 009 |
| Public evidence for cleanup and resource cost | Trust and performance claims become inspectable | Exact versions, raw data, matched settings, repeats, limitations | 001, 008, 009 |

## Windows is part of the comparison

Do not market a forced-shutdown, explicit Sleep/Hibernate or closed-lid veto as
the differentiator. Microsoft documents Power Request termination on explicit
sleep and a timeout for system/execution requests on Modern Standby on DC power.
These constrain the product regardless of its feature list.
[Microsoft PowerSetRequest reference](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powersetrequest).

Lock-screen behavior also needs scenario-specific testing. Search snippets and
live documentation can differ; use the current primary page and record the
tested OS/settings. No claim that Wardoff works better while locked follows from
this desk research.

## How to earn a stronger claim

Use the protocol and acceptance in
[RM-reference-009](planning/reference/adoption.md#rm-reference-009-reproducible-comparison-and-usability).
Separate capability coverage, task completion, resource use and failure recovery.
Mark unsupported scenarios not applicable; do not give arbitrary zeros to an
unverified product. Credit competitors' better results. Recheck sources before
external messaging or whenever a priority depends on a changed feature.

There is no evidence yet that Wardoff surpasses all of them. The roadmap turns
that ambition into a limited set of user outcomes whose results can be checked.
