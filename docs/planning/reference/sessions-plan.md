# Owned sessions implementation plan

Slice: `RM-reference-004`. Started: 2026-10-24. Status: design reviewed;
implementation in progress. Independent review accepted the corrected plan:
cleanup retries derive their target from the current ledger and wake pause,
so a failed old cleanup cannot clear a newer intent.

**Goal:** independent, inspectable temporary protection that ends with its owner
or deadline, preserves indefinite manual Block, and stays cancelled after Allow.

**Authority:** [PLAN.md](../../../PLAN.md), the
[slice contract](work-sessions.md#rm-reference-004-owned-sessions-and-expiration),
[architecture](../../ARCHITECTURE.md), [CLI](../../../README.md) and
[validation matrix](../../MVP_VALIDATION_MATRIX.md). The maintainer authorized
the remaining roadmap, testing and phase-by-phase feature-branch backups. This
is the next dependency after accepted slice 003, backed up at `88e6345`.
Continue on `codex/reference-baseline`; preserve other worktrees and hooks.

## Facts and design

Today the application owns one mode and one power intent, and waits indefinitely
for messages. IPC has one request per connection and authenticates the user via
its existing session-scoped pipe ACL. It does not capture a client process.
Wake restoration remembers a previous mode/intent; that alone cannot distinguish
manual protection from a session that expired while Windows slept.

Use a pure bounded ledger at `src/runtime_policy/sessions.rs`, an owned process
handle boundary at `src/session_owner.rs`, and the existing application effects
and IPC. No package, service, persisted session, policy change or paid resource.
The new protocol is a building block for the native panel (005) and command
wrapper (006); this slice adds no temporary-session CLI syntax. Doctor exposes
sessions. Legacy Block/Allow, default launch and read-only CLI remain usable.

### Clock, bounds and identity

Read Win32 `GetTickCount64` directly. Microsoft documents that its elapsed time
includes sleep/hibernation and is independent of wall-clock corrections:
[Windows Time](https://learn.microsoft.com/en-us/windows/win32/sysinfo/windows-time),
[GetTickCount64](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-gettickcount64).
Do not substitute Rust `Instant`, an unbiased clock or a framework wrapper whose
sleep semantics differ. Use checked millisecond arithmetic and injected `u64`
ticks in pure tests. Refreshing a request periodically is forbidden; the timer
only expires/reconciles intents and observes owner exit.

At most 64 pending or active sessions total. Duration is 1 ms through 7 days,
inclusive; reject zero, larger values, overflow and invalid wire numbers with a
fixed readable error. Manual Block has no deadline. An explicit reason is
required, trimmed, 1 through 80 Unicode characters with no control characters.
Reasons appear in live diagnostics only; never log their text, process arguments
or complete incoming session frames.

An owner key is the kernel client PID plus process creation FILETIME. Obtain
the PID and Windows session from the connected pipe, require the runtime's
Windows session, open only query-limited-information/synchronize rights, and keep
the real process handle until its last pending/active session ends. Reject an
unavailable, already-exited or uninspectable owner. Retained handles and creation
identity prevent later PID reuse from adopting or releasing existing sessions.
The primary can own panel sessions using the same process-handle boundary.
Never terminate an owner or its work. This is lifecycle isolation inside the
existing same-user boundary, not protection against hostile same-user software.

### Protocol and cancellation

Add four commands on the existing control pipe, leaving legacy frames unchanged:

1. `begin_session` captures the owner and returns a `SessionKey` containing
   `runtime_id`, `generation` and monotonically allocated `id`. The runtime ID
   is a fresh Win32 `CoCreateGuid` value encoded as 16 bytes, not a wall-clock
   timestamp that could repeat after clock changes. The ready response identifies
   session protocol version 1. The pending ticket expires after 5 seconds
   and requests no protection. Beginning a new session represents a fresh user
   action; clients must never automatically begin again after global cancellation.
2. `acquire_session` supplies that key, `duration_ms`, `intent` and `reason`.
   It must come from the ticket owner before ticket expiry. Acquire and
   acknowledge the aggregate protection before replying with its session view.
   A repeat with the same original parameters returns the existing view without
   extending its deadline. Conflicting repeats fail. A released/expired ticket
   cannot be acquired again.
3. `extend_session` supplies the key, expected revision and new duration from
   now. Only the owner can extend a live session. Revisions start at 1, increase
   with checked arithmetic, and reject a stale duplicate without changing time.
4. `release_session` releases only a matching owner's entry. A missing key in
   the current generation is an acknowledged no-op, making duplicate release
   harmless. An existing other owner's key is rejected. An old runtime or
   generation is explicitly cancelled and cannot rearm anything.

Use distinct response variants for ready tickets and active session views; no
legacy fallback. An old server rejecting a session command produces clear
restart guidance. Never send these mutations on the read-only status pipe.

Connections are deliberately short-lived. Closing a connection does not release
an owner's session. If an acknowledgement is lost, retry exactly the same
acquire key/parameters; it neither duplicates nor extends protection. If the
client exits instead, its retained process handle is observed within the normal
one-second maintenance interval. A failed release has the same bounded owner
and duration fallback. Test disconnect before/after an acknowledgement, not an
invented permanent-connection requirement.

Allow clears manual intent, all pending/active sessions and pending wake
restoration, and advances generation before attempting Windows cleanup. Retried
old acquire/extend/release messages cannot undo this stop, even if cleanup or a
tray update fails. Counter exhaustion disables new tickets rather than wrapping.
Only a fresh explicit Begin or manual Block can request new protection. Global
stop never terminates processes or their work.

### Aggregation and failures

The ledger contains independent optional manual intent and session entries.
Pending tickets do not contribute. With neither, effective mode is Allow.
Any manual/active entry requests Block; system-and-display wins over system-only.
Bare Block, Hide, default launch and tray Block select indefinite full-pair
manual intent; explicit display-timeout selects that manual intent. They preserve
other active sessions. Expiry/release/owner loss never clear manual intent.

For acquire or manual Block, prepare a ledger draft, apply aggregate effects
using existing activation/UI rollback, then commit the draft only on success.
For expiry, release and global stop, cancellation is irrevocable. Commit removal
first and reconcile Windows state without rolling protection back after a tray
error. Report a cleanup/reconciliation failure in diagnostics, retry on the
maintenance interval, and clear the warning only after the target and tray are
successfully reconciled. No completed/failed cleanup can resurrect an entry.
Every retry recomputes its target from the current committed ledger and current
wake pause. Never queue or retain a target mode/intent: a newer manual Block or
session acquired after a failed cleanup must survive the retry. Cover both
failed-cleanup-then-new-manual and failed-cleanup-then-new-session orderings.
Rate-limit repeated identical errors. Do not acknowledge an acquisition whose
Windows protection was not acquired; retain existing per-layer degradation.
After any failed activation/rollback, schedule reconciliation against the last
committed ledger even when it has no sessions; a failed draft must not become
the only representation of the effective intent.

Use a native message wait with a one-second timeout only while entries or a
cleanup retry exist; otherwise retain the indefinite wait. Sweep deadlines and
owner liveness before processing requests and before wake restoration. Release
unused process handles after each sweep. Production ticks never use wall time.

Tray Block must stay available when protection is session-only so a person can
request indefinite protection. Preserve the legacy full-pair manual labels;
label the session-only action `Block indefinitely`. Allow must be available
when any intent or pending restoration can be stopped. Red/green continue to
represent the runtime's applied Block/Allow mode.

Use the fixed native reason `Wardoff Block mode: idle-power protection.` because
the shared Windows request can now represent manual and temporary intentions.
Doctor carries individual reasons and requested display intent. Preserve dated
evidence of the previous manual-only wording.

### Suspend and restoration

Tray Sleep/Hibernate temporarily release effects while retaining the ledger.
Maintenance may expire entries during that pause but must not reapply protection
before resume or an explicit new command. On resume, prune expired/dead owners
first and reconcile only the current aggregate; never restore the saved old
intent blindly. Renew a still-current Power Request once as before, never after
its last intent expired. A failed power action restores only still-valid intent.
Shutdown/Reboot success clears the ledger; failure restores current valid
intent. Explicit Allow cancels restoration; forced cleanup and quit clear the
ledger and owned handles. Expiry never invokes any power action.

### Diagnostics and compatibility

Keep legacy `--status` JSON bytes/keys and exits unchanged. Add optional/defaulted
live-snapshot fields `sessions` and `session_cleanup_pending` to schema 1; old
snapshots remain valid and old readers ignore added fields. The additive Doctor
runtime fields expose a bounded list: key, owner PID, reason, intent, remaining
milliseconds and revision, plus cleanup state. `manual_indefinite` comes from
the ledger, not the effective Block mode. Human Doctor lists reasons and remaining
time, and makes pending cleanup explicit. Reject malformed/oversized session
snapshot data as unavailable rather than presenting it as trusted live evidence.

## Execution and validation checkpoints

- [x] **1. Review and save this plan.** Resolve actionable review findings before
  behavior changes. Run Docs and preserve the accepted plan in a local commit.
- [x] **2. Pure ledger RED/GREEN.** Delegate only
  `src/runtime_policy/sessions.rs`. Test overlap, aggregation, manual precedence,
  pending expiry, ownership, reused PIDs, idempotent acquire/release, revisions,
  bounds, overflow, owner disappearance, cancellation and no rearming. Delegate
  execution to the validation lane; retain actual RED before implementation.
- [x] **3. Transport/owner RED/GREEN.** Root owns `src/ipc.rs`,
  `src/session_owner.rs`, `Cargo.toml`'s existing Windows feature list and the
  module declaration. Validate wire compatibility, status-pipe rejection,
  unavailable/exited process, same-session identity and owned-handle cleanup.
- [x] **4. Application/diagnostics RED/GREEN.** Root owns application and wake
  integration, diagnostics and tray changes. Test cancellation versus failed
  effects/UI, failed cleanup followed by a new intent before retry, suspend
  expiry before restore, no periodic re-acquire and actual
  process-lifetime behavior. Update focused existing lifecycle tests. Local/inert
  checks are complete; real runtime qualification remains in checkpoint 6.
- [ ] **5. Review and Full.** Update architecture, README/development map and
  matrix owners. Obtain independent diff review; run Docs, developer checks,
  smoke safety and Full MSVC checks. Save exact source before VM qualification.
- [ ] **6. Isolated desktop qualification.** Coordinate free local resources;
  reuse the owned offline guest only after checking its identity. Run exact
  source-bound standard/admin smoke and a session fixture: overlap, owner exit,
  foreign release, duplicate/lost reply, manual preservation, Allow cancellation,
  expiry and native request cleanup. Qualify suspend expiry where the same VM
  supports it; physical-device rows remain untested. Never use the host runtime
  or another agent's tasks/processes. Archive protocol source and raw receipts.
- [ ] **7. Save the phase and continue.** Record source/artifact hashes, results,
  failures/skips and cleanup. Run the private publication gate, push only the
  authorized feature branch without tags, verify its exact remote SHA and Git
  durability, then select 005. Do not merge dev/main or trigger a release.

Stop dependent work for a material unresolved contract, a private-content gate,
unknown resource ownership or unintended effects outside the isolated guest.
Continue safe independent work; preserve failures and useful work. Hardware or
volunteer gaps limit claims and do not authorize spending or unsolicited contact.

## Implementation record 2026-10-25

The reviewed plan was saved at `bfca3e2`. Actual test checkpoints retained their
initial compiler failures separately from behavioral RED. Ledger/native-owner
and wire RED preceded implementation. A revision-overflow regression exposed a
deadline mutation on failure; validating revision before mutation corrected it.
Doctor/session bounds, retry-current-intent and tray-accessibility policy tests
also ran RED before GREEN. The inert production-IPC fixture demonstrated that
caller fields cannot select its process owner. It also exposed caller-controlled
enum text in rejection logs; rejection logging now uses a fixed message while
the caller still receives the detailed error.

The first complete Cargo run passed 209 top-level tests (plus the existing
nested shutdown helper); its one ignored diagnostic child is exercised through
its parent regression. This is local/inert evidence, not session runtime
acceptance. A further RED covers native-wait deadlines: new messages must not
postpone the next expiry check. Full checks and isolated desktop qualification
are still pending at this record.

The original free delegation lane was unavailable. After local native-worker
quota exhaustion, the root continued implementation and validation without a
paid fallback; the authorized native Luna review lane remained available.
Independent code review found a stale extension acknowledgement and release
errors after logical removal. Three actual RED tests reproduced the issues.
Acquire/extend now recheck a live view after maintenance; logical releases stay
acknowledged while failed cleanup remains visible/retryable, and duplicate
release invokes no effects. Review of the corrected paths passed. A further
tray regression preserves the manual full-pair action while a temporary session
supplies display protection over a system-only manual choice.

Final local Full checks passed: 216 top-level tests, one ignored diagnostic child
exercised by its parent, strict Clippy and release build. The inert status pipe
also rejects write-capable clients with access denied and no dispatched command.
Developer checks and all seven smoke-safety fixtures passed. No protection
runtime or state-changing acceptance ran on the host. Source checkpoint and
offline Windows qualification follow; the phase is not accepted yet.
