# Accessible session controls: core design

Slice: `RM-reference-005`. Started: 2026-10-25. Status: cancellation core reviewed;
native view design and implementation are not ready yet.

## Goal and authority

Keep permanent Block/Allow directly available. Add an optional native view for
starting, understanding, extending and stopping timed sessions. Manual protection
must remain independent of session lifetime. The maintainer authorized the
remaining roadmap, preserving the philosophy and zero-budget constraint.

The [slice contract](work-sessions.md#rm-reference-005-accessible-tray-and-session-panel),
[PLAN.md](../../../PLAN.md), [architecture](../../ARCHITECTURE.md) and
[validation matrix](../../MVP_VALIDATION_MATRIX.md) govern this work. Slice 004
is accepted from source `85125f2`; its acceptance and progress are backed up at
`72034b7` on `codex/reference-baseline`. The previous offline test guest is
closed and its resources released. Reuse the checkout and preserve other worktrees.

## Existing boundaries

- [SessionLedger](../../../src/runtime_policy/sessions.rs) owns pure identity,
  generation, deadlines, revision and aggregation. The application owns native
  effects, reconciliation, pause and retained owner handles.
- [SessionOwner::current](../../../src/session_owner.rs) already captures the
  runtime for panel-created sessions. Closing the view must not terminate the
  runtime or cancel its sessions; Quit still clears protection and exits.
- [The tray service](../../../src/tray/mod.rs) already owns a Win32 message-loop
  thread and communicates actions/state with the application. Window callbacks
  must not manipulate the ledger, power requests or other owners directly.
- Read-only CLI remains non-elevating, legacy status bytes stay compatible, and
  the pipe API continues to require the actual owner for release/extension.
  A human override is an internal application action, not a new pipe permission.
- Keep the existing Windows stack. No browser runtime, new package, persisted
  settings, telemetry, automatic power-off action or changed default launch.

## Cancellation core: independently bounded work unit

Extract two small ledger operations, usable by the later trusted human-control
path and existing owner-checked operations:

1. Cancel one current session key, including a pending ticket. Reject another
   runtime/generation. A missing key in the current generation is an idempotent
   no-op. This operation changes no manual intent and no peer entry.
2. Cancel all pending/active sessions while preserving manual intent. Advance
   generation, including when the entry set is already empty, so old tickets,
   acknowledgements and queued starts cannot rearm protection. On generation
   exhaustion, disable admission rather than wrapping; cancellation still holds.

`release(owner, key)` must retain its owner check before using single-key
cancellation. Foreign owners still cannot release a peer through IPC.
`global_stop()` clears manual intent and uses all-session cancellation, retaining
its existing behavior. These production callers keep the refactor useful and
avoid unused UI scaffolding. No transport, tray, application or user-visible
behavior changes belong in this core-only checkpoint.

Required RED/GREEN cases: selected cancellation preserves peers/manual; pending
ticket cancellation; repeated cancellation; stale runtime/generation rejection;
foreign owner release still rejected; all-session cancellation preserves each
manual intent, removes pending/active entries and invalidates old messages;
empty cancellation advances generation; overflow stays cancelled/admission-off;
global Allow still clears everything. Reuse existing overlap/idempotence tests.
Run focused ledger checks, independent diff review and Full before checkpointing.

## Later human-action contract

The native view must distinguish these choices in text and accessible names:

| Action | Effect |
| --- | --- |
| Permanent Block | Select manual full-pair protection without creating a timer |
| Allow / stop all protection | Clear manual intent, all sessions/tickets and pending restoration |
| Stop selected timed session | Cancel only that complete key; preserve manual intent and peers |
| Stop all timed sessions | Cancel all tickets/sessions and advance generation; preserve manual intent |

For example, manual system-only plus a temporary presentation session becomes
manual system-only after stopping timed sessions; Allow instead leaves no intent.
Never terminate the applications represented by rows.

Show the same authoritative live session/layer/request data as diagnostics.
Selection binds the full key, not a row index or PID. Before processing a queued
mutation, prune expired/dead entries and validate its runtime/generation; an
extension also requires the current revision. A stale view refreshes visibly
and never automatically creates a replacement session after Allow.

Starting from the panel uses this runtime's retained owner and a ledger draft:
validate, begin/acquire, apply aggregate effects, then commit/acknowledge on
success. Reopening or refreshing the view never starts protection. No automatic
retry of a start after a lost reply. Cancellation is irrevocable and failed
native cleanup stays visible/retryable using the current aggregate.

The extension control must describe its time semantics honestly. Prefer adding
the selected amount to a freshly evaluated remaining duration, with checked
arithmetic and the existing seven-day remaining-duration bound. Never label a
shortening operation as adding time. Preserve owner identity and manual intent.

Background work may let the display time out; presentation requests its system
and display pair. A stronger manual/peer request still wins and the view must
explain that. Reasons use the existing bounded validation and are never logged
with command lines or incoming frames.

## View design and acceptance still to complete

Before native view code, resolve the actual control layout, keyboard navigation,
English/French strings, accessibility names, DPI/high-contrast behavior, owned
window/font cleanup, and the precise message/snapshot exchange. Prefer standard
Windows controls on the existing surface thread; introduce no additional UI
framework or power-request refresh loop. Rendering/visibility failure must not
alter existing Block/Allow protection. Keep an unavailable/stale state explicit.

The implementation plan must include focused pure/action tests, Full, developer
checks and smoke safety; then exact-source standard/admin desktop qualification.
Exercise keyboard and screen-reader access, reasons and countdowns, manual
preservation, selected/all cancellation, stale selection, visible native errors,
English/French critical controls, high contrast/scaling, Explorer recreation and
normal Quit. Preserve source/artifact hashes, failed attempts and hardware limits.
Coordinate a new free VM unit before entry; never reuse the closed guest process
identity or ports as evidence of a new launch. Push only after phase acceptance.

## Execution checkpoints

- [x] Inspect the accepted ledger, owner and tray/application boundaries.
- [x] Independently review this cancellation core before behavior-preserving edits.
- [ ] Ledger cancellation RED/GREEN, review, Full and local checkpoint.
- [ ] Complete and review the native view design and execution brief.
- [ ] Implement and validate native controls and application integration.
- [ ] Complete source-bound desktop/accessibility qualification and record limits.
- [ ] Save the accepted phase remotely, verify its exact SHA and continue to 007.
