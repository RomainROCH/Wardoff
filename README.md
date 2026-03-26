# Wardoff

*The first modern open-source tool for taking control of Windows shutdowns.*

![MIT License](https://img.shields.io/badge/license-MIT-green)
![Rust](https://img.shields.io/badge/language-Rust-orange)
![Windows](https://img.shields.io/badge/platform-Windows%2010%2B-blue)

Unexpected shutdowns and reboots still interrupt gaming sessions, long-running builds, remote administration, and overnight workloads. Existing utilities are often closed-source, abandoned, or focused mainly on sleep prevention rather than transparent shutdown control. Wardoff is a Windows-native Rust project that tries to be explicit about both its current capabilities and its hard limits.

This repository is no longer just a planning scaffold. The current codebase implements a working MVP-level runtime with a tray surface, CLI, structured logging, single-instance coordination, and the safe user-space shutdown layers that fit the current branch scope.

## Features MVP

Implemented in this repository today:

- working Windows tray/runtime application with Block and Allow state management
- CLI support for:
  - `--block`
  - `--allow`
  - `--status`
  - `--hide`
  - `--log`
  - `--tail`
  - `--autostart on|off`
- Layer 1 interactive shutdown blocking via `WM_QUERYENDSESSION`, `ShutdownBlockReasonCreate()`, and `SetProcessShutdownParameters()`
- Layer 2 standard-mode local `shutdown.exe` detection via ETW process-start monitoring plus best-effort `AbortSystemShutdownW(None)` attempts
- Layer 3 Windows Update reboot-task protection for `Microsoft\Windows\UpdateOrchestrator\Reboot`
- Layer 4 remote shutdown abort polling via `AbortSystemShutdownW(None)`
- sleep, hibernate, and display-idle blocking via `SetThreadExecutionState(...)`
- rotating structured JSONL logs under `%LOCALAPPDATA%\Wardoff\logs\`
- Task Scheduler autostart management
- single-instance enforcement plus named-pipe IPC so secondary CLI invocations can control the primary runtime
- tray power actions for Shutdown, Reboot, Sleep, Hibernate, and Quit

## Planned features v1.0

These items are still missing in the current shipped implementation:

- opt-in aggressive IFEO mode for earlier local `shutdown.exe` interception
- Windows Event Log integration
- native Windows toast notifications
- timer-based blocking windows
- profiles and a settings window
- packaging and distribution polish via GitHub Releases, Winget, Scoop, and Chocolatey

## Honest limitations

- **Current Wardoff still does not promise to block local `shutdown /t 0 /f`.** Layer 2 now watches ETW process-start events for local `shutdown.exe` launches and immediately tries `AbortSystemShutdownW(None)`, but a forced zero-second local shutdown can still outrun a user-space abort attempt.
- The current branch ships Layer 1, Layer 2 standard mode, Layer 3, Layer 4, and sleep/display blocking. It does **not** yet ship aggressive IFEO interception.
- Windows Update protection is tied to the `Microsoft\Windows\UpdateOrchestrator\Reboot` scheduled task and requires administrator rights.
- Remote shutdown abort logic only helps while Windows still exposes a shutdown timeout window and the process has the rights needed for `AbortSystemShutdownW(None)`.
- Default no-argument startup is designed to request elevation so the full safe MVP can run. Some explicit CLI paths can still run without elevation, but elevated-only features may then be skipped and logged as unavailable.
- There is no Windows Event Log integration, toast UI, settings window, timer/profile system, or polished installer/distribution flow yet.

## How it works

Wardoff is designed as a layered Windows-only tool because no single user-space API covers every shutdown origin. The architecture reference is documented in [docs/WINDOWS_SHUTDOWN_LAYERS.md](docs/WINDOWS_SHUTDOWN_LAYERS.md):

1. Layer 1: standard interactive shutdown blocking
2. Layer 2: local `shutdown.exe` handling (standard mode)
3. Layer 3: Windows Update reboot task control
4. Layer 4: remote shutdown abort polling

Wardoff also implements a separate power-state blocker for sleep, hibernate, and display idle prevention.

## Installation

Wardoff is currently source-first. There is not yet a polished release/distribution pipeline.

Build from source on Windows with the MSVC toolchain:

```powershell
cargo build --release
```

The resulting binary is typically:

```text
target\release\wardoff.exe
```

## Usage

### Default launch

Running `wardoff` with no arguments starts the primary runtime in **Block** mode with a visible tray icon. On the default startup path, Wardoff may relaunch itself elevated so Layer 3 and other privileged functionality can operate when available.

### CLI

```powershell
wardoff
wardoff --block
wardoff --allow
wardoff --status
wardoff --hide
wardoff --log
wardoff --log --tail 50
wardoff --autostart on
wardoff --autostart off
```

Command behavior in the current implementation:

- `wardoff` starts the tray runtime in Block mode
- `--block` switches the primary instance to Block mode or starts a headless Block-mode runtime
- `--allow` switches the primary instance to Allow mode or starts a visible Allow-mode runtime
- `--status` prints compact JSON describing the current state, active layers, uptime, and blocked count
- `--hide` starts Block mode with the tray icon created but hidden
- `--log` prints recent structured JSONL records
- `--tail N` changes how many recent records `--log` prints
- `--autostart on|off` creates, updates, or removes the current-user scheduled task used for Start with Windows

Example `--status` shape:

```json
{"state":"block","layers":{"shutdown":true,"local_shutdown":true,"update":true,"remote":true,"sleep":true},"uptime_seconds":42,"blocked_count":3}
```

If no primary runtime is active, `--status` returns:

```json
{"state":"inactive"}
```

### Tray surface

The current tray menu includes:

- Block
- Allow
- Start with Windows
- Shutdown
- Reboot
- Sleep
- Hibernate
- Quit

Tray icons are red in Block mode and green in Allow mode.

### Logging

Wardoff writes rotating structured JSONL logs to:

```text
%LOCALAPPDATA%\Wardoff\logs\wardoff.jsonl
```

The current implementation keeps up to 3 log files and rotates when the active log grows past 5 MiB.

## Comparison

| Product | Source model | Stack | `shutdown.exe` handling | Status |
| --- | --- | --- | --- | --- |
| Wardoff | Open-source MIT project | Rust | ETW-based standard-mode monitoring for local `shutdown.exe`, plus Layers 1, 3, and 4; no aggressive IFEO mode yet | MVP runtime implemented on current branch |
| ShutdownBlocker | Closed freeware | .NET Framework 4.0 | Uses IFEO for `shutdown.exe` | Last known update March 2017 |
| ShutdownGuard | Open-source MIT project | Pure C with MinGW | Historical DLL injection approach | Archived as `UNSUPPORTED` since 2014 |
| Don't Sleep | Closed freeware | Closed-source Windows utility | Does not block `shutdown.exe` | Actively maintained |
| PreventTurnOff | Closed freeware | Closed-source Windows utility | Same limitation as Don't Sleep | Actively maintained, simplified |

See [docs/COMPARISON.md](docs/COMPARISON.md) for the full comparison and trade-offs.

## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md), read `PLAN.md`, and keep changes aligned with the current MVP scope and the current branch's honest limitations.

## License

Wardoff is licensed under the [MIT License](LICENSE).
