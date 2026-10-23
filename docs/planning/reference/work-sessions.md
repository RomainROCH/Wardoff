# Work sessions

Section goal: add optional protection for a reason and duration alongside simple
indefinite Block/Allow, with ownership a human can inspect and cancel. These are
planned contracts, not existing CLI features.

## Items

### RM-reference-004 Owned sessions and expiration

Status: pending. Depends on: RM-reference-003. Priority: P1. Size: L.

Goal: two jobs can request protection without undoing each other's requests.

Work: first specify session identity, owner lifetime, bounded expiration,
aggregate intent and IPC compatibility. Keep decisions near
[runtime_policy.rs](../../../src/runtime_policy.rs), effects in
[main.rs](../../../src/main.rs), transport in [ipc.rs](../../../src/ipc.rs).
Timers use an elapsed-time contract that handles suspend and clock changes;
validate the chosen Windows clock semantics before coding.

Acceptance: ending one of two overlapping sessions preserves the other; each
owner may release only its own session through the new API. Missing/reused PIDs,
disconnect, duplicate release, zero/overflow durations and resume after expiry
have deterministic tests. Timed expiry releases protection and never powers off
the PC. Existing `--block` retains its explicit indefinite manual intent;
expiring or releasing an optional task/timer cannot cancel that manual request.
`--allow` remains global stop. It cancels sessions and pending wake restoration;
old owner messages cannot silently rearm protection. Global stop leaves user
work running. Expose reason and remaining duration; do not log sensitive arguments.

Evidence: none. Notes: session ownership is a lifecycle contract within the
existing same-user security boundary, not a new hostile-local-user defense.

### RM-reference-005 Accessible tray and session panel

Status: pending. Depends on: RM-reference-004. Priority: P1. Size: L.

Goal: ordinary users can use Block/Allow directly, or optionally start a timed
session, understand it and stop it.

Work: extend [tray/mod.rs](../../../src/tray/mod.rs) and add a small native
session view using the existing Windows stack. Keep one-action permanent
Block/Allow available without creating a task or timer. Offer background-work and
presentation choices, 15/30/60-minute shortcuts and custom duration. Show the
same state as CLI diagnostics. Validate the small view before expanding settings.

Acceptance: keyboard-only and screen-reader users can use permanent Block/Allow,
start, inspect and extend an optional session, stop it without clearing manual
Block, and stop all. Preserve red Block/green Allow semantics, with
text and accessible names rather than color alone. Show degraded/unavailable
protection visibly, reason, remaining time and an OS-limit explanation. No hidden
default shutdown on expiry. Include English/French critical controls with
reviewable strings; test high contrast, scaling and Explorer restart. Do not
change default launch behavior without an explicit compatibility decision.

Evidence: none. Notes: do not turn this into a settings application or an embedded
browser runtime. Keep implementation details out of the human workflow.

### RM-reference-006 Command-scoped protection

Status: pending. Depends on: RM-reference-004, RM-reference-007. Priority: P1. Size: L.

Goal: protect a command for its real lifetime with normal terminal behavior.

Work: design the proposed `wardoff run -- <program> <arguments>` contract and
implement it through the same sessions as the tray. Pass arguments as an argument
vector, never by constructing a shell command. Define direct-child lifetime
first; descendant/background-job tracking is not implicit.

Acceptance: acquire/acknowledge protection before starting the child. Launch
failure creates no stale session. Preserve stdin/stdout/stderr, working directory
and child exit status, including nonzero exits. Define and test Ctrl+C, wrapper
failure, owner disconnect, global stop and cleanup while another session is
active. Never kill an unrelated process or rearm a globally cancelled session.
Do not retain full command lines in logs. Document detached-child limitations.

Evidence: none. Notes: PowerToys already offers process attachment; this is
workflow quality and integration work, not a claim that process tracking is new.

### RM-reference-007 Battery, lock and resume behavior

Status: pending. Depends on: RM-reference-004, RM-reference-005. Priority: P1. Size: M.

Goal: protection does not become an unexplained battery drain or stale wake state.

Work: add explicit policies for new automatic sessions: default to system-only
for background work, decline automatic acquisition at battery <=10%, and release
automatic requests at <=5%. Manual indefinite protection shows a warning and
remains visibly user-controlled; OS critical-battery behavior always takes
precedence. Unknown battery data must be labeled unknown. Never terminate work.

Acceptance: injected AC/DC, low/unknown battery, lock/unlock and suspend/resume
tests cover policy decisions and expiration. Locking never disables lock-screen
security; presentation requests may not force a locked display back on. Test
actual behavior on available systems and disclose untested rows. Explicit
Sleep/Hibernate must proceed. Do not refresh requests to evade Modern Standby
battery limits. No changes to sleep timers, lock policy or power plans.

Evidence: none. Notes: these thresholds are proposed new-session defaults and
must be stated in the UI; they do not silently redefine legacy Block behavior.

## Session log

- 2026-10-23: added pending session, UI, command and recovery slices. No new
  product syntax or behavior is implemented by this document.
