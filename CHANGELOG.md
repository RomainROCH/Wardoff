# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- Initial project documentation scaffold.

## [0.1.0] - 2026-03-22

### Added
- Initial Windows MVP runtime
- Layer 1 interactive shutdown and sign-out blocking
- Layer 3 protection for `\Microsoft\Windows\UpdateOrchestrator\Reboot`
- Layer 4 best-effort remote shutdown abort polling
- Sleep, hibernate, and display-idle blocking via `SetThreadExecutionState(...)`
- Tray UI with Block, Allow, Shutdown, Reboot, Sleep, Hibernate, and Quit actions
- CLI support for `--block`, `--allow`, `--hide`, `--status`, `--log`, `--tail`, and `--autostart`
- Structured rotating JSONL logging
- Single-instance coordination and named-pipe IPC control
- Task Scheduler autostart integration
- Release manifest and elevation flow for the Windows binary
