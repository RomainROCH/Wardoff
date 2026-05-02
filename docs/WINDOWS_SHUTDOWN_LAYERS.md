# Windows Shutdown Layers

> Status note: this document describes the code currently present in the repository. It also distinguishes between what is implemented in source and what the project currently treats as part of the conservative, documented MVP surface.

Wardoff uses multiple layers because Windows shutdown, sign-out, reboot, sleep, and update-triggered restart paths do not all pass through one API. Different code paths need different handling, and some paths remain inherently race-prone from user space.

## Layer summary

| Layer | Purpose | Implementation status in source | Current MVP positioning | Primary source files | Key limitation |
| --- | --- | --- | --- | --- | --- |
| Layer 1 | Interactive shutdown and sign-out blocking | Implemented | Supported MVP behavior | `src/blocker/shutdown.rs`, `src/blocker/mod.rs` | Only covers the standard session-end path |
| Layer 2 | Local `shutdown.exe` detection and best-effort abort | Implemented in code | Present in repo, but not promoted as a supported 0.1.0 headline feature | `src/blocker/local.rs`, `src/blocker/abort.rs`, `src/blocker/mod.rs` | Cannot honestly promise to stop `shutdown /t 0 /f` |
| Layer 3 | Windows Update reboot-task protection | Implemented | Supported MVP behavior | `src/blocker/update.rs`, `src/blocker/mod.rs` | Requires elevation and periodic re-checks |
| Layer 4 | Remote shutdown abort polling | Implemented | Supported MVP behavior | `src/blocker/remote.rs`, `src/blocker/abort.rs`, `src/blocker/mod.rs` | Only works while Windows still exposes an abortable window |
| Separate power-state guard | Sleep, hibernate, and display-idle prevention | Implemented | Supported MVP behavior | `src/blocker/sleep.rs`, `src/blocker/mod.rs` | Uses execution-state requests, not a shutdown veto |

`src/blocker/mod.rs` wires the runtime together through `BlockerCoordinator`, which activates Layer 2, Layer 3, Layer 4, sleep blocking, and finally Layer 1 when entering Block mode, then tears them down in reverse order when returning to Allow mode.

## Why Wardoff needs layers

Windows treats these as different categories of behavior:

- interactive shutdown and sign-out broadcast `WM_QUERYENDSESSION`
- local command-line shutdowns can start through `shutdown.exe`
- Windows Update uses scheduled-task and service-driven restart flows
- remote shutdown requests can create a pending shutdown that another process may still abort
- sleep and display-idle prevention are handled through execution-state hints instead of shutdown negotiation

Because of that split, one mechanism is never enough.

## Layer 1 — Interactive shutdown and sign-out

Layer 1 is the clean, documented Windows path for stopping a normal user-initiated shutdown or sign-out.

### What the implementation does

Source: `src/blocker/shutdown.rs`

- creates both a message-only window and a hidden top-level session window
- registers the shutdown-block reason on the hidden top-level window because `HWND_MESSAGE` windows do not receive `WM_QUERYENDSESSION`
- registers suspend/resume notifications on the hidden top-level session window so tray-initiated Sleep/Hibernate can restore Block mode after wake or resume
- calls `SetProcessShutdownParameters(0x3FF, SHUTDOWN_NORETRY)` during setup
- calls `ShutdownBlockReasonCreate(...)` when Block mode is enabled
- returns `FALSE` from `WM_QUERYENDSESSION` while Layer 1 is active
- counts successful Layer 1 interceptions through `record_blocked_event()`
- supports a forced-shutdown cleanup callback through `WM_ENDSESSION`

### Why the two-window design exists

The plan-level wording around a message-only window is incomplete by itself. The code shows the actual design:

- `message_window` exists for the hidden runtime plumbing
- `session_window` is the hidden top-level window that receives `WM_QUERYENDSESSION`
- that same hidden `session_window` also registers for suspend/resume notifications because modern Windows does not reliably broadcast the needed `WM_POWERBROADCAST` suspend/resume events to top-level windows unless the process opts in

That distinction matters because Windows does not broadcast session-end messages to message-only windows.

### Runtime behavior

Wiring:

- created by `BlockerCoordinator::new(...)` in `src/blocker/mod.rs`
- activated in `BlockerCoordinator::activate_block_mode(...)`
- deactivated in `BlockerCoordinator::activate_allow_mode(...)`
- forced-shutdown cleanup is triggered from `src/main.rs` through `blocker::shutdown::set_end_session_cleanup_callback(...)`

What users should expect:

- standard Windows "this app is preventing shutdown" behavior
- visible block reason text
- normal interactive shutdown and sign-out attempts can be refused while Block mode is active
- if Sleep or Hibernate is launched from the tray while Wardoff is already in Block mode, Wardoff temporarily switches to Allow so the power transition can proceed, then restores Block after wake or resume

What Layer 1 does **not** solve:

- forced local `shutdown.exe` paths
- Update Orchestrator task handling
- remote shutdowns that are already past the abortable phase

## Layer 2 — Local `shutdown.exe`

Layer 2 is the hardest path to document responsibly.

### Current repository reality

Source: `src/blocker/local.rs`

The repository does contain a real Layer 2 implementation:

- `LocalShutdownBlocker` starts only while Block mode is active
- it first verifies whether `AbortSystemShutdownW(None)` is even usable through `preflight_shutdown_abort_capability()` in `src/blocker/abort.rs`
- it creates a real-time ETW session for `Microsoft-Windows-Kernel-Process`
- it watches process-start events for `shutdown.exe`
- when it detects `shutdown.exe`, it immediately calls `AbortSystemShutdownW(None)`
- it increments the blocked counter only when the abort actually succeeds
- if capability checks or ETW setup fail, it logs the reason and leaves Layer 2 inactive without crashing the runtime

Relevant source points:

- ETW provider GUID and event filtering: `src/blocker/local.rs`
- abort helper shared with Layer 4: `src/blocker/abort.rs`
- activation from Block mode: `src/blocker/mod.rs`

### Why the docs stay conservative

Even though this code exists, Wardoff's top-level user-facing MVP docs intentionally do **not** present ETW-based local-shutdown interception as a supported 0.1.0 promise.

That conservative boundary is intentional because:

- ETW detection is reactive, not pre-launch interception
- success still depends on a shutdown still being abortable
- `shutdown /t 0 /f` remains a real race that the project should not market as solved

So the honest wording is:

- Layer 2 exists in the repository
- it is wired into Block mode
- it may abort some local shutdown requests
- it is **not** currently marketed as a guaranteed or headline MVP feature

### What it can and cannot do

It can sometimes help when:

- `shutdown.exe` starts a shutdown with a timeout window
- the process has the rights needed for `AbortSystemShutdownW(None)`
- ETW is available and the abort wins the race

It cannot honestly promise:

- blocking `shutdown /t 0 /f`
- blocking every local shutdown request
- behaving like a pre-execution hook

## Layer 3 — Windows Update reboot-task protection

Layer 3 addresses the scheduled reboot task used by modern Windows Update orchestration.

### What the implementation does

Source: `src/blocker/update.rs`

- checks whether the process is elevated before starting
- starts a dedicated worker thread with its own COM apartment
- connects to Task Scheduler through `ITaskService`
- opens `\Microsoft\Windows\UpdateOrchestrator`
- looks up the `Reboot` task
- disables the task when needed
- re-checks it every 5 minutes through `recheck_interval()`
- restores the original enabled state on exit if Wardoff was the component that disabled it

### Important behavioral details

- if the process is not elevated, Layer 3 logs that it was skipped and continues
- if the task or folder does not exist on the machine, Layer 3 logs and exits cleanly
- if Windows re-enables the task during Block mode, Layer 3 disables it again on the next poll

This matches the implementation reality more closely than older planning language about broader update interception.

### Boundaries

Layer 3 is specifically about:

- `\Microsoft\Windows\UpdateOrchestrator\Reboot`

It is **not** currently:

- a general Windows Update ETW monitor
- a `usoclient.exe` interception feature
- a blanket promise against all update-triggered restarts

## Layer 4 — Remote shutdown abort polling

Layer 4 is a polling loop around `AbortSystemShutdownW(None)` for remote shutdown scenarios that still have time to cancel.

### What the implementation does

Source: `src/blocker/remote.rs`

- starts a worker thread while Block mode is active
- calls `AbortSystemShutdownW(None)` immediately once at startup
- keeps polling every 900 ms via `remote_abort_interval()`
- counts successful aborts as blocked events
- stops the layer if Windows denies the required privilege
- logs unexpected failures and exits the worker cleanly

### Boundaries

Layer 4 helps only when:

- a shutdown is pending
- Windows still allows an abort
- the process has the required rights

It does not rewind a shutdown after the no-return point.

## Separate power-state guard — Sleep, hibernate, and display idle

This is not a shutdown layer, but it is a real part of the current runtime.

Source: `src/blocker/sleep.rs`

- starts a dedicated worker thread in Block mode
- calls `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)`
- refreshes the request every 30 seconds
- clears the request with `SetThreadExecutionState(ES_CONTINUOUS)` when Block mode ends
- logs activation, refresh failures, and cleanup failures to the JSONL logger

Current scope:

- sleep blocking
- hibernate blocking
- display-idle prevention

## Coordinator and lifecycle wiring

The layer relationship is easiest to understand from these files:

- `src/blocker/mod.rs` — owns `BlockerCoordinator`, mode transitions, and blocked-event counting
- `src/main.rs` — bootstrap, Win32 message loop, IPC processing, tray action handling, and forced-shutdown cleanup
- `src/logger/mod.rs` — structured JSONL event sink used by all layers

When Wardoff enters Block mode, `BlockerCoordinator` currently attempts to activate:

1. Layer 2 local shutdown monitoring
2. Layer 3 Update Orchestrator protection
3. Layer 4 remote abort polling
4. sleep/display blocking
5. Layer 1 interactive shutdown blocking

If activation fails, the coordinator rolls back the partial startup to avoid leaving the runtime in a mixed state.

When Windows forces session shutdown anyway, `src/main.rs` invokes `forced_shutdown_cleanup()` so long-running workers can stop without trying to keep the machine blocked during final teardown.

## MVP boundary versus repository contents

Supported and safe to describe as current user-facing MVP behavior:

- Layer 1 interactive shutdown/sign-out blocking
- Layer 3 Update Orchestrator reboot-task protection
- Layer 4 remote shutdown abort polling
- sleep, hibernate, and display-idle prevention

Present in source, but documented more cautiously:

- Layer 2 ETW-based local `shutdown.exe` detection and abort attempts

Planned later and not implemented:

- aggressive IFEO interception for `shutdown.exe`
- Windows Event Log integration
- toast notifications
- timers
- profiles
- settings UI

That split keeps the docs honest: the codebase is allowed to contain work beyond the supported MVP surface, but public-facing claims should stay conservative until the behavior is intentionally documented as shipped.
