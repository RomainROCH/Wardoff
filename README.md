# Wardoff

![MIT License](https://img.shields.io/badge/license-MIT-green)
![Rust](https://img.shields.io/badge/language-Rust-orange)
![Windows](https://img.shields.io/badge/platform-Windows%2010%2B-blue)

*The first modern open-source tool for taking control of Windows shutdowns.*

Wardoff is a Windows-native Rust utility for people who want a visible, scriptable way to keep a machine in a protected **Block** state during gaming sessions, overnight jobs, remote work, or maintenance windows. The project is intentionally honest about what the current MVP does today and what still belongs to later releases.

## Current MVP status

Wardoff currently ships a working tray/runtime app plus CLI with these MVP-level capabilities:

- Block/Allow runtime state with a red/green tray icon
- Layer 1 interactive shutdown and sign-out blocking through `WM_QUERYENDSESSION`, `ShutdownBlockReasonCreate()`, and `SetProcessShutdownParameters()`
- Layer 3 protection for the scheduled task `Microsoft\Windows\UpdateOrchestrator\Reboot` when that task exists
- Layer 4 best-effort remote shutdown abort polling with `AbortSystemShutdownW(None)`
- sleep, hibernate, and display-idle blocking via `SetThreadExecutionState(...)`
- structured rotating JSONL logging under `%LOCALAPPDATA%\Wardoff\logs\`
- single-instance coordination so later CLI calls can control the primary runtime
- Task Scheduler autostart management
- tray actions for Shutdown, Reboot, Sleep, Hibernate, and Quit

## Documentation entrypoints

If you are brand-new to the repo, read these in order:

1. **`README.md`** - current user-facing MVP snapshot
2. **[`PLAN.md`](PLAN.md)** - current status, boundaries, and prioritized next steps
3. **[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)** - where the runtime pieces live in code
4. **[`docs/BUSINESS_MODEL.md`](docs/BUSINESS_MODEL.md)** - source of truth for monetization, signed binaries, sponsorship, consulting/support, and commercialization decisions
5. **[`docs/WINDOWS_SHUTDOWN_LAYERS.md`](docs/WINDOWS_SHUTDOWN_LAYERS.md)** - detailed shutdown-layer behavior, including Layer 2 ETW/local-shutdown nuances and limits
6. **[`docs/MVP_VALIDATION_MATRIX.md`](docs/MVP_VALIDATION_MATRIX.md)** - acceptance and validation guide covering smoke coverage, tray and Explorer-restart checks, handoff-focused spot checks, and admin-only validation
7. **[`CONTRIBUTING.md`](CONTRIBUTING.md)** - workflow, testing, and documentation guardrails
8. **[`.github/copilot-instructions.md`](.github/copilot-instructions.md)** - agent-specific read order and scope rules

## Not implemented yet

These are **not** part of the current documented MVP and should not be treated as shipped user-facing features:

- aggressive IFEO interception
- ETW-based local `shutdown.exe` interception as a documented feature
- Windows Event Log integration
- Windows toast notifications
- timers
- profiles
- settings UI
- packaged distribution via Winget, Scoop, Chocolatey, or a polished installer

For why IFEO stays out of the current MVP, see [`docs/IFEO_WARNING.md`](docs/IFEO_WARNING.md).

## Honest limits

- **Wardoff does not promise to stop `shutdown /t 0 /f`.** A forced zero-second local shutdown is outside what a normal user-space app can reliably block.
- Windows Update reboot-task protection requires administrator rights.
- Remote shutdown abort logic only helps when Windows still exposes a timeout window and the process has the rights required for `AbortSystemShutdownW(None)`.
- Some CLI paths can run without elevation, but admin-only features will report that requirement instead of pretending they succeeded.
- On editions such as Windows LTSC where `\Microsoft\Windows\UpdateOrchestrator\Reboot` does not exist, Layer 3 is skipped automatically; that is normal, not a failure.

For the detailed per-layer behavior and the cautious Layer 2 local `shutdown.exe` wording, see [`docs/WINDOWS_SHUTDOWN_LAYERS.md`](docs/WINDOWS_SHUTDOWN_LAYERS.md).

## What comes next

The repo is already in a conservative MVP state, so the next likely work is **hardening and clarifying what already ships**, not pretending a big new feature wave is done.

Today, a new agent should assume this order of priority:

1. improve confidence and docs around the implemented shutdown, update-reboot, remote-abort, sleep, tray, IPC, logging, and autostart behavior
2. keep build/run/read-order guidance easy for a first-time contributor or evaluator
3. treat larger feature ideas as later backlog unless `PLAN.md` explicitly moves them forward

For the current roadmap summary, read [`PLAN.md`](PLAN.md).

## Installation

Wardoff is currently source-first.

### Requirements

- Windows 10 or Windows 11
- Rust toolchain
- MSVC build tools for `x86_64-pc-windows-msvc`

### Build from source

```powershell
cargo build --release
```

The compiled binary is:

```text
target\release\wardoff.exe
```

## Usage

### Quick start

```powershell
wardoff
wardoff --version
wardoff --block --hide
wardoff --status
wardoff --autostart on
wardoff --log --tail 10
wardoff --allow
```

### What the main commands do

```powershell
wardoff
wardoff --block
wardoff --block --hide
wardoff --hide
wardoff --allow
wardoff --status
wardoff --log
wardoff --log --tail 10
wardoff --autostart on
wardoff --autostart off
wardoff --version
```

- `wardoff` starts a new primary runtime in **Block** mode with the tray visible when no primary instance is already running
- `wardoff --block` switches an existing runtime into Block mode, or starts a new headless Block-mode primary if none is running
- `wardoff --block --hide` is accepted and behaves like hidden Block-mode startup when it launches a new primary runtime
- `wardoff --hide` switches an existing runtime into Block mode, or starts a new Block-mode primary with the tray icon hidden
- `wardoff --allow` switches the running instance to Allow mode, or starts a visible Allow-mode runtime if needed
- `wardoff --status` prints compact JSON for scripts
- `wardoff --log` prints recent structured log entries
- `wardoff --log --tail 10` prints the newest 10 structured log entries
- `wardoff --autostart on|off` enables or disables the scheduled-task autostart entry
- `wardoff --version` prints the package version, for example `wardoff 0.1.0`
- direct PowerShell invocations of `wardoff --help`, `--version`, `--status`, and `--log --tail N` now behave like normal console commands with real stdout, stderr, and exit codes

Read-only commands stay non-elevated:

- `--help` and `--version` short-circuit locally in clap
- `--status` reads status through the dedicated `\\.\pipe\WardoffStatus` pipe when a primary runtime is active
- `--log` and `--log --tail N` read the rotating JSONL log files directly
- those read-only commands stay attached to the calling shell so direct PowerShell CLI invocations can capture their output inline
- today, `--status` can still take a few seconds to return `{"state":"inactive"}` when no primary runtime is running; when a runtime is active, the status-pipe path should return quickly

State-changing/default behavior uses the normal runtime path:

- default `wardoff` startup bootstraps a new visible Block-mode primary runtime and may trigger Wardoff's built-in self-elevation path when admin-only protections are desired
- `--block`, `--allow`, `--hide`, and `--autostart on|off` are not read-only commands; they either start a runtime locally or talk to the primary runtime over the state-changing control pipe `\\.\pipe\WardoffControl`
- when that primary runtime is already elevated, the same interactive Windows user can still send control-pipe commands from a normal non-elevated shell
- if the active primary runtime belongs to a different Windows user context, Wardoff reports that product mismatch clearly instead of surfacing a raw pipe access-denied error
- when a CLI launch would create a new long-lived primary runtime from an existing shell console, Wardoff relaunches that runtime in a detached background process so the shell prompt is not left hanging
- when the long-lived runtime starts from Explorer, autostart, or another launch path without an inherited shell console, Wardoff hides and frees any dedicated console window during startup

Example status output while Wardoff is active:

```json
{"state":"block","layers":{"shutdown":true,"local_shutdown":false,"update":true,"remote":true,"sleep":true},"uptime_seconds":42,"blocked_count":3}
```

If no primary runtime is running:

```json
{"state":"inactive"}
```

### Exit codes

- `wardoff --help` exits `0`
- `wardoff --version` exits `0`
- `wardoff --status` exits `0` when Wardoff is active, exits `1` when no primary runtime is active, and exits `1` on errors
- `wardoff --log` and `wardoff --log --tail N` exit `0` on success and `1` on errors
- `wardoff --block` and `wardoff --allow` exit `0` on success, including when they start a runtime or switch the running runtime, and exit `1` when they fail
- `wardoff --autostart on|off` exits `0` on success and `1` on failure

## Running as Administrator

Wardoff is explicit about elevation:

- the release manifest now stays at `asInvoker`, so explicit CLI invocations can start in a normal non-elevated console
- the release build now embeds that `asInvoker` manifest through the normal Windows resource path instead of ad-hoc linker flags, which makes the launch contract more predictable for shell invocations
- the release binary is now console-friendly in release builds, so direct PowerShell invocations of read-only commands stay in the calling console instead of behaving like a GUI-style launch
- if `wardoff` needs to bootstrap a new primary runtime with no explicit command, it still triggers Wardoff's built-in self-elevation path when administrator rights are needed for that default startup
- `src/main.rs` now keeps an explicit non-elevating read-only dispatch for `--status` and `--log --tail N` before any runtime bootstrap logic
- `wardoff --status` uses a dedicated read-only status pipe, and `wardoff --log --tail N` reads structured log files directly, so both commands stay non-elevated even when the primary runtime is already elevated
- state-changing secondary commands still use the control pipe `\\.\pipe\WardoffControl`, but Wardoff now creates that pipe as local-only and explicitly grants control access to the current Windows user SID so same-user control still works across elevation boundaries
- a non-elevated shell can send `--block` and `--allow` to an elevated primary runtime when both belong to the same interactive Windows user; if Wardoff is running as a different user context, the CLI returns a clear product message instead of raw `os error 5`
- `--help` and `--version` still short-circuit inside clap and remain pure local console output
- if a new long-lived primary runtime is launched from an existing console, Wardoff detaches and relaunches that runtime so the console session can return immediately
- if a new long-lived primary runtime starts without an inherited shell console, Wardoff hides and frees the standalone console early so Explorer-style launches do not leave an empty console window behind
- Layer 3 UpdateOrchestrator protection requires elevation
- on editions such as LTSC where `\Microsoft\Windows\UpdateOrchestrator\Reboot` is missing, Layer 3 is skipped automatically and that is expected
- Layer 4 remote shutdown abort polling depends on the shutdown-abort privilege and is intended to run elevated
- `--autostart on|off` requires elevation because it changes a scheduled task
- if you run an explicit CLI command in a non-elevated console, Wardoff keeps working where it can and reports when an admin-only action is unavailable

## Tray behavior

The current tray menu includes:

- Block
- Allow
- Start with Windows
- Shutdown
- Reboot
- Sleep
- Hibernate
- Quit

Red icon = Block mode. Green icon = Allow mode.

For manual verification of tray sync, Explorer restart recovery, secondary handoff windows, and admin-only Start with Windows behavior, use [`docs/MVP_VALIDATION_MATRIX.md`](docs/MVP_VALIDATION_MATRIX.md).

## Logging

Wardoff writes rotating structured JSONL logs to:

```text
%LOCALAPPDATA%\Wardoff\logs\wardoff.jsonl
```

The current implementation keeps up to 3 log files and rotates when the active file grows past 5 MiB.

## Comparison

| Product | Source model | `shutdown.exe` story | Current reality |
| --- | --- | --- | --- |
| Wardoff | Open-source MIT | Honest MVP: interactive shutdown/sign-out blocking, UpdateOrchestrator protection, remote abort polling, but no documented ETW/IFEO local interception yet | Implemented MVP runtime on this branch |
| ShutdownBlocker | Closed freeware | Commonly associated with aggressive `shutdown.exe` interception approaches | Last known update March 2017 |
| ShutdownGuard | Open-source MIT | Historical injection-based approach | Archived / unsupported |
| Don't Sleep | Closed freeware | Focused mainly on sleep/power-state prevention, not transparent shutdown control | Actively maintained |
| PreventTurnOff | Closed freeware | Similar positioning, simplified | Actively maintained |

For the fuller product comparison, see [`docs/COMPARISON.md`](docs/COMPARISON.md).

## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md), then [PLAN.md](PLAN.md), and keep changes aligned with the current MVP boundaries.

## Support the project

If Wardoff saves you time, consider supporting the project.

You can support Wardoff on [GitHub Sponsors](https://github.com/sponsors/RomainROCH). Signed binaries will be available soon for people who want SmartScreen-free deployment.

## License

Wardoff is licensed under the [MIT License](LICENSE).
