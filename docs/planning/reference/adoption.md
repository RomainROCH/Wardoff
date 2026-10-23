# Adoption and evidence

Section goal: make validated work freely usable and learn which advantages hold.
The absence of signing funds is an execution constraint, not a reason to paywall
the core or make claims that an unsigned build cannot support.

## Items

### RM-reference-008 Free portable distribution

Status: pending. Depends on: RM-reference-007. Priority: P1. Size: M.

Goal: a new user can obtain, try and remove a tested build without building Rust.

Security gate: review the residual findings carried by `RM-reference-001`,
including the status pipe's default DACL, before broadening distribution beyond
the existing source-first/self-administered scope. A passing smoke test does not
close those findings.

Work: prepare a portable candidate with existing CI/release infrastructure,
version/build identification, SHA-256 checksums, a short quick start and removal
instructions. Explain unsigned/SmartScreen limits honestly. Review WinGet/Scoop
submission only after the portable workflow passes; package review is not
guaranteed. Follow [Microsoft's package submission process](https://learn.microsoft.com/en-us/windows/package-manager/package/).

Acceptance: in an isolated existing Windows environment, fresh extraction,
launch, diagnostic inspection, cancellation and removal pass against the exact
artifact. Autostart refusal from an untrusted path is explained, not bypassed by
weakening ACLs. Checksums prove integrity relative to a trusted reference, not
publisher identity. No self-signed trust-bypass instructions, forced installs,
paid certificate, hosted backend or new paid CI capacity. Publish only through
the authorized Git/release workflow; otherwise retain a reviewable local package.

Evidence: none. Notes: no automatic updater in this slice. Package distribution
does not depend on signed-binary sales.

### RM-reference-009 Reproducible comparison and usability

Status: pending. Depends on: RM-reference-006, RM-reference-008. Priority: P1. Size: M.

Goal: replace a superiority hypothesis with scoped, reviewable evidence.

Work: reuse the existing benchmark protocol and
[validation matrix](../../MVP_VALIDATION_MATRIX.md). Freeze tool versions,
settings, build hashes and OS/power contexts. Compare matched system-only,
display-on and timed-work scenarios with PowerToys Awake, Don't Sleep and
Insomnia (Stanley Projects), chosen because it explicitly targets active work
and app/AI triggers. Record each version, official download/source and artifact
hash before collecting data; a later comparator change requires a new protocol
revision and rerun rather than selecting a winner after seeing results.
Use standalone Awake rather than attributing
all PowerToys-suite overhead to it. Unsupported features stay not applicable.

Acceptance: produce a locally reviewable report with raw measurements, multiple runs,
order rotation, baseline subtraction where valid, CPU time, private memory,
startup/status latency and cleanup results. Record tradeoffs and uncertainty;
do not invent universal RAM or energy targets from one machine. Re-run the
five-task volunteer protocol and thresholds in
[PRODUCT_VISION.md](../../PRODUCT_VISION.md). Participation is voluntary and
unpaid, with consent before collecting diagnostics. If volunteers/hardware are
missing, mark those results untested and narrow claims. This gate concerns
leadership claims, not permission to retain a clearly labeled preview build.

Evidence: none. Notes: prepare outreach/report drafts locally. Sending messages,
posting comparisons or collecting third-party data needs its own authorization.

### RM-reference-010 Demand-led local integrations

Status: pending. Depends on: RM-reference-009 and demand gate. Priority: P2. Size: M.

Goal: support one repeated real workflow without collecting its private content.

Work: proceed only after at least three independent voluntary reports identify
the same limitation of timers/command sessions. Choose one adapter or local
event interface, using the existing session ownership model. Otherwise defer
this slice and record why; do not add integrations to fill a checklist.

Acceptance: explicit opt-in, documented local events, bounded owner lifetime,
uninstall/rollback and failure tests. Do not modify another tool's configuration
without authorization. No transcript crawling or command-content collection by
default, and no paid AI/API dependency. Compare against Insomnia's documented
integrations before claiming a new advantage. Success is demonstrated on the
reported workflow, not by the number of adapters shipped.

Evidence: none. Notes: Windows Event Log, CPU/network inference, broad schedules
and remote management are separate future decisions, not hidden sub-tasks here.

## Session log

- 2026-10-23: created pending zero-budget adoption and evidence slices. No release,
  third-party configuration change or outreach occurred.
