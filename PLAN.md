# Wardoff plan

This file is the repo-level status and roadmap summary for Wardoff.

If you are new here, read in this order:

1. `README.md` for the current user-facing MVP snapshot
2. `PLAN.md` for current status, boundaries, and likely next work
3. `docs/ARCHITECTURE.md` for the code map
4. `CONTRIBUTING.md` and `.github/copilot-instructions.md` for workflow and scope guardrails

For monetization, sponsorship, signed-binary, paid-support, or broader commercialization decisions, follow `docs/BUSINESS_MODEL.md` rather than inferring policy from engineering docs.

## Current repository status

Wardoff is currently a shipped, source-first **0.1.0-style MVP** for Windows.

The branch already contains a working runtime plus CLI centered on one main promise: keep a machine in a visible **Block** state using conservative, documented Windows techniques where possible.

### Implemented and supported now

- Layer 1 interactive shutdown and sign-out blocking
- Layer 3 protection for `\Microsoft\Windows\UpdateOrchestrator\Reboot` when that task exists on the machine
- Layer 4 best-effort remote shutdown abort polling
- sleep, hibernate, and display-idle blocking
- tray UI with Block/Allow state and power actions
- CLI control surface
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

The project is **past greenfield planning** and **not** waiting for a broad feature brainstorm.

The most likely near-term work is to make the current MVP easier to understand, validate, and harden before expanding the scope.

## Prioritized next steps

### 1. MVP hardening and validation

Keep improving confidence in the features that already exist:

- tighten documentation around actual runtime behavior and limits
- improve manual validation guidance for shutdown, sign-out, sleep, hibernate, remote shutdown, autostart, and elevation-sensitive paths
- fix bugs or rough edges in tray, IPC, logging, and Task Scheduler flows
- reduce the current roughly 4-second inactive `wardoff --status` path caused by sequential IPC retry loops
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

Only after the current MVP feels stable should the project take on the next layer of user-facing scope. The first candidate area is better local shutdown visibility/handling, but it must stay honest about Windows limits and must not turn `shutdown /t 0 /f` into a fake promise.

## Later backlog, not current MVP

These remain later-phase ideas, not present-tense product claims:

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
