# Wardoff plan

This file is the repo-level status and roadmap summary for Wardoff.

For the product, start with [README.md](README.md). For development, use
[CONTRIBUTING.md](CONTRIBUTING.md) and the
[task-to-code map](docs/DEVELOPMENT.md#task-to-code-map). Agents start at
[AGENTS.md](AGENTS.md). This plan owns scope and priorities, not setup commands.

For monetization, sponsorship, signed-binary, paid-support, or broader commercialization decisions, follow `docs/BUSINESS_MODEL.md` rather than inferring policy from engineering docs.

## Product direction and detailed roadmap

The maintainer requested a competitive, zero-budget product roadmap on
2026-10-23. The [product vision](docs/PRODUCT_VISION.md) defines the target:
user control over supported shutdown, restart and idle-power paths. Simple
Block/Allow and indefinite manual protection remain central; optional task-bound
protection adds clear ownership and duration without replacing them.
The [competitive research](docs/COMPARISON.md) explains why timers,
process attachment and AI integrations alone are not sufficient differentiation.

Use [the reference roadmap](docs/planning/reference/index.md) for ordered slices,
dependencies, acceptance and evidence. The maintainer authorized starting with
`RM-reference-001`, a tested integration baseline, with the philosophy above
preserved. On 2026-10-24, the maintainer selected the remaining roadmap for
sequential execution: validate each phase, push its accepted work, then continue.
Keep the zero-budget constraint and all conditional acceptance gates.
The [baseline evidence](docs/planning/reference/baseline-2026-10-23.md) records
successful local checks, portable startup, elevated and non-elevated desktop
smoke, and tray checks. Slice 001 is accepted and backed up on the working branch.
Slice 002 has passed its [diagnostic acceptance](docs/planning/reference/diagnostics-plan.md#acceptance-2026-10-24)
and verified phase backup. Slice 003 passed its
[power-intent acceptance](docs/planning/reference/power-intent-plan.md#acceptance-2026-10-24).
Its phase backup was verified at `88e6345`. Slice 004 passed its
[owned-session acceptance](docs/planning/reference/sessions-plan.md#acceptance-2026-10-25)
and its phase backup was verified at `866cf85`. Slice 005 is selected for
native accessible controls; its implementation and acceptance remain pending.
Later features remain planned until their acceptance passes.

## Current repository status

Wardoff is a source-first Windows MVP. [Cargo.toml](Cargo.toml) owns the current
package version; the scope below describes supported behavior, not release status.

The branch already contains a working runtime plus CLI centered on one main promise: keep a machine in a visible **Block** state using conservative, documented Windows techniques where possible.

### Implemented and supported now

- Layer 1 interactive shutdown and sign-out blocking
- Layer 3 protection for `\Microsoft\Windows\UpdateOrchestrator\Reboot` when that task exists on the machine
- Layer 4 best-effort remote shutdown abort polling
- idle-sleep and automatic display-timeout prevention through Power Requests (no explicit Sleep/Hibernate veto)
- optional system-only Block allowing the display's normal inactivity timeout;
  ordinary Block keeps the existing system/display pair
- tray UI with Block/Allow state and power actions
- CLI control surface
- read-only `doctor` explanations and versioned JSON diagnostics
- structured rotating JSONL logs
- named-pipe IPC and single-instance behavior
- Task Scheduler autostart

### Current messaging boundaries

Keep top-level docs and user-facing answers conservative:

- Wardoff does **not** promise to stop `shutdown /t 0 /f`
- local `shutdown.exe` handling should be described cautiously, not as a guaranteed flagship feature
- IFEO interception is not shipped user-facing behavior
- Windows Event Log integration is not shipped
- toast notifications are not shipped
- timers are not shipped
- profiles are not shipped
- settings UI is not shipped
- packaging/distribution polish is not shipped

## What a new agent should conclude today

The project is **past greenfield planning**. The new product direction now has a
researched roadmap; begin with its prerequisites rather than a new broad brainstorm.

The most likely near-term work is to make the current MVP easier to understand, validate, and harden before expanding the scope.

## Prioritized next steps

### 1. MVP hardening and validation

Keep improving confidence in the features that already exist:

- tighten documentation around actual runtime behavior and limits
- improve manual validation guidance for shutdown, sign-out, sleep, hibernate, remote shutdown, autostart, and elevation-sensitive paths
- fix bugs or rough edges in tray, IPC, logging, and Task Scheduler flows
- retain regression coverage for the local status fallback change: connection retries share one absolute two-second deadline across status and legacy fallback, preserving the initial status retry grace and remaining fallback budget. Formatting, compilation, strict Clippy and 74 Rust tests passed, including the eight focused IPC cases; one native Power Request test was excluded. Isolated CLI runs with an occupied session mutex and both endpoints unavailable measured median duration of 3882 ms before and 2110 ms after, with matching output after timestamp normalization; these measurements cover only that case. Connected response reads remain outside the budget; inactive status avoids retries and normal control retains 20 attempts
- keep admin versus non-admin behavior explicit
- keep LTSC-style Layer 3 skip behavior documented as normal when the UpdateOrchestrator reboot task is absent

### 2. Source-first operator/developer polish

Make the current source-first MVP easier to adopt without overstating it:

- improve repo read order and truth-source docs
- keep README, plan, architecture notes, and changelog aligned
- keep README, plan, architecture notes, copilot guidance, and changelog aligned on the read-only CLI contract, both IPC pipes, default startup behavior, and exit codes
- clarify expected build/run flow for a first-time evaluator
- preserve honest messaging around local forced shutdown limits

### 3. Cautious post-MVP expansion

After the tested baseline, follow the explicit gates in the reference roadmap:
explain protection, separate system/display intent, add owned timed sessions and
human controls, validate recovery, then improve free adoption and command workflows.
Better local visibility remains valuable; stronger interception is not a required
next step and must not turn `shutdown /t 0 /f` into a fake promise.

## Later backlog, not current MVP

These are not present-tense product claims. Timers, focused session controls and
free distribution now have pending roadmap slices; the remaining ideas stay
deferred unless new evidence justifies them:

- stronger local `shutdown.exe` handling approaches
- Windows Event Log integration
- toast notifications
- timers
- profiles
- settings UI
- packaging/distribution work

## Decision rule for "what's next?"

When someone asks only **"what's next?"**, answer from this file in this order:

1. current focus is MVP hardening and clearer validation/documentation
2. next likely engineering work is improving the already-shipped layers and operator flow
3. later-phase feature ideas exist, but they are backlog items, not the current committed scope

For the maintainer's longer-term ambition, use the linked vision and roadmap;
do not treat unimplemented slices as current supported features. The execution
budget is zero: no paid tools, certificates, runners, hardware or services.
