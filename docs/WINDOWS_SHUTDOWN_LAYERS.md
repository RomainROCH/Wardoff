# Windows Shutdown Layers

> Status note: this document reflects the current implementation. Wardoff now ships Layer 1, Layer 3, Layer 4, and sleep/display blocking in the current branch. Layer 2 is still future work.

Wardoff is designed around multiple layers because Windows does not expose one universal user-space hook that covers every shutdown path. An interactive shutdown, a local `shutdown.exe` call, a Windows Update reboot, and a remote shutdown request do not all travel through the same mechanism.

## Layer summary

| Layer | Target | Current status | Core mechanism | Admin boundary | Key limitation |
| --- | --- | --- | --- | --- | --- |
| Layer 1 | Standard interactive shutdown/logoff | Implemented | `WM_QUERYENDSESSION`, `ShutdownBlockReasonCreate`, `SetProcessShutdownParameters` | No | Only covers the normal interactive shutdown path |
| Layer 2 | Local `shutdown.exe` | Not implemented yet | Planned ETW detection plus `AbortSystemShutdown`, with optional future IFEO mode | IFEO mode requires admin | Current builds do **not** intercept local `shutdown.exe`; `shutdown /t 0 /f` is not blocked |
| Layer 3 | Windows Update reboot scheduling | Implemented | Disable and re-check `Microsoft\Windows\UpdateOrchestrator\Reboot` through Task Scheduler COM | Yes | Windows may re-enable the task, so Wardoff must re-check it periodically |
| Layer 4 | Remote shutdown | Implemented | Poll `AbortSystemShutdownW(None)` approximately every 900 ms | Required privileges must be available | Only works when the shutdown still has a timeout window |

Wardoff also implements a separate sleep/hibernate/display-idle blocker via `SetThreadExecutionState(...)`. That is not a shutdown layer, but it is part of the current MVP surface.

## Why a layered design is necessary

Windows shutdown handling is fragmented on purpose:

- applications receive session-end messages such as `WM_QUERYENDSESSION`
- command-line tools like `shutdown.exe` can start a delayed or immediate shutdown
- Windows Update can queue its own restart task under Update Orchestrator
- remote callers can initiate a shutdown from another machine

If a project only handles one of these paths, users will still hit shutdowns that bypass the chosen hook. Wardoff therefore documents each path explicitly and is careful about what each layer can and cannot do.

## Layer 1 — Standard interactive shutdown blocking

This is the cleanest and most official path, and it is implemented today.

Current implementation details:

- Wardoff creates both a message-only window and a hidden top-level companion window
- the hidden top-level window owns the actual shutdown block reason because Windows does not broadcast `WM_QUERYENDSESSION` to `HWND_MESSAGE` windows
- `ShutdownBlockReasonCreate()` registers a visible reason string
- `SetProcessShutdownParameters(0x3FF, SHUTDOWN_NORETRY)` requests very high shutdown priority
- while Block mode is active, Wardoff returns `FALSE` from `WM_QUERYENDSESSION`
- blocked attempts are counted and logged to the structured JSONL sink

Expected user experience:

- Windows shows the standard "this app is preventing shutdown" style flow
- the blocker reason is visible
- the process remains alive long enough to refuse the interactive request

What Layer 1 does not solve:

- it does not stop a local `shutdown.exe /t 0 /f` that has already launched
- it does not replace Windows Update task handling
- it does not cover every remote scenario

## Layer 2 — Local `shutdown.exe`

This is still the hardest user-space problem, and it is intentionally **not** part of the current safe MVP.

Current state:

- no ETW process-start monitoring is implemented yet
- no IFEO mode is implemented yet
- no local `shutdown.exe` interception ships in the current branch

That means:

- Wardoff currently does **not** stop local `shutdown.exe` launches directly
- Wardoff currently does **not** block local `shutdown /t 0 /f`
- the limitation is real and should be communicated clearly

### Planned standard mode (later): ETW plus abort

The future standard approach remains to observe process start events from the ETW provider:

- provider: `Microsoft-Windows-Kernel-Process`
- provider GUID: `22fb2cd6-0e7b-422b-a0c7-2fad1fd0e716`
- trigger of interest: process start where the image name is `shutdown.exe`

Conceptual flow:

```text
Observe ETW ProcessStart events
→ if ProcessName == shutdown.exe
→ immediately call AbortSystemShutdown(NULL)
→ log the source, action, and result
```

This future approach can help when `shutdown.exe` started a shutdown with a timeout such as `/t 30`, because Windows still has a grace window in which `AbortSystemShutdown(...)` can win the race.

### Hard limit: `shutdown /t 0 /f`

Even with the future ETW layer, local `shutdown /t 0 /f` remains too fast once the process has already entered the forced path. In other words:

- ETW could still observe that the event happened
- the application could still log it honestly
- but it still could not promise to stop it after the fact

### Planned aggressive mode (later): IFEO

Wardoff also plans an opt-in aggressive mode based on Image File Execution Options (IFEO). Instead of reacting after `shutdown.exe` starts, IFEO intercepts the executable launch itself by setting a debugger value under:

`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\shutdown.exe`

That mode is documented separately in [IFEO_WARNING.md](IFEO_WARNING.md) because it is controversial, admin-only, and likely to draw EDR attention.

## Layer 3 — Windows Update reboot protection

Windows Update deserves its own layer because a scheduled reboot is not the same thing as an interactive shutdown request. This layer is implemented today.

Current implementation details:

- uses Task Scheduler COM interfaces such as `ITaskService` and `ITaskFolder`
- connects to the scheduler service on a dedicated worker thread with its own COM apartment
- opens `\Microsoft\Windows\UpdateOrchestrator`
- fetches the `Reboot` task
- disables it while Block mode is active
- re-checks every 5 minutes because Windows may turn it back on
- records the original enabled state and restores it on exit if Wardoff was the component that disabled it

Important boundaries:

- this is an administrator feature
- if the process is not elevated, Layer 3 is skipped and the reason is logged
- the project specifically targets `UpdateOrchestrator\Reboot`, not the older `MusNotification` approach

The broader plan still mentions future ETW monitoring of `usoclient.exe`, but that is not part of the current shipped implementation.

## Layer 4 — Remote shutdown

Remote shutdown requests can still provide a grace period. When that happens, Wardoff keeps polling `AbortSystemShutdownW(None)` on the local machine.

Current behavior:

- runs a worker loop approximately every 900 ms
- calls `AbortSystemShutdownW(None)`
- counts and logs successful interceptions
- keeps the cadence short enough to win against ordinary delayed remote shutdowns
- stops and logs clearly if Windows denies the required permission or another unexpected error occurs

What this layer can and cannot do:

- it can help when the remote shutdown still has a timeout window
- it cannot reverse a shutdown that has already crossed the no-return point
- it depends on the process having the rights Windows requires for `AbortSystemShutdownW(None)`

## Related MVP surface — Sleep, hibernate, and display idle

This is not one of the four shutdown layers, but it is part of the current MVP because users often want to prevent more than just shutdown.

Current API call:

```text
SetThreadExecutionState(
    ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
)
```

Current behavior:

- a dedicated worker thread enables the execution-state request when Block mode starts
- the request is refreshed every 30 seconds
- the request is cleared when Block mode ends
- success and failure are written to the structured JSONL log

This currently covers:

- sleep blocking
- hibernate blocking
- display idle prevention

## MVP versus later phases

Implemented in the current branch:

- Layer 1
- Layer 3
- Layer 4
- sleep/hibernate/display blocking
- tray toggle and tray power actions
- CLI surface for `--block`, `--allow`, `--status`, `--hide`, `--log`, `--tail`, and `--autostart`
- Task Scheduler autostart
- rotating structured JSONL file logging
- single-instance runtime coordination plus named-pipe IPC

Planned later scope:

- Layer 2 ETW monitoring
- opt-in IFEO mode
- Windows Event Log integration
- toast notifications
- timer-based behavior
- profiles and settings UI

The architecture is intentionally transparent about these boundaries so users know what exists, what is planned, and what remains impossible from ordinary user space.
