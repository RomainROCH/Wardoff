# Foundations

Section goal: make the current runtime safe to extend and its claims explainable.
Source paths identify existing owners. Only the active slice recorded in the
index is being executed; later slices remain planned work.

## Items

### RM-reference-001 Tested integration and truth baseline

Status: done. Depends on: none. Priority: P0. Size: M.

Goal: one recoverable integration head with known limits and no duplicated fixes.

Work:
- Inspect current branches, PR state and active worktrees. Reconcile developer
  tooling and the existing IPC request-bound fix against the actual integration
  state. On 2026-10-23, remote main `8c45977` already contains both originals;
  `dev` still points to the older `465c24e`, so the local candidate uses main.
  Do not recreate that fix or disturb another agent's unmerged work.
- Recheck open findings in [SECURITY_AUDIT.md](../../SECURITY_AUDIT.md) against
  actual code; distinguish fixed findings, residual risks and untested cases.
- Bind existing and new acceptance evidence to exact heads. Use
  [DEVELOPMENT.md](../../DEVELOPMENT.md) and the
  [validation matrix](../../MVP_VALIDATION_MATRIX.md), including isolated smoke.
- Before any push, verify repository visibility, runner billing and triggered
  automation. If execution cannot be guaranteed to cost zero, keep validation
  local and prepare the candidate without triggering billable work.

Acceptance: Full checks, developer fixtures and smoke-safety fixtures pass on
the chosen head. After integrating the existing request-bound fix, run
`cargo test --locked --test ipc_request_limits -- --nocapture`, including
`isolated_runtime_rejects_bad_clients_and_recovers` from
`tests/ipc_request_limits.rs`. Record rejection/recovery for malformed,
oversized, unterminated, silent and trickling clients, healthy fragmented
requests, independent status availability and shutdown with an incomplete client.
Retain exact command, source SHA, exit status and concise results. If the suite
was renamed, record the mapping and equivalent cases rather than omit the gate.
Run the real smoke only in an isolated environment and record skips. The
integration report names retained work,
unmerged dependencies and the permitted claim set. Merge/publication follows
its authorization; a local integration is not described as remotely released.

Evidence: [baseline record](baseline-2026-10-23.md). Fresh Git/PR inspection,
Full MSVC checks and the explicit eight-case IPC gate passed. Isolated CLI
startup passed after a static-CRT fix. Follow-up desktop smoke passed with full
and limited UAC tokens; the service-session failure was traced to a refused
display Power Request. A test-only correction handles equivalent Windows paths
without weakening task ownership checks. Protected-path autostart and tray
checks passed; owned runtime/task cleanup and guest shutdown were verified.
Recorded skips and physical/manual gaps remain explicit. No new product feature
is part of this slice.

### RM-reference-002 Read-only explanation and diagnostics

Status: pending. Depends on: RM-reference-001. Priority: P0. Size: M.

Goal: tell a person why protection is active, unavailable or limited.

Work: extend [cli.rs](../../../src/cli.rs),
[main.rs](../../../src/main.rs) and affected layer owners with a read-only
explanation surface. Specify additive, versioned JSON fields and stable reason
codes before implementation. Keep existing `--status` output/exit behavior
compatible; a future `doctor` command is a proposal, not current syntax.

Acceptance: distinguish requested, API-active, failed, unavailable and untested
conditions without claiming physical effectiveness from a boolean. Explain
shutdown/restart coverage alongside power behavior, and show indefinite manual
Block as a normal first-class choice, not a missing timer or incomplete task.
Explain missing UpdateOrchestrator tasks, elevation/path restrictions and unavailable
IPC. No UAC, runtime startup, task mutation or power-plan edits during inspection.
Test inactive, partial failure, occupied-mutex failure and elevated/non-elevated
paths. Diagnostic output omits usernames, full personal paths and command arguments
by default; any fuller export needs preview and explicit user selection.

Evidence: none. Notes: reuse bounded JSONL logs; do not introduce a remote support
service or Windows Event Log dependency.

### RM-reference-003 System and display intent

Status: pending. Depends on: RM-reference-002. Priority: P1. Size: M.

Goal: protect a background task while letting its display turn off normally.

Work: model independent system/display requests in
[sleep.rs](../../../src/blocker/sleep.rs) and pure decisions in
[runtime_policy.rs](../../../src/runtime_policy.rs). Preserve current explicit
Block behavior and rollback. New selections expose user intent without claiming
new shutdown coverage. A display request also needs a system request, as described
by [Microsoft](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powersetrequest).

Acceptance: system-only acquires no display request; presentation intent acquires
both. Failure of either acquisition rolls back correctly, and repeated mode
changes/quit release exactly the requests owned. Test status accuracy, resume,
and preservation of the existing power plan. Record actual display/idle behavior
only on available test environments; never mark missing hardware as a pass.

Evidence: none. Notes: no synthetic input, lock suppression or power-plan rewrite.

## Session log

- 2026-10-23: created pending slices from the product/competition research; no
  implementation or new runtime validation performed.
- 2026-10-23: consolidated current main and the roadmap; fixed portable MSVC
  startup. Full/IPC checks and clean-Windows CLI startup passed. At that
  checkpoint, slice 001 stayed active pending desktop-runtime validation.
- 2026-10-23: completed the desktop follow-up, diagnosed the session-0 Power
  Request failure and corrected smoke path identity. Accepted slice 001 with
  elevated/non-elevated smoke, protected-path autostart and tray evidence.
  Slice 002 and later work remain pending.
