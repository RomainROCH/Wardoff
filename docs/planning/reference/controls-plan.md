# Accessible session controls: core design

Slice: `RM-reference-005`. Started: 2026-10-25. Status: cancellation core
implemented, reviewed and validated locally; native view design and
implementation are not ready yet.

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

### Application and surface exchange

The following is the controller contract for the later implementation, not a
claim that the panel exists. It does not settle the pending visual design.

Keep the application thread as the sole owner of the ledger, retained process
handles, native effects and wake restoration. Reuse the existing tray thread
for the modeless view. Its callbacks only enqueue typed requests; they never
wait for the application, call Power Request APIs or dereference
`ACTIVE_APPLICATION`. This also lets the view report a delayed response while
the application is inside an explicit Sleep/Hibernate call.

Use an internal `SurfaceAction` envelope around existing `TrayAction` values
and new controls requests on the existing action channel and wake message.
Keep the current copyable tray actions and their dispatch separate from request
payloads. In particular, the existing debug-format tray error logging must not
receive a session reason or a whole controls request.

A controls request contains a monotonically increasing request ID, its
suspend-inclusive send time, one typed operation and a one-result reply channel.
The application uses a nonblocking reply send; a closed view must not stall
protection. Operations are refresh, start a session, add time, stop selected,
and stop all timed sessions. Permanent Block and global Allow use their existing
application handlers. They remain available independently of a pending timed
operation.

Allow one outstanding controls request for the surface service's lifetime.
Hiding and reopening the view retains its request ID and reply receiver; it
does not reset this bound. Enforce the bound before enqueueing: the existing
application action channel is unbounded. Refresh on opening and at most once
per second while visible, using the existing surface polling loop. Do not
queue refreshes while awaiting a reply or while hidden. No new worker, periodic
native-request refresh, pipe command or serialized protocol is needed. Retain
the reply receiver after two seconds without a reply and mark the view stale;
do not report that the operation failed or resend it. A late reply resolves the
same request ID. Disconnection marks controls unavailable. Ordinary refresh or
reopening never retries a start or an extension.

Before each request, the application prunes expired/dead entries against the
real retained handles. A snapshot contains the runtime/generation epoch,
capture tick, manual intent, effective intent, at most 64 active `SessionView`
rows, and the same layer/request/cleanup/wake observations used by diagnostics.
Use `capture_status` and the current ledger; do not launch `powercfg`, read log
history or infer native success from requested intent. A visible countdown may
subtract elapsed ticks from remaining time, but it is not authoritative and
cannot authorize an operation. Keep expired selection keys distinct from a new
row at the same position.

Every timed mutation carries the epoch from its snapshot. Validate it against
the application ledger before any mutation. A different runtime/generation is
a stale request, including a start queued before global Allow or stop-all-timed.
Reject a timed mutation that has waited five seconds before handling. Refresh
has no mutation deadline. Permanent Block/Allow do not require a recent snapshot;
Allow must remain usable when the view is stale. Checked request-ID exhaustion
disables timed controls instead of reusing IDs. These deadlines bound admission,
not the duration of Windows calls or a shutdown veto.

| Operation | Application transaction |
| --- | --- |
| Start | Inspect `SessionOwner::current`, then begin and acquire in one cloned ledger using the validated reason, intent and duration. Apply aggregate effects and commit through `commit_intent_draft`; retain the owner handle only for committed entries. Prune again and acknowledge only a still-active session. No failed draft leaves a ticket or a new committed owner entry. |
| Add time | Resolve the full selected key in the current ledger and require its expected revision. Add the requested positive amount to its freshly computed remaining duration with checked arithmetic and the seven-day bound. Preserve the original owner and reason. A trusted in-process ledger helper may share the existing extension logic; owner-checked IPC extension keeps its checks and existing set-remaining semantics. |
| Stop selected | Use `cancel_session` for that full key, prune unused owner handles and reconcile from the current aggregate. A duplicate removal is an acknowledged no-op; no peer or manual intent changes. |
| Stop all timed | Use `cancel_sessions`, drop unused retained owners and reconcile. Pending tickets are included and the epoch advances even when empty. Manual intent is preserved. |

For selected/all cancellation, clear pending wake restoration only if no
manual or session intent remains; preserve an intentional pause for surviving
choices. A new explicit Start follows the existing acquisition semantics for
clearing a pause. Cancellation is irrevocable even if native cleanup fails:
reply with the removed state and visible cleanup failure, retain retry state,
and never reconstruct a cancelled entry. Activation failure instead keeps the
previous committed ledger and reports that failure. Reconciliation retries
always use the current ledger, never a target cached by the window.

After handling a request, return its ID, a typed outcome and a freshly captured
snapshot, including recoverable errors. The application resolves owner identity
from its own ledger/retained handles; the window supplies no owner credentials.
Selected extension is a deliberate trusted human action for the selected row,
including a peer-owned row; it does not transfer ownership or grant that peer
new IPC rights. An expired, revised or globally cancelled selection refreshes
with an explicit stale-selection result and never starts a replacement session.

Controller acceptance must inject delayed/closed reply channels, stale epochs,
revisions, expiry during activation, full capacity, duration/ID overflow and
native activation/cleanup failure. Verify actual call order and committed
state, peer/manual preservation, fresh replies, no automatic resends, no reason
in logging, and no extra periodic work while hidden. These checks complement
the native keyboard/accessibility qualification; they cannot replace it.

### Native view still pending

API evidence checked on 2026-10-25 constrains the pending native design:

- Standard Win32 controls have [built-in UI Automation providers](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-providersoverview).
  This supports using ordinary buttons, edits and a list box; it does not prove
  that this application's labels, selection, focus or updates are accessible.
- [IsDialogMessageW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-isdialogmessagew)
  can provide dialog keyboard navigation to an ordinary window with controls.
  A message it consumes must not also pass through Translate/DispatchMessage.
- Use paired [system colors](https://learn.microsoft.com/en-us/windows/win32/uxguide/vis-color)
  and native controls. Verify live theme/high-contrast changes and readable focus;
  a colored tray icon alone cannot communicate protection or failure.
- The current manifest declares execution level, not DPI awareness, and
  `build.rs` embeds it only in release builds. Microsoft recommends declaring
  [DPI awareness in the manifest](https://learn.microsoft.com/en-us/windows/win32/hidpi/setting-the-default-dpi-awareness-for-a-process),
  with an ordered fallback for older Windows. A per-monitor implementation must
  handle [WM_DPICHANGED](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged),
  including fonts, child layout and the suggested window rectangle.
  [GetDpiForWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdpiforwindow)
  requires Windows 10 version 1607: do not introduce an unconditional import
  that silently raises the documented Windows 10 baseline. Test the release
  artifact's actual manifest and scaling, rather than relying on a debug build.

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
- [x] Ledger cancellation RED/GREEN, review, Full and local checkpoint.
- [x] Specify and independently review the nonvisual application/surface contract.
- [ ] Complete and review the native view design and execution brief.
- [ ] Implement and validate native controls and application integration.
- [ ] Complete source-bound desktop/accessibility qualification and record limits.
- [ ] Save the accepted phase remotely, verify its exact SHA and continue to 007.

## Core checkpoint, 2026-10-25

Plan checkpoint: `581d6b3`. Five new cancellation cases first failed behaviorally
against no-op stubs (0 passed, 5 failed), separately from the initial missing-method
compiler errors. The implementation then passed all 24 ledger cases and
independent source review. Full passed documentation (38 files), formatting,
locked compilation, strict Clippy and release build. Cargo reported 231 top-level
test executions plus the existing nested shutdown helper; the five new cases
are included by three test targets. The existing ignored diagnostic child stays
covered through its parent regression.

Core source checkpoint `a2d609e` was backed up to the established working branch;
the remote SHA was verified and no Actions run was started. The subsequent
nonvisual controller contract passed independent review. Its one-request bound
must be enforced across hide/reopen before enqueueing on the existing unbounded
channel. Native view design and implementation are still pending.

The checked ledger source SHA-256 is
`9196E229DC72EB410DC549A7231EE57C5906F9345491867A55B8960BD75C6DEC`.
This checkpoint changes only cancellation factoring and tests: no transport,
tray, native view, startup default or user-visible protection behavior changed.
It is not acceptance of slice 005. No protection runtime ran on the host.

Failed validation wrappers remain evidence, not code failures or successful
Full runs: one lacked execution permission; another converted ordinary Cargo
stderr into a terminating PowerShell error. Native OS stream capture then ran the
unchanged check script to exit 0 with an unchanged source hash. Preserve native
exit status when automating checks; do not suppress stderr or weaken the checks.
