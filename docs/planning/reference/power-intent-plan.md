# System and display intent implementation plan

Slice: `RM-reference-003`. Started: 2026-10-24. Status: implementation in progress.
This is planned behavior until implementation and acceptance are recorded.
Plan review accepted the design after clarifying same-mode intent changes and
legacy rejection checks. The legacy parser's error branch continues its server
loop despite misleading log wording; verify this with a subsequent valid command.

**Goal:** protect background work while allowing the display's existing idle
timeout, with the current indefinite Block behavior remaining the default.

**Architecture:** use one shared pure intent value, one process-owned Power
Request object and the existing coordinator/application boundary. Change only
the display request when an active Block choice changes; keep its system
request acquired. Preserve the selected intent across resume and tray power
action restoration.

**Technology:** existing Rust/MSVC, Clap, Serde and Win32 Power Request APIs.
No new dependency, paid resource, service or power-plan modification.

## Authority and starting point

[PLAN.md](../../../PLAN.md) owns scope;
[slice acceptance](foundations.md#rm-reference-003-system-and-display-intent),
[architecture](../../ARCHITECTURE.md), [CLI](../../../README.md) and
[validation matrix](../../MVP_VALIDATION_MATRIX.md) own their respective contracts.
The maintainer selected the remaining roadmap for serial execution with tested
phase backups. Slice 002 passed and its branch backup was verified at
`7997d84f2c95087ed3c45fa8a29425fc800cc61d`. Its binary source was `8dd14a5`.
Continue in `codex/reference-baseline`; preserve other worktrees and hooks.

## Facts and decisions

The current sleep owner already tracks each acquired request separately but
always acquires the pair. The diagnostic wire extension already has separate
requested/API-active fields. Legacy status has one `sleep` Boolean. Pending
tray wake restoration currently remembers only the action, losing any future
display choice. Microsoft requires a system request alongside a display
request; see [PowerSetRequest](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powersetrequest).

### Command and compatibility

```powershell
wardoff --block                             # existing system + display behavior
wardoff --block --display-timeout allow     # system only; allow normal display timeout
wardoff --block --display-timeout prevent   # system + display
wardoff --allow                             # release Wardoff's requests
wardoff doctor --json                       # observe each request independently
```

The optional `--display-timeout allow|prevent` requires `--block` and conflicts
with `--hide` and every other action. Reject missing/unknown values and either
ordering of read-only subcommand conflicts with Clap exit 2. The words describe
an inactivity timeout, not an immediate screen-off action. The choice applies
to this runtime session; add no persisted preference or new tray control here.
System-only removes Wardoff's display request; other applications and Windows
policy can still keep the display on.

Bare Block, default launch, Hide and tray Block explicitly select the existing
pair. All remain indefinite and preserve their current tray/console/elevation
behavior. Allow cancels pending wake restoration. A new explicit intent request
also cancels pending restoration before applying the user's choice.

Bare mode commands keep their existing IPC representation. An explicit display
choice uses a new `set_power_intent` request with `intent: system_only` or
`system_and_display`; it also requests Block. Do not add an optional field to
old `set_mode`, because an old runtime could silently ignore it. An old runtime
must reject the new request without changing mode; surface a readable restart/
version-mismatch error. Do not fall back to legacy Block after that rejection.
Preserve the current control/status access checks and session scope.
For an explicit intent only, recognize the legacy parse error identifying the
unknown `set_power_intent` variant and add a clear instruction to restart the
running runtime with this executable. Preserve activation/permission/transport
errors instead of misclassifying them as version mismatches. Verify that the
legacy server still accepts subsequent ordinary mode commands after rejection.

Keep exact legacy status keys, formatting and exits. `layers.sleep` means that
the configured idle-power request set is acquired, including system-only. Use
Doctor for the display distinction. No new diagnostic schema field/version is
needed: system-only reports system requested/active true and display false;
Allow reports both false. An unsuccessful change retains the previous intent
and records `last_transition_failure: idle_power`, separate from current APIs.
Physical effectiveness stays `untested`.

### Backend and failure semantics

Create `src/power_intent.rs` for the shared `PowerIntent` value with exactly two
variants: `SystemOnly` and default `SystemAndDisplay`. Derive the required Copy,
equality and snake-case Serde traits. Its mask is respectively `[true, false]`
or `[true, true]`; there is no display-only state. This small pure owner avoids
requiring standalone IPC tests to include the Windows sleep implementation.

`SleepBlocker::activate(intent)` and its private `PowerRequests` boundary use
that mask. On first activation, acquire system first, then optional display.
Failure rolls back only the newly acquired requests and closes the private
handle. Repeated identical activation performs no API calls.

When Block is already active, pair -> system-only clears only display;
system-only -> pair sets only display. Commit the mask only after API success.
Failure preserves the old mask/observation and the still-owned system request;
do not stop other protection layers or drop the existing object. This avoids a
protection gap and requires no replacement-handle machinery. If no object is
active after a failed resume, retry normal acquisition for the selected intent.

Use the truthful reason `Wardoff Block mode: manual idle-power protection.`
for this object in both intents. A fixed reason is needed because an in-place
choice change does not replace the object. The request category and Doctor
carry the display distinction. Update the current smoke/procedure expectation;
leave dated evidence of the old reason intact.

Allow, quit, activation rollback and forced cleanup release exactly the acquired
mask and close the private handle, retaining existing cleanup error handling.
Resume closes the old object and reacquires the chosen mask once, rather than
incrementing possibly surviving request counts. Preserve existing Windows
policy, explicit sleep and battery limits; do not add periodic refresh.

### Coordinator, application and wake restoration

The coordinator stores the selected intent separately from Block/Allow. Add a
configuration entrypoint accepting both; retain `set_mode` as the default-Block
wrapper used by existing callers. Initialize the selected intent through
`RuntimeOptions` and bootstrap. In Allow, no requests are acquired.
Every same-mode fast path must compare the intent too: system-only -> bare CLI
Block, default launch, Hide or tray Block must acquire display even though the
mode string remains Block. Cover all four routes explicitly.

An active Block intent change touches only the sleep owner. Failed changes keep
the previous mode/intent and expose the failure. Allow -> Block retains existing
layer activation ordering and full rollback. Capture requested system/display
from mode plus the selected mask, and API activity from the one observation.

Application UI rollback remembers the previous mode and intent together. If a
tray synchronization fails, attempt that exact configuration and report its
actual final state if rollback fails. Log an intent change even when the mode
remains Block. Keep runtime effects in main and pure decisions in runtime_policy.

Extend both pending-wake states with the previous intent alongside the action.
Suspend preserves both; resume restores both, even if the suspend notification
was missed. A failed tray Sleep/Hibernate request restores the exact previous
configuration. Ordinary resume of a runtime still in Block renews its current
intent. Explicit mode/intent commands cancel the pending restore as before.
Do not introduce new presence-aware or display-on-wake policy in this slice.

## Execution and review checkpoints

- [ ] **1. Obtain CLI RED.** Add parsing/conflict regressions in `src/cli.rs`
  using the existing parser. The valid system-only command must fail against
  the old parser. Runner: `cargo test --locked cli::tests`; retain the assertion
  and executed-test count, not a compilation error or zero-test result.
- [ ] **2. Introduce the pure value and backend RED/GREEN.** Root owns
  `src/power_intent.rs` and its module declaration. Delegate only
  `src/blocker/sleep.rs` with the interface above. First expose the parameter
  with old pair behavior and add meaningful fault/count regressions, then have
  the runner observe RED before implementing the new mask. Cover system-only,
  pair, no-op, repeated flips with one uninterrupted system count, each startup
  failure/rollback, failed display add/clear preserving the old state, Allow,
  Drop and resume in both intents. Runner: `cargo test --locked blocker::sleep::tests`.
- [ ] **3. Wire pure command/wake decisions.** Delegate only `src/cli.rs` and
  `src/runtime_policy.rs`. Add the explicit intent action, RuntimeOptions field,
  dedicated IPC mapping and intent-preserving pending restore tests. Cover all
  default launch modes and both actions/intents, missed suspend and cancellation.
- [ ] **4. Integrate effects and compatibility.** Root owns `src/blocker/mod.rs`,
  `src/ipc.rs` and `src/main.rs`. Include the real pure type in standalone IPC
  tests (`tests/ipc_request_limits.rs`, `tests/status_connection_budget.rs` and
  `tests/instance_lifecycle.rs`) where required. Test new request round trips,
  invalid intent rejection and unchanged legacy status bytes. Inspect every
  runtime dispatch, console relaunch and configuration rollback callsite.
- [ ] **5. Update owners and validate.** Root owns README, architecture, task map,
  validation matrix and the single smoke reason expectation. Run focused tests,
  `scripts/check.ps1 -Mode Full`, `tests/developer_checks.ps1` and
  `tests/smoke_safety.ps1` for the changed PowerShell suite. Review the exact diff
  before qualifying the source-bound release candidate. Keep logs outside docs.
- [ ] **6. Accept in the isolated desktop.** Reuse the owned offline VM only
  after verified absence of Wardoff runtime/tasks; use fresh protected/user
  fixture paths, exact source/binary hashes and owned process/task collectors.
  Test startup and live changes in both intents, repeated flips, bare Block
  restoring the pair, Allow/quit cleanup, legacy status and Doctor accuracy,
  actual `powercfg /requests` categories/counts, and unchanged power settings.
  Run same-binary smoke in standard/admin contexts. With a legacy runtime,
  verify explicit-intent rejection, unchanged state and continued control-pipe
  availability with a subsequent valid Allow/Block round trip. Exercise real tray
  Sleep/resume intent restoration if available, with exact owned-window identity;
  distinguish pure/synthetic coverage from an actual transition. Observe actual
  display/idle behavior only where the environment supports it, without input
  that resets the timer. Record unavailable hardware as not tested.
- [ ] **7. Save the accepted slice.** Record failures, skips, source/artifact
  hashes, cleanup and review. Apply existing private-content/metadata gates,
  commit only owned paths, push the existing branch and verify its remote SHA.
  Then select slice 004; do not publish a release or mutate other worktrees.

All Cargo commands use the established stable x64 MSVC/static-CRT target and
locked dependencies. Focused checks precede Full; reuse unchanged valid checks.
Stop for a material unresolved decision, actual privacy/permission boundary or
required paid resource. Reevaluate a contradicted design before continuing.
