# Architecture

This document explains how the current Wardoff runtime is organized so a new developer can understand where startup, shutdown protection, tray behavior, logging, and IPC live.

For current shipped-status messaging and roadmap priority, start with `README.md` and `PLAN.md`; this file is the code-structure companion.

## High-level overview

Wardoff is a Windows-native Rust application built around one primary process that owns:

- the blocker state machine
- the hidden Windows message loop
- the tray surface
- the named-pipe control and read-only status servers
- the structured logging pipeline

At runtime, the primary instance coordinates several worker-backed protection layers. Secondary CLI invocations do not create a second full runtime; they forward commands to the primary instance over named-pipe IPC.

## Module map

### `src/main.rs`

Owns application bootstrap and shutdown:

- parses CLI intent
- routes explicit read-only CLI actions through a dedicated non-elevating path
- claims or defers to the primary instance
- handles elevation behavior for the default launch path
- initializes structured logging
- creates `BlockerCoordinator`
- starts the tray service when needed
- starts the named-pipe IPC server
- runs the Win32 message loop
- coordinates orderly shutdown and forced-shutdown cleanup

This is the best entry point for understanding the whole runtime.

### `src/blocker/`

Contains the layered protection logic.

- `mod.rs` — shared types plus `BlockerCoordinator`
- `shutdown.rs` — Layer 1 interactive shutdown/sign-out blocking
- `local.rs` — local `shutdown.exe` ETW detection and best-effort abort attempts
- `update.rs` — Layer 3 Update Orchestrator reboot-task protection
- `remote.rs` — Layer 4 remote shutdown abort polling
- `sleep.rs` — sleep/hibernate/display-idle blocking
- `abort.rs` — shared `AbortSystemShutdownW(None)` helpers used by local and remote shutdown code

### `src/tray/`

Contains the background tray thread and tray UI state management.

- `mod.rs` — tray thread, menu actions, delayed Explorer-ready retry logic, and TaskbarCreated-based recreation after Explorer restarts
- `icon.rs` — tray icon asset generation

### `src/logger/`

`src/logger/mod.rs` contains the structured logging system:

- human-readable stderr logging via `env_logger`
- background JSONL writer thread
- file rotation
- recent-log tail reading for CLI output

### `src/ipc.rs`

Contains the named-pipe IPC transport used by secondary commands to talk to the primary runtime.

### `src/instance.rs`

Contains the single-instance mutex logic.

### `src/autostart.rs`

Contains Task Scheduler integration for Start with Windows behavior.

### Other supporting modules

- `src/cli.rs` — argument parsing and status output
- `src/windows_util.rs` — elevation checks, relaunch, and Windows-specific helpers
- `build.rs` + `wardoff.manifest` — release-manifest embedding and requested execution level

## Core runtime object: `BlockerCoordinator`

Defined in `src/blocker/mod.rs`, `BlockerCoordinator` is the central owner for the blocker subsystems.

It holds:

- `ShutdownBlocker` for Layer 1
- `LocalShutdownBlocker` for local `shutdown.exe` handling
- `UpdateRebootBlocker` for Update Orchestrator task protection
- `RemoteShutdownBlocker` for remote shutdown polling
- `SleepBlocker` for execution-state-based power-state blocking

It also owns the current `BlockerMode`:

- `Block`
- `Allow`

### Responsibilities

`BlockerCoordinator` is responsible for:

- constructing all layer objects
- switching the application between Block and Allow mode
- activating layers in a defined order
- rolling back partial activation if one layer fails
- deactivating layers cleanly
- exposing current per-layer status
- tracking the total blocked-event count
- performing reduced forced-shutdown cleanup when Windows is already ending the session

### Activation order

When the runtime enters Block mode, `BlockerCoordinator` currently activates:

1. local-shutdown handling
2. update-reboot handling
3. remote shutdown polling
4. sleep/display blocking
5. interactive shutdown blocking

That order keeps the most visible interactive veto last, after the background helpers are already in place.

### Deactivation order

When the runtime returns to Allow mode, it:

1. disables Layer 1 first
2. stops sleep blocking
3. stops remote shutdown polling
4. stops update-task monitoring
5. stops local ETW monitoring

If a Block-mode transition fails partway through, `BlockerCoordinator` rolls back the already-started layers so the process does not remain half-armed.

## Process model and single-instance behavior

Wardoff is designed to have one primary runtime per machine session boundary enforced by a named mutex.

### Singleton mutex

Source: `src/instance.rs`

Wardoff uses:

```text
Global\WardoffInstance
```

Behavior:

- the first process to claim the mutex becomes the primary runtime
- later invocations become secondary clients
- later invocations probe the existing mutex with minimal access before treating access restrictions as fatal
- an internal retry path exists to bridge short handoff windows during elevated relaunch

This prevents multiple tray-owning, blocker-owning runtimes from competing with each other.

## IPC architecture

Source: `src/ipc.rs`

Secondary commands use two local named-pipe paths:

```text
\\.\pipe\WardoffControl
\\.\pipe\WardoffStatus
```

### Why IPC exists

Without IPC, commands like:

- `wardoff --allow`
- `wardoff --status`
- `wardoff --autostart on`

would need to start independent runtimes or fail whenever a primary instance already existed.

Instead, the secondary process:

1. detects that another instance already owns the mutex
2. connects to the appropriate local pipe
3. either sends a JSON control request or reads a status reply
4. waits for the primary runtime's response

### Request flow

The primary runtime:

- starts `IpcServer` on a background thread
- starts a separate read-only status pipe alongside the control pipe
- receives parsed `IpcRequest` values for control traffic
- wakes the UI thread with a custom `WM_APP`-based message
- handles the request on the main application thread
- returns `IpcResponse` for control commands or plain status JSON for read-only status queries

Current request types include:

- mode changes
- autostart changes
- internal server shutdown during orderly app exit

`wardoff --status` is handled specially: it prefers the dedicated read-only status pipe so a non-elevated shell can still query an elevated primary runtime without gaining access to the state-changing control pipe. State-changing secondary commands continue to use `\\.\pipe\WardoffControl`.

The control pipe is now created with explicit local-only security instead of the process-default descriptor:

- remote named-pipe clients are rejected
- the object grants control access to the current process token's user SID plus the normal elevated admin/system identities Wardoff already runs under
- a medium-integrity mandatory label allows the same interactive Windows user to send control commands to an elevated primary runtime
- if a caller still hits access denied, Wardoff maps that to a clear product message instead of returning a raw Win32 pipe error

At the moment, the inactive `wardoff --status` path is intentionally conservative but slow: when no primary runtime is present, the client first retries the read-only status pipe and then retries the control pipe before concluding that Wardoff is inactive. Each pipe path currently uses 20 attempts with a 100 ms delay, so the fully inactive path accumulates to roughly 4 seconds before returning `{"state":"inactive"}`. This current delay comes from the sequential named-pipe retry loops, not from UAC.

### Why the UI thread handles requests

Mode transitions, tray updates, and application state changes all need one central owner. Routing IPC requests back to the main application loop avoids cross-thread state mutations in the blocker and tray subsystems.

## Tray architecture

Source: `src/tray/mod.rs`

The tray runs on its own thread with its own Windows message handling and menu event processing.

### Current responsibilities

- create the tray icon and menu
- reflect Block versus Allow state
- expose tray actions back to the main runtime
- keep the autostart checkbox synchronized
- retry creation when Explorer is not ready yet
- recreate the tray icon after Explorer restarts

### Delayed startup retry

The tray thread does not assume Explorer is immediately ready.

If tray creation fails:

- it waits
- retries every 2 seconds
- logs periodic warnings while still waiting

This avoids failing the whole application just because the shell tray host is temporarily unavailable during login.

### Explorer restart recovery

The tray module also creates a hidden listener window for the registered `TaskbarCreated` message.

When Explorer restarts:

- Windows broadcasts `TaskbarCreated`
- the tray module marks recreation as pending
- the current tray controller is dropped
- the tray icon is recreated on the next pass

That is why the tray can recover after an Explorer crash or restart instead of silently disappearing forever.

## Logging architecture

Source: `src/logger/mod.rs`

Wardoff uses two logging surfaces:

- human-readable stderr logs for immediate operator/developer output
- structured JSONL logs for machine-readable operational history

### Structured logger behavior

The structured logger:

- starts a dedicated writer thread
- writes newline-delimited JSON records
- stores logs under `%LOCALAPPDATA%\Wardoff\logs\`
- rotates at 5 MiB
- keeps up to 3 log files

Subsystems log through shared helper functions so events from tray, IPC, application startup, and blocker layers land in one consistent sink.

## Update Orchestrator and autostart Task Scheduler use

Task Scheduler appears in two different parts of the architecture:

### `src/blocker/update.rs`

Uses Task Scheduler COM APIs to control:

```text
\Microsoft\Windows\UpdateOrchestrator\Reboot
```

This is Layer 3 protection and runs only while Block mode is active.

If the `\Microsoft\Windows\UpdateOrchestrator` folder or `Reboot` task does not exist on the machine, Layer 3 exits cleanly. That is expected on some Windows editions, including LTSC-style installations where the reboot task may be absent; it is a normal skip, not a failure.

### `src/autostart.rs`

Uses Task Scheduler COM APIs to manage:

```text
\Wardoff
```

This is not a shutdown layer. It is the Start with Windows feature.

Both modules use COM and Task Scheduler APIs, but they serve different purposes.

## Elevation and release manifest behavior

Release-elevation behavior is controlled by:

- `build.rs`
- `wardoff.manifest`

Current design:

- in release builds on Windows, `build.rs` uses `winres` to compile `wardoff.manifest` into the normal Win32 manifest resource
- `wardoff.manifest` requests `asInvoker`, so Windows does not force elevation before CLI argument parsing
- clap still short-circuits `--help` and `--version` locally before Wardoff reaches any runtime bootstrap logic
- `src/main.rs` now routes `--status` and `--log --tail N` through an explicit read-only dispatch before calling `prepare_default_launch(...)` or any default-runtime bootstrap path
- `src/main.rs` calls `prepare_default_launch(...)` only when a no-arguments launch is bootstrapping a new primary runtime and selectively relaunches through `relaunch_self_elevated()` when admin-only protections are desired
- release builds now run as a console-friendly executable so direct shell invocations of read-only commands keep normal stdout/stderr/exit-code behavior
- when a long-lived primary runtime would start from an inherited console, `src/main.rs` relaunches that runtime through `relaunch_self_detached()` and exits the shell-facing process cleanly instead of blocking the console forever
- when a long-lived primary runtime starts with a dedicated standalone console instead of an inherited shell console, `src/main.rs` hides and frees that console before the tray/runtime loop settles in
- explicit CLI commands such as `--help`, `--version`, `--status`, and `--log --tail N` stay non-elevated unless the command itself later checks for administrator rights
- `--status` reaches an elevated primary runtime through the dedicated read-only status pipe instead of the bidirectional control pipe
- state-changing commands such as `--block`, `--allow`, `--hide`, and `--autostart on|off` remain on the normal runtime/control-pipe path rather than the read-only status path
- `tests/smoke_test.ps1` now includes a release-binary guard that reads the embedded manifest resource directly and asserts `requestedExecutionLevel level="asInvoker"`

That design preserves scriptable non-elevated CLI usage while still letting the default runtime path request elevation so admin-only protections such as Update Orchestrator task control can be available.

The code still handles non-elevated scenarios explicitly instead of pretending they succeeded.

## Shutdown and cleanup flow

The cleanup story is split between orderly shutdown and forced session shutdown.

### Orderly shutdown

In normal exit paths, `src/main.rs` coordinates:

- rejecting pending IPC work
- shutting down the IPC server
- deactivating blocker layers
- stopping the tray thread
- flushing and stopping structured logging

### Forced shutdown cleanup

Layer 1 receives `WM_ENDSESSION` through `src/blocker/shutdown.rs`.

That triggers a callback into `src/main.rs`, where the application:

- marks forced cleanup as completed
- asks `BlockerCoordinator` to run reduced cleanup
- stops long-running background protections that should not fight Windows once shutdown is already committed

This path exists so the app can release resources cleanly when the user or system forces shutdown despite Block mode.

## Architectural boundaries to keep in mind

For current documentation and review, treat these as supported, visible runtime features:

- Layer 1 interactive blocking
- Layer 3 update reboot-task protection
- Layer 4 remote abort polling
- sleep/hibernate/display-idle blocking
- tray UI
- CLI plus IPC control
- structured JSONL logging
- single-instance coordination
- autostart task management

Be more cautious with:

- local ETW-based `shutdown.exe` handling in `src/blocker/local.rs`

That code exists and is wired in, but current top-level user-facing MVP docs deliberately do not market it as a guaranteed flagship feature because the hardest local forced-shutdown race is still not honestly solved.

## Suggested reading order for new developers

If you are new to the codebase, read in this order:

1. `src/main.rs`
2. `src/blocker/mod.rs`
3. `src/blocker/shutdown.rs`
4. `src/blocker/update.rs`
5. `src/blocker/remote.rs`
6. `src/blocker/sleep.rs`
7. `src/ipc.rs`
8. `src/tray/mod.rs`
9. `src/logger/mod.rs`
10. `src/autostart.rs`

That path gives the fastest route to understanding how the runtime is assembled and how the major subsystems cooperate.
