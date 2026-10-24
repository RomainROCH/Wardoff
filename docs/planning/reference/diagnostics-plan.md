# Read-only diagnostics implementation contract

Slice: `RM-reference-002`. Started: 2026-10-24. Status: design/implementation in
progress. This specifies the change; the commands below are not yet shipped.

## Goal and authority

Explain protection intent, API activity, failures and Windows limits without
starting protection or requesting elevation. Preserve manual indefinite
Block/Allow and existing `--status` output/exit codes.

Authority: [PLAN.md](../../../PLAN.md), [slice acceptance](foundations.md#rm-reference-002-read-only-explanation-and-diagnostics),
[architecture](../../ARCHITECTURE.md), [CLI](../../../README.md) and
[validation matrix](../../MVP_VALIDATION_MATRIX.md).
Continue from accepted, backed-up `d85a509` on `codex/reference-baseline`.
The maintainer selected the remaining roadmap for serial execution, tests and
verified phase pushes; preserve other agents' worktrees and the zero budget.

## Facts

Main returns `StatusOutput` through the existing status request. The dedicated
status pipe writes that object directly; Serde readers accept unknown fields.
Inactive CLI JSON is exactly `{"state":"inactive"}`, exit 1. The ordinary
status path also has a legacy control fallback.

Local, Update and Remote expose asynchronous booleans, losing the difference
between pending startup, unavailable capabilities and failed activation. Update
knows when its reboot task is missing but retains the cause only in logs.
Sleep acquisition is transactional; a failed resume can leave Block requested
with inactive Power Requests. Token/task/file-access queries have read-only
building blocks, but their raw errors may contain personal paths.

## Command and wire contract

Add `wardoff doctor` for human output and `wardoff doctor --json` for scripts.
Runtime flags and this subcommand conflict. Both forms dispatch as read-only
before elevation/runtime startup. Existing flags/default launch keep their behavior.

Exit 0 means a report was produced, including inactive/degraded states, not that
protection is guaranteed. Exit 1 means an existing runtime could not be inspected.
Invalid syntax keeps Clap's exit 2.

| Observation | `runtime.state` | `runtime.evidence` | Exit |
| --- | --- | --- | --- |
| No primary exists | `inactive` | `inactive` | 0 |
| Reachable compatible primary, including degraded protection | Observed `block`/`allow` | `live` | 0 |
| Reachable legacy/unsupported snapshot | Observed `block`/`allow` | `legacy` | 0 |
| Mutex/endpoint cannot be inspected, invalid response or deadline exhausted | `unreachable` | `unreachable` | 1 |
| Local probes unavailable/timed out, runtime otherwise observed | Preserve runtime observation; context is unknown | Preserve evidence | 0 |
| Invalid arguments | No report | No report | 2 |

Add an optional versioned `diagnostics` snapshot to the existing status-pipe
payload. Main adds it to its existing response. Legacy CLI serialization omits
the extension, preserving `--status` stdout. Older Serde clients ignore it; new
clients label missing/unsupported snapshots as legacy/unknown evidence.
No third pipe or mutable inspection request is required.
The extension is internal to IPC. Active and inactive `--status` stdout retain
their exact previous byte representation; test both CLI and wire serialization.

The optional wire snapshot has these fields: `schema_version`, `elevated`
(Boolean/null), `manual_indefinite`, `pending_wake_restore`,
`last_transition_failure` (layer ID/null), `layers` (ID/observation pairs) and
`power_requests` (system/display requested/API-active pairs).

Doctor starts one absolute two-second inspection deadline. IPC connection/read
uses remaining time, never a restarted budget. Afterwards, local probes/history
receive at most one second from the same remaining budget, reserving cleanup
time. If no time remains, skip probes and return unknown facts. Accept at most
64 KiB of one newline-terminated UTF-8 response; do not enter an unbounded legacy
control read. Keep the old `--status` transport intact.
Observe presence with `OpenMutexW`; never create/claim the runtime singleton for
Doctor. This avoids reserving a primary slot during a concurrent launch.

Blocking COM/filesystem probes run in one temporary read-only instance of the
same executable, using a hidden internal probe action that dispatches before
runtime startup. A standard caller uses its ordinary token. An elevated caller
uses a LUA/restricted token with administrative groups/privileges removed and
medium integrity; unsupported/service-token cases return unknown, never fall
back to an unrestricted elevated helper. There is no UAC, credential prompt or
background service. Pass only the minimal Windows/profile environment.

Own the process/thread/pipe/job handles, whitelist inherited standard handles,
create the helper suspended, assign a kill-on-close job, then resume it. Only
this owned helper is terminated/reaped on timeout, malformed/oversized output or
failure. Accept at most 16 KiB of its fixed JSON result. Never terminate a PID
looked up by name. Account for helper startup and cleanup in the remaining
deadline. This bounds application waits, not arbitrary Windows driver latency.
Check the budget between launch stages and refuse further work when exhausted.
Synchronous process creation/token/kernel calls cannot be preempted by this
application budget; publish no hard wall-clock guarantee. Retain actual timings
for normal and failing Windows cases instead of hiding this platform limitation.

## Diagnostic model

Report and runtime snapshot use `schema_version: 1`; the report includes the
package version. Consumers tolerate added fields and unknown reason codes.
The report separates runtime state/evidence, manual indefinite choice, pending
wake restore, caller/runtime elevation, each layer's requested/API-active state,
coverage, system/display request acquisition and local inspection results.

Layer IDs remain shutdown, local shutdown, Update, remote and idle power.
Cached observations are inactive, starting, active, elevation required/unknown,
task absent, activation failed, monitoring failed or unknown. Report them as
not requested, starting, API-active, failed, unavailable or unknown; physical
effectiveness remains untested by inspection. Allow with an unexpectedly active
API is visible as an inconsistency, not silently clamped to inactive.

Stable reason codes include `not_requested`, `starting`, `api_active`,
`elevation_required`, `elevation_unknown`, `update_task_absent`,
`activation_failed`, `monitoring_failed`, `legacy_runtime`, `no_runtime`,
`ipc_unreachable`, `inspection_unavailable` and `inspection_timed_out`.
Historical failures never establish the cause of a current inactive layer.
Human output starts with the current choice, including indefinite manual Block,
and explains best-effort shutdown and idle-only power protection in plain language.

The report's concrete fields are:

| JSON path | Contract |
| --- | --- |
| `schema_version`, `version` | Integer 1 and package-version string |
| `runtime.state` | `inactive`, `block`, `allow`, `unreachable` |
| `runtime.evidence` | `inactive`, `live`, `legacy`, `unreachable` |
| `runtime.manual_indefinite`, `runtime.pending_wake_restore`, `runtime.elevated` | Boolean/null; unknown legacy details stay null |
| `runtime.last_transition_failure` | Layer ID/null; a past failed transition, not current activity |
| `context.client_elevation` | `elevated`, `standard`, `unknown` |
| `context.session_kind` | `service`, `user_session`, `unknown`; no username/path |
| `context.inspection` | `complete`, `unavailable`, `timed_out` for the local worker |
| `context.update_task` | `present`, `absent`, `unknown` |
| `context.autostart.enabled` | Boolean/null |
| `context.autostart.task_identity` | `absent`, `matched`, `unverified` |
| `context.autostart.path_access` | `user_writable`, `restricted`, `unknown`; not a security certification |
| `layers[].id` | `shutdown`, `local_shutdown`, `update`, `remote`, `idle_power` |
| `layers[].requested`, `layers[].api_active` | Boolean/null, kept distinct |
| `layers[].state` | `not_requested`, `starting`, `api_active`, `failed`, `unavailable`, `unknown` |
| `layers[].reason_code` | Stable code from the observation/projection |
| `layers[].effectiveness` | Always `untested` |
| `layers[].coverage` | `interactive_shutdown`, `best_effort_abort`, `update_task_only`, `idle_timeouts` |
| `power_requests.system`, `power_requests.display` | Each has `requested` and `api_active` Boolean/null fields |
| `recent_failures[]` | Fixed `code`, validated `timestamp`/null and `layer`/null |
| `limits` | Fixed codes: `forced_shutdown_not_guaranteed`, `explicit_sleep_not_vetoed`, `policy_can_override`, `physical_effectiveness_untested` |

All five layer IDs are present. A legacy false pair status does not establish
that both individual power requests are absent. Unsupported snapshot versions
keep detailed intent/reasons unknown. History recognizes only startup failure,
failed idle-power acquisition and failed idle-power cleanup; it does not turn an
expected unavailable Update task into a historical failure.

## Ownership and privacy

Use one small shared atomic observation per asynchronous layer, replacing its
Boolean as the source of `is_active`. Preserve activation, rollback, cleanup and
thread ordering. Record a reason before unavailable/error returns; retain worker
failure until stop/retry, and reset on normal stop. Sleep retains its synchronous
acquisition/resume result. The coordinator retains the layer of a failed Block
transition without exporting its raw error. Main produces the snapshot.
Capture each layer's one atomic state once per reply, then derive its API-active
Boolean, reason and state from that captured value. Requested mode is owned by
main and cannot transition during capture. Legacy booleans and the extension
use the same capture; there is no claim of an atomic transaction across Windows
subsystems. A stopped worker cannot erase a recorded failure until explicit
stop/retry; test this lifecycle and generation/reset behavior.

`Application::pending_block_restore_after_wake` already owns the tray restoration
lifecycle in main. Snapshot `pending_wake_restore` is exactly `is_some()`:
true from a tray Sleep/Hibernate request made in Block through its pending/
waiting-for-resume phases; false at startup, explicit CLI/tray Block or Allow,
failed power action, consumed restore attempt and shutdown. Legacy/unsupported
snapshot detail is null. It is not a general indication that Windows resume
succeeded. Ordinary resume renewal has no tray pending flag; retain its success/
failure in the sleep observation. A failed tray restore clears pending and
retains the failed transition layer. Manual indefinite intent is true in Block
or while that tray restore is pending; it is false in ordinary Allow.
Tests cover no pending restore, both pending phases, explicit cancellation,
successful/failed consumption, ordinary renewal failure and legacy null values.

Read-only local probes/log inspection share the one-second helper sub-deadline;
timeout yields unknown facts. They never call task setters, alter ACLs/power
plans, start UAC or start protection. Classify autostart write access against the
non-elevated token where available; unknown is not trusted. This classification
does not certify ancestors, reparse points or task ACLs.
For autostart, expose enabled/disabled only after matching the single executable
action, working directory, interactive principal, logon type and run level to
this executable/user. An absent task is false/absent; mismatched or unreadable
identity is null/unverified and is never changed by inspection. Update task
`present` reports presence at its expected path only, not authenticity, ACL
safety or physical reboot protection.

Read at most 512 KiB of the current JSONL suffix and retain at most five
recognized historical failures. Export fixed codes and validated timestamps,
never free-form actions. Omit usernames, personal paths and command arguments.
No full diagnostic export, hosted support, new crate or paid resource is added.
The helper uses the existing Windows dependency; enable its Job Objects feature.
Correct the shared path-write check to detect any mutating right, including
partial grants and ACL/owner changes, rather than requiring every write bit at
once. This is needed for truthful path diagnostics and preserves the intended
autostart refusal. Keep the wider distribution security gate open.
When suffix truncation skips a prefix, discard its first partial/possibly partial
line; discard an unterminated last line and records longer than 8 KiB. Skip
malformed JSON/UTF-8, unknown events and records without Boolean `success:false`.
Recognize `application_start_failed`, `sleep_blocked` and `sleep_unblocked`, mapped
to `previous_start_failed`, `previous_idle_power_failed` and
`previous_idle_power_cleanup_failed`. Accept RFC3339 timestamps, normalize valid
values to UTC and use null for missing/invalid values. Return newest file-order
records first, deduplicate by fixed code/layer/normalized timestamp, and keep
at most five. Do not reorder using a possibly adjusted wall clock.

## Execution and acceptance

1. Add CLI read-only/conflict regressions and obtain expected RED from the runner.
2. Implement pure models/history projection, retained layer observations and
   read-only local probes.
3. Add the optional snapshot, bounded reader and main dispatch, preserving legacy JSON.
4. Update user/developer docs and record the intentional wire extension in architecture.
5. Validate/review, bind the executable to committed source, record acceptance,
   push the working branch, verify its remote SHA and select slice 003.

Required evidence:

- Pure tests for requested/active/failed/unavailable/untested, startup, missing
  task, unknown elevation, partial failure, unexpected active APIs in Allow,
  legacy/future snapshots and historical-data privacy.
- CLI parsing/conflicts/read-only tests and exact legacy status JSON with/without
  an internal diagnostic snapshot.
- IPC tests for fragmented, oversized, invalid, truncated, silent and trickling
  responses under the shared deadline; log suffix bounds and malformed records.
- Helper tests for normal completion, malformed/oversized output, startup/token
  failure and timeout, proving only its owned process is killed and no helper
  remains. Cover standard/elevated callers, restricted-token failure without
  elevated fallback, and partial-write access masks.
- Full MSVC checks, strict Clippy, docs and existing IPC/status regression suites.
- In the isolated signed-in desktop: inactive, elevated/non-elevated, Block/Allow,
  missing task, legacy runtime, occupied mutex/unreachable IPC, absence of UAC,
  runtime/task mutations, same-binary smoke, artifact hashes and owned cleanup.
- Review, private-content/metadata gates and a verified phase push. Preserve
  other worktrees and retain physical/disruptive gaps explicitly.

Stop only for a material unresolved decision, actual permission/privacy boundary
or required paid resource. The accepted phase hands directly to the next selected
slice. Timers, sessions, power-intent controls and release tags belong to later phases.

## Windows implementation evidence

### Implementation checkpoint, 2026-10-24

Implementation commit `d4781ca` passed the Full MSVC checks (119 tests, strict
Clippy, release build and docs) and static review. Its release SHA-256 is
`E0137A1F30B48BB191C3E6FCF6CE0F6D7B245FDC2FE4C539017398E4F4B82F62`.
The phase is not accepted or backed up remotely yet.

The isolated signed-in Windows desktop passed the normal-token Doctor cases:
inactive human/JSON, exact legacy status, Block/Allow, a legacy runtime and an
occupied mutex without an endpoint. Task definitions and the power plan stayed
unchanged. Same-binary normal smoke passed 14/14 with seven explicit skips and
an observed zero exit code. A first fixture lost the child exit code despite
successful smoke output; its corrected process-handle collector was rerun.

Elevated local inspection remains an open regression on that candidate. Token
creation, medium integrity and suspended process creation succeed, but the full
restricted child exits with `0xC0000142` before producing output. The same native
cmd/self-child regressions pass on the development host and fail in the elevated
VM task. This is not evidence that inspection completed. An empty-desktop-name
experiment reproduced the same failure and was removed. Investigation continues
without granting privileges or changing existing system ACLs.

The subsequent isolated experiment reproduced the failure with default object
descriptors and with explicit process/thread descriptors alone. Setting the
**owned restricted token's default DACL** to SYSTEM and its own user SID allowed
the child to initialize. The implementation now applies that descriptor only to
the new token, retaining medium integrity, disabled administrative groups and
removed privileges. It never changes a descriptor on an existing system object.
The same three elevated VM native regressions then passed, including a child
querying its own non-elevated token and the owned job/stdout/cleanup path. The
Green test executable SHA-256 was
`BFE859FF68280B0C25D44BD87C61F96BDED66EE27EEF2C61AF5D3EF4013DB11A`.
The final release and remaining desktop acceptance still need rerunning.

Cross-token and autostart-identity acceptance are also still pending. Private
drivers, raw outputs, source/binary hashes and failed attempts are retained in the
phase's isolated lab evidence; fixture parsing alone is not runtime acceptance.

The subsequent release from `0413065` passed the Full checks (122 tests), all
Doctor desktop cases in both token contexts, and normal-token smoke (14/14,
seven skips). Autostart acceptance then exposed a separate identity-comparison
defect: registration's existing current-user helper returns an account name,
while inspection compared it directly with a resolved SID. The new native
regression failed on that comparison. Normalize both identities to SIDs before
comparison; do not change registration, task permissions or elevation. The final
candidate must repeat desktop, autostart and cross-token acceptance after this
fix. The interrupted control connection was traced to the VM's automatic sleep;
the controller now keeps only the test desktop active during diagnostics,
without modifying its power plan. These are not physical idle-power tests.

An abandoned COM thread is insufficient for a reliable timeout: Windows documents
a possible DLL-detach deadlock when process exit encounters locks held by another
thread. This motivates the owned temporary process rather than changing the
protection runtime. See [ExitProcess](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-exitprocess).
[CreateRestrictedToken](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-createrestrictedtoken)
supports privilege/SID restrictions;
[CreateProcessAsUserW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessasuserw)
documents primary-token rights and the restricted-caller-token privilege exception.
If the platform cannot provide the restricted helper, inspection remains partial.
