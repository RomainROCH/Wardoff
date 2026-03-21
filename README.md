# Wardoff

*The first modern open-source tool for taking control of Windows shutdowns.*

![MIT License](https://img.shields.io/badge/license-MIT-green)
![Rust](https://img.shields.io/badge/language-Rust-orange)
![Windows](https://img.shields.io/badge/platform-Windows%2010%2B-blue)

Unexpected shutdowns and reboots still interrupt gaming sessions, long-running builds, remote administration, and overnight workloads. Existing utilities are often closed-source, abandoned, or focused mainly on sleep prevention rather than transparent shutdown control. Wardoff is planned as an open Windows-native project that documents its limits instead of hiding them. Today, this repository is still an early scaffold: documentation is present, a minimal Rust workspace may exist, but no runtime shutdown interception is implemented yet.

## Features MVP

Current repository state: documentation plus an early Rust scaffold only. No working tray app, CLI, or shutdown blocker is implemented yet.

Implemented in this repository today:

- project planning and architecture documentation based on a four-layer shutdown strategy
- a minimal Rust workspace scaffold with placeholder entry-point code
- technical notes about shutdown limits, Windows Update reboot handling, and future IFEO warnings
- contributor, changelog, and licensing files for the project baseline

Planned runtime MVP surface (not yet implemented):

- Layer 1 standard shutdown blocking with `ShutdownBlockReasonCreate()`, `WM_QUERYENDSESSION`, and `SetProcessShutdownParameters()`
- Layer 3 control of `Microsoft\Windows\UpdateOrchestrator\Reboot`
- Layer 4 remote shutdown abort polling via `AbortSystemShutdown(...)`
- sleep, hibernate, and display-idle blocking via `SetThreadExecutionState(...)`
- tray icon with Block/Allow state and basic power actions
- CLI commands for `--block`, `--allow`, `--status`, and `--hide`
- Task Scheduler auto-start and rotating JSON-lines file logs

## Planned features v1.0

All items in this section are planned and not implemented in the current repository.

- ETW monitoring for local `shutdown.exe` and related process activity
- opt-in aggressive IFEO mode for earlier local `shutdown.exe` interception
- Windows Event Log integration
- native Windows toast notifications
- timer-based blocking windows
- packaging and distribution via GitHub Releases, Winget, Scoop, and Chocolatey

## Honest limitations

- `shutdown /t 0 /f` cannot be blocked from user space once the local command is already running. The planned standard APIs can only react in time when a timeout window still exists.
- The future IFEO mode is the only planned user-space interception point for the local `shutdown.exe` launch path, and it will be opt-in, admin-only, and security-sensitive.
- Windows Update protection is tied to the `Microsoft\Windows\UpdateOrchestrator\Reboot` task and requires administrator rights.
- Remote shutdown abort logic only helps while Windows still exposes a shutdown timeout window.
- This repository does not yet provide a runnable binary, working tray icon, or working CLI.

## How it works

Wardoff is designed as a layered Windows-only tool because no single user-space API covers every shutdown origin. The architecture reference is documented in [docs/WINDOWS_SHUTDOWN_LAYERS.md](docs/WINDOWS_SHUTDOWN_LAYERS.md):

1. standard interactive shutdown blocking
2. local `shutdown.exe` handling
3. Windows Update reboot task control
4. remote shutdown abort polling

There is also a separate planned power-state blocker for sleep, hibernate, and display idle prevention.

## Installation

Installation is not available yet. Planned distribution paths are:

- `cargo install wardoff` once a published crate exists
- downloadable binaries from GitHub Releases once release packaging exists

## Usage

The commands below describe the planned CLI surface. They are not available in the current repository yet.

```powershell
wardoff --block
wardoff --allow
wardoff --status
wardoff --hide
```

Planned tray behavior:

- clear Block (red) versus Allow (green) state
- quick actions for Block, Allow, Shutdown, Reboot, Sleep, Hibernate, and Quit
- optional hidden/background launch mode

## Comparison

| Product | Source model | Stack | `shutdown.exe` handling | Status |
| --- | --- | --- | --- | --- |
| Wardoff | Open-source MIT project | Rust (planned) | Planned layered handling; no runtime implementation yet | Planning/documentation stage |
| ShutdownBlocker | Closed freeware | .NET Framework 4.0 | Uses IFEO for `shutdown.exe` | Last known update March 2017 |
| ShutdownGuard | Open-source MIT project | Pure C with MinGW | Historical DLL injection approach | Archived as `UNSUPPORTED` since 2014 |
| Don't Sleep | Closed freeware | Closed-source Windows utility | Does not block `shutdown.exe` | Actively maintained |
| PreventTurnOff | Closed freeware | Closed-source Windows utility | Same limitation as Don't Sleep | Actively maintained, simplified |

See [docs/COMPARISON.md](docs/COMPARISON.md) for the full comparison and trade-offs.

## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md), read `PLAN.md`, and keep changes aligned with the current MVP scope.

## License

Wardoff is licensed under the [MIT License](LICENSE).
