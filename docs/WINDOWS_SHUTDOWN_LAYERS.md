# Windows Shutdown Layers

> Status note: this document describes Wardoff's planned architecture. The current repository does not yet ship a working runtime blocker.

Wardoff is designed around multiple layers because Windows does not expose one universal user-space hook that covers every shutdown path. An interactive shutdown, a local `shutdown.exe` call, a Windows Update reboot, and a remote shutdown request do not all travel through the same mechanism.

## Layer summary

| Layer | Target | Planned phase | Core mechanism | Admin boundary | Key limitation |
| --- | --- | --- | --- | --- | --- |
| Layer 1 | Standard interactive shutdown/logoff | MVP | `WM_QUERYENDSESSION`, `ShutdownBlockReasonCreate`, `SetProcessShutdownParameters` | No | Only covers the normal interactive shutdown path |
| Layer 2 | Local `shutdown.exe` | v1.0 | ETW detection plus `AbortSystemShutdown`, with optional future IFEO mode | IFEO mode requires admin | `shutdown /t 0 /f` is too fast for ETW plus abort once the process is already running |
| Layer 3 | Windows Update reboot scheduling | MVP | Disable and re-check `Microsoft\Windows\UpdateOrchestrator\Reboot` through Task Scheduler COM | Yes | Windows may re-enable the task, so the app must re-check it periodically |
| Layer 4 | Remote shutdown | MVP | Poll `AbortSystemShutdown(NULL)` approximately every 900 ms | Windows permission failures must be surfaced clearly | Only works when the shutdown still has a timeout window |

Wardoff also plans a separate sleep/hibernate/display-idle blocker via `SetThreadExecutionState(...)`. That is not a shutdown layer, but it is part of the MVP surface.

## Why a layered design is necessary

Windows shutdown handling is fragmented on purpose:

- applications receive session-end messages such as `WM_QUERYENDSESSION`
- command-line tools like `shutdown.exe` can start a delayed or immediate shutdown
- Windows Update can queue its own restart task under Update Orchestrator
- remote callers can initiate a shutdown from another machine

If a project only handles one of these paths, users will still hit shutdowns that bypass the chosen hook. Wardoff therefore documents each path explicitly and is careful about what each layer can and cannot do.

## Layer 1 — Standard interactive shutdown blocking

This is the cleanest and most official path. A process creates a message-only window (`HWND_MESSAGE`), registers a visible blocker reason, and responds to the shutdown query message.

Planned ingredients:

- message-only window to receive `WM_QUERYENDSESSION`
- `ShutdownBlockReasonCreate()` so Windows can show a user-readable reason
- `SetProcessShutdownParameters(0x3FF, SHUTDOWN_NORETRY)` to request very high shutdown priority
- return `FALSE` when `WM_QUERYENDSESSION` is received

Conceptual flow:

```rust
// Conceptual example only, not current repository code.
let hwnd = create_message_only_window();
ShutdownBlockReasonCreate(hwnd, w!("Wardoff is blocking shutdown"));
SetProcessShutdownParameters(0x03FF, SHUTDOWN_NORETRY);

match msg {
    WM_QUERYENDSESSION => {
        // Record the attempt and refuse the shutdown.
        return FALSE;
    }
    _ => {}
}
```

Expected user experience:

- Windows shows the standard "this app is preventing shutdown" style screen
- the blocker reason is visible
- the process remains alive long enough to refuse the interactive request

What Layer 1 does not solve:

- it does not stop a local `shutdown.exe /t 0 /f` that has already launched
- it does not replace Windows Update task handling
- it does not cover every remote scenario

## Layer 2 — Local `shutdown.exe`

This is the hardest user-space problem and is intentionally not part of the safe MVP.

### Planned standard mode (v1.0): ETW plus abort

The future standard approach is to observe process start events from the ETW provider:

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

This approach can help when `shutdown.exe` started a shutdown with a timeout such as `/t 30`, because Windows still has a grace window in which `AbortSystemShutdown(...)` can win the race.

### Hard limit: `shutdown /t 0 /f`

If the local command is `shutdown /t 0 /f`, user-space observation is too late once the process has already entered the fast, forced path. In other words:

- ETW can still see that the event happened
- the application can still log it honestly
- but it cannot promise to stop it after the fact

That limitation must be documented, not hidden.

### Planned aggressive mode (later): IFEO

Wardoff also plans an opt-in aggressive mode based on Image File Execution Options (IFEO). Instead of reacting after `shutdown.exe` starts, IFEO intercepts the executable launch itself by setting a debugger value under:

`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\shutdown.exe`

That mode is documented separately in [IFEO_WARNING.md](IFEO_WARNING.md) because it is controversial, admin-only, and likely to draw EDR attention.

## Layer 3 — Windows Update reboot protection

Windows Update deserves its own layer because a scheduled reboot is not the same thing as an interactive shutdown request.

The technical target from the plan is the Task Scheduler entry:

- task path: `Microsoft\Windows\UpdateOrchestrator\Reboot`

Planned API surface:

- COM Task Scheduler interfaces such as `ITaskService` and `ITaskFolder`
- connect to the scheduler service
- open `\Microsoft\Windows\UpdateOrchestrator`
- fetch the `Reboot` task
- disable it while Block mode is active
- re-check every 5 minutes because Windows may turn it back on

Conceptual flow:

```text
CoInitializeEx(...)
→ ITaskService::Connect(...)
→ GetFolder("\\Microsoft\\Windows\\UpdateOrchestrator")
→ GetTask("Reboot")
→ put_Enabled(VARIANT_FALSE)
→ re-check every 5 minutes while Block mode is active
```

Important boundaries:

- this is an administrator feature
- failure must be explicit and logged
- the project specifically targets `UpdateOrchestrator\Reboot`, not the older `MusNotification` approach

The broader plan also mentions future monitoring of `usoclient.exe`, but the current MVP documentation stays focused on Task Scheduler control and periodic re-disable behavior.

## Layer 4 — Remote shutdown

Remote shutdown requests can still provide a grace period. When that happens, Wardoff plans to keep polling `AbortSystemShutdown(NULL)` on the local machine.

Planned behavior:

- run a loop every approximately 900 ms
- call `AbortSystemShutdown(NULL)`
- record when the call succeeds or fails
- keep the cadence short enough to win against ordinary delayed remote shutdowns

Conceptual flow:

```rust
loop {
    let _ = AbortSystemShutdown(NULL);
    sleep(Duration::from_millis(900));
}
```

What this layer can and cannot do:

- it can help when the remote shutdown still has a timeout window
- it cannot reverse a shutdown that has already crossed the no-return point
- it should log the outcome instead of pretending success

## Related MVP surface — Sleep, hibernate, and display idle

This is not one of the four shutdown layers, but it is part of the planned MVP because users often want to prevent more than just shutdown.

Planned API call:

```text
SetThreadExecutionState(
    ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
)
```

This is the planned mechanism for:

- sleep blocking
- hibernate blocking
- display idle prevention
- the broader "don't go away while I am busy" experience

It should remain independently controllable from shutdown blocking because the user may want one without the other.

## MVP versus later phases

Planned MVP scope:

- Layer 1
- Layer 3
- Layer 4
- sleep/hibernate/display blocking
- tray toggle, basic CLI, Task Scheduler autostart, and file logging

Planned later scope:

- Layer 2 ETW monitoring
- opt-in IFEO mode
- Windows Event Log integration
- toast notifications
- timer-based behavior

The architecture is intentionally transparent about these boundaries so users know what exists, what is planned, and what remains impossible from ordinary user space.
