# Wardoff reference roadmap

Created: 2026-10-23. Source baseline: `39aadd2`. Planning artifact only.

## Description

Turn the [product vision](../../PRODUCT_VISION.md) into small, verifiable changes
that improve work protection on Windows. [PLAN.md](../../../PLAN.md) remains the
scope entrypoint; this folder owns detailed sequencing, acceptance and evidence.
The [comparison](../../COMPARISON.md) records the research behind the priorities.

## Goals and non-goals

Deliver understandable protection, independent bounded sessions, safe command
automation, and freely obtainable builds backed by reproducible evidence.
Do not pursue universal shutdown vetoes, IFEO, a driver, a background service,
cross-platform support, accounts, telemetry or paid infrastructure in this roadmap.
Do not change Windows power plans or lock policies to manufacture a passing test.

## Operating rules

- Work one selected slice at a time. Read its dependencies and acceptance first.
- New slices are `pending`; none has implementation acceptance evidence yet.
- Evidence identifies source SHA, artifact hash where relevant, commands,
  environment and skips. Never turn a VM result into a physical-device claim.
- Split an L item into reviewable implementation steps before coding. Resolve
  protocol and lifecycle details in that slice's design before edits begin.
- Reuse valid checks. Never add a paid resource or a billable fallback.
- Publication, outreach and changes to third-party tool configuration need their
  applicable authorization; this planning task performs none of them.
- Reevaluate after each milestone or at most four completed slices. Record new
  evidence in [reevaluation-log.md](reevaluation-log.md).

## Current truth

The local baseline has tray/CLI, layered shutdown protection, Power Requests,
JSONL logs, IPC and autostart. It also has bounded log tails and local validation
scripts. It does not have the future sessions, timers, doctor, panel or wrapper.
The IPC request-bound fix exists on a separate branch/PR and must be reconciled
with the actual integration target, rather than assumed merged.

The [idle-power record](../../IDLE_POWER_VALIDATION.md) documents eight VM cases
on an older exact candidate. Physical host S3/display, Hibernate and Modern
Standby remain unvalidated there. The dated
[resource evidence](../../benchmarks/2026-09-29-v0.2.0/README.md) covers one
machine/binary and does not establish performance leadership.

## Target state and milestones

| Milestone | User-visible outcome | Gate |
| --- | --- | --- |
| M1: know what is protected | One tested baseline, understandable read-only explanation, separate system/display intent | 001-003 accepted; failures and unavailable capabilities visible |
| M2: protect a task safely | Overlapping timed sessions, usable controls, cancellation and battery/resume behavior | 004, 005 and 007 accepted; lifecycle tests and available Windows cases recorded |
| M3: easy to adopt and automate | Tested free portable candidate, command workflow and a locally reviewable comparison report | 006, 008 and 009 accepted; external publication is a separately authorized follow-up |
| M4: extend only on demand | A justified optional integration with an explicit local contract | 010 accepted only if its demand and privacy gate pass |

M1/M2/M3 are acceptance milestones, not promised release dates or version numbers.
An unsigned source-first preview can remain narrower than the full roadmap.

## Priority and execution order

S = one small reviewable unit; M = several units; L = split before implementation.
These are relative estimates, not a delivery schedule. Priority weighs avoided
failure, recurring user value, evidence confidence, dependency and maintenance
cost. No adoption or revenue estimate is assumed.

| Order | Slice | Priority / size | Why now | Depends on |
| --- | --- | --- | --- | --- |
| 1 | [001 Tested baseline](foundations.md#rm-reference-001-tested-integration-and-truth-baseline) | P0 / M | New features need a trustworthy base | none |
| 2 | [002 Explain and diagnose](foundations.md#rm-reference-002-read-only-explanation-and-diagnostics) | P0 / M | Removes false confidence and makes support actionable | 001 |
| 3 | [003 Separate power intent](foundations.md#rm-reference-003-system-and-display-intent) | P1 / M | Ordinary background work should not require an illuminated display | 002 |
| 4 | [004 Owned timed sessions](work-sessions.md#rm-reference-004-owned-sessions-and-expiration) | P1 / L | Core contract for bounded, overlapping work | 003 |
| 5 | [005 Human controls](work-sessions.md#rm-reference-005-accessible-tray-and-session-panel) | P1 / L | Makes the new capability useful without a terminal | 004 |
| 6 | [007 Energy and recovery](work-sessions.md#rm-reference-007-battery-lock-and-resume-behavior) | P1 / M | A forgotten request must not become a battery or resume surprise | 004, 005 |
| 7 | [008 Free adoption](adoption.md#rm-reference-008-free-portable-distribution) | P1 / M | Makes a validated candidate obtainable without a certificate budget | 007 |
| 8 | [006 Protect a command](work-sessions.md#rm-reference-006-command-scoped-protection) | P1 / L | Builds automation on the same proven ownership contract | 004, 007 |
| 9 | [009 Compare and learn](adoption.md#rm-reference-009-reproducible-comparison-and-usability) | P1 / M | Determines which superiority claims the evidence supports | 006, 008 |
| 10 | [010 Optional integrations](adoption.md#rm-reference-010-demand-led-local-integrations) | P2 / M | Avoids maintaining speculative adapters | 009 and demand gate |

## Current slice and progress

- Active: none. Started: none. Implementation evidence: none for these new slices.
- Recommended next: `RM-reference-001` only, after verifying the current Git/PR state.
- Stop condition: its recorded integration/evidence gate, or a material unresolved
  conflict, permission boundary or need for a paid resource.

| Section | Status | Progress | Evidence |
| --- | --- | --- | --- |
| [Foundations](foundations.md) | pending | 0/3 | Existing sources inform planning, not acceptance |
| [Work sessions](work-sessions.md) | pending | 0/4 | none |
| [Adoption and evidence](adoption.md) | pending | 0/3 | none |

## Deferred choices

Defer CPU/network inference, transcript watching, broad settings, remote control,
Event Log integration and automatic power-off actions until observed needs
justify them. IFEO and kernel interception are excluded, not prerequisites for
1.0. Paid signing and commercial offers are outside this zero-budget program.

If the audience tests or matched comparisons reject the proposed advantage,
revise the next milestone rather than add features to defend the original plan.
