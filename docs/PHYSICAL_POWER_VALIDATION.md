# Physical power validation kit

This kit prepares manual evidence for the idle-power contract in the
[MVP validation matrix](MVP_VALIDATION_MATRIX.md). It does not perform the tests.
VM acceptance does not establish physical S3, monitor, Hibernate or Modern
Standby acceptance.

## Passive preparation

`tests/physical_power/Prepare-PhysicalPowerValidation.ps1` verifies a supplied
binary's SHA-256, copies it into a fresh local directory and records read-only
power settings. It creates a provenance manifest and a pending results template.
It does not build or launch Wardoff, change settings, schedule work or initiate
a power transition. Existing files are not overwritten.

Example, from the repository root, with real paths and provenance supplied by
the operator:

```powershell
.\tests\physical_power\Prepare-PhysicalPowerValidation.ps1 `
    -WardoffExecutable 'C:\TestArtifacts\wardoff.exe' `
    -ExpectedSha256 '<verified 64-character SHA-256>' `
    -OutputDirectory 'C:\TestArtifacts\physical-power-run' `
    -ApplicationSourceCommit '<binary source commit>' `
    -AcceptedIntegrationCommit '<accepted application commit>'
```

The two commits may differ only when the artifact's application source has been
verified against the accepted application source. A matching supplied hash alone
does not prove that relationship. Keep the build and acceptance evidence with
the kit. Preparation is complete when the kit is present and verified; every
physical result remains pending.

For focused verification of the preparation tools, run
`tests/physical_power/Test-PassivePreparation.ps1`. It uses a text artifact and
mocked diagnostics in a temporary directory; it does not run Wardoff, invoke
real power diagnostics or query the event log.

`Collect-PhysicalPowerCheckpoint.ps1` is also read-only. It verifies the kit
binary without executing it, captures power settings, requests and overrides,
and records recent power-event identifiers and timestamps where accessible.
Failures to read a diagnostic are preserved, not converted into passes. Capture
files can contain other applications' names or paths; keep the entire kit local
and review/redact evidence before sharing it.

```powershell
# Use a fresh label for each capture. This does not start a test.
.\Collect-PhysicalPowerCheckpoint.ps1 -KitDirectory . -Label 'before-tests'
```

## Conditions for a future test window

Run the remaining instructions only during an explicitly arranged maintenance
window. Sleep stops work on this PC, including unrelated Codex agents. Save work,
coordinate with the owners of that work and have a person present to observe the
monitor and wake the PC. Preparation does not authorize stopping their processes.
The kit neither discovers nor manages other agents.

Use an already suitable test setup. Record Windows build, hardware, power source,
the physical monitor in use, the existing sleep/display timeouts and the means
of waking the PC. Do not enable Hibernate, change the power plan, add request
overrides or change firmware merely to make a result pass. If a required timeout
is disabled, a state is unavailable, another application prevents sleep, or
remote/input activity prevents a clean idle interval, record the constraint and
defer that case. Resolve setup questions separately before another test window.

An S3 PC is not a Modern Standby PC. Test S0 Low Power Idle on native supporting
hardware. Test Hibernate only where it is already enabled and available. Do not
count either missing capability as a pass. On a laptop, do not approach critical
battery levels to conduct a test.

Use a non-elevated Wardoff runtime for idle-request acquisition where possible.
An elevated console may be needed for `powercfg /requests`; that diagnostic
requirement is separate from the runtime. Record the actual privilege level.
Do not toggle Start with Windows or test unrelated protection layers in this run.

## Execution and evidence

These are manual steps for the future window. No preparation script performs
them. From the kit directory, start `wardoff.exe --allow` when no Wardoff runtime
is already active. Record the new test runtime's PID, exact executable path and
process start time for cleanup. If a pre-existing runtime is present, defer rather
than taking control of it. Keep the visible tray for the normal Quit check.

During the window, use `wardoff.exe --status` for state/layer evidence and
`wardoff.exe --log --tail 30` for recent logs. Save their outputs locally with UTC
timestamps. Mode changes use `wardoff.exe --block` and `wardoff.exe --allow`.
Those commands can start a runtime if none exists, so use them only after
establishing ownership of the test runtime. Record new PIDs if it is restarted.

Run the checkpoint collector before each idle interval and after observation or
wake. Use fresh labels such as `block-before`, `block-after`, `wake-after`,
`allow-before`, `allow-after`, `quit-after` and `kill-after`. Inspect exit codes in
the captures. An unavailable request listing leaves request evidence pending.

For idle observations, note the last input time, expected deadline and actual
elapsed time. Wait beyond the relevant configured timeout, allowing for normal
Windows timer scheduling. Do not touch the keyboard/mouse, use a remote desktop,
run interactive diagnostics or poll Wardoff while the idle interval is in
progress. A person should observe the physical screen without generating input.
Record how sleep and resume were identified; an event identifier or a visible
request alone does not prove the physical transition.

| Case | Manual action | Required evidence and expected behavior |
| --- | --- | --- |
| S3-1 | Enter Block; wait beyond the existing automatic sleep timeout | PC remains awake. Record elapsed idle time and physical observation, plus system/display requests before and after. |
| S3-2 | In Block, wait beyond the existing automatic display timeout | Physical monitor remains active. Record observation separately from request visibility. S3-1 and S3-2 may share one sufficiently long idle interval. |
| S3-3 | In Block, choose Sleep from the Windows Start power menu | PC actually enters S3. Explicit Windows sleep is expected to proceed. Record the transition and how the PC was woken. |
| S3-4 | Resume the same Block runtime | State stays Block, `layers.sleep=true`, and both requests are present again. Then repeat an idle interval beyond the sleep/display timeouts to verify actual protection after wake. |
| S3-5 | Switch the same runtime to Allow; wait without input | Wardoff stays alive with `layers.sleep=false`; its requests disappear. Existing automatic display-off and sleep work. Observe both where the existing timeout ordering allows it; otherwise record which observation needs another suitable setup. |
| S3-6 | Capture requests in Block and Allow | Wardoff appears under SYSTEM and DISPLAY in Block with reason `Wardoff Block mode: prevent idle sleep and automatic display timeout.`; disappears in Allow. No extra residual Wardoff request remains. |
| S3-7 | Return the owned runtime to Block, then Quit directly from its tray | Owned process exits and its requests disappear, without first switching to Allow. |
| S3-8 | Start a fresh owned test runtime, enter Block, then terminate that exact PID | Immediately before termination, recheck PID, exact executable path and process start time against the owned runtime record. Abort on a mismatch, an exited process or an unreadable identity. Only that verified process is terminated; its requests disappear after abrupt exit. Never kill all processes by name. Record the identity and before/after request evidence. |

Keep `manifest.json` unchanged: its `physicalTestsExecuted=false` records the
state at preparation time. In `results.json`, set `physicalTestsExecuted=true`
when the first physical test is actually attempted, even if that attempt fails.
Keep it false if all cases are deferred. Change each case from `pending` to
`pass` or `fail` only after observation. Use `not-tested` with a reason when
capabilities, setup or scheduling prevent an attempt; no transition is needed
to document that deferral. Add evidence filenames, UTC times, durations and
explanatory notes. Do not set a pass from capability detection,
synthetic resume messages or request visibility alone. Repeat failed or
ambiguous cases only after identifying the confounding condition.

## Other supported environments and resume paths

On a prepared Hibernate-capable machine, repeat explicit Windows Hibernate from
Block and verify actual S4/resume, the surviving Block state and renewed request
pair. Repeat an idle interval after resume. Wardoff does not promise a general
Hibernate veto. Record these as the two Hibernate rows in `results.json`.

On native Modern Standby hardware, repeat applicable idle, explicit-sleep,
resume and release checks on AC and DC. Record the complete observation window,
including beyond the configured sleep timeout plus five minutes on DC, and an
AC/DC change where applicable. Windows limits system-required requests on DC;
expiry at that boundary is not evidence of a universal sleep veto defect. Never
refresh requests in a loop to defeat that limit. Record the two Modern Standby
rows with the applicable contract and observed behavior, not a blanket promise
that the PC must remain awake indefinitely.

For broader matrix coverage, separately test tray Sleep/Hibernate from Block
and Allow. A tray action from Block temporarily enters Allow and restores Block
after wake with `block_restored_after_wake`; an action from Allow stays Allow.
Record existing Sleep-button/lid behavior and unattended/maintenance wake only
where the setup supports them. These additional observations do not replace the
eight requested S3 cases.

## Completion and recovery

Capture a final checkpoint after the owned test process exits. Compare the active
scheme and settings with the baseline, and compare overrides with the
`before-tests` checkpoint. The kit and Wardoff's idle-power layer should not
alter them. If unrelated activity changed the environment, record the difference
and investigate ownership before any restoration; do not overwrite another
owner's changes. The kit has no automatic restoration action because it makes
no power-setting changes.

Confirm no owned test process or Wardoff request remains, leave the machine
awake, and notify the operator that normal work can resume. Retain failures,
skips and untested states in the final record. Request an independent review of
the evidence before claiming physical acceptance. Shutdown/reboot, forced
critical-battery tests and unrelated autostart/update changes are outside this
kit's scope.
