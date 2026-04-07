# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Changed
- Clarified in README, architecture notes, and agent guidance that Layer 3 is skipped normally on editions such as LTSC when `\Microsoft\Windows\UpdateOrchestrator\Reboot` is absent.
- Aligned docs on the non-elevated read-only CLI contract, the `\\.\pipe\WardoffControl` and `\\.\pipe\WardoffStatus` split, default startup honesty, and current exit-code wording.
- Refreshed the documentation truth-source and read order so new contributors and agents can identify the current MVP status, documentation entrypoints, and likely next work from the repository alone.
- Hardened Allow-mode deactivation so switching to Allow clears Block-mode behavior more cleanly.
- Tightened autostart tray-sync honesty so elevated task changes are reflected accurately in the tray menu state.
- Improved secondary handoff recovery when a follower reaches Wardoff during a primary restart or handoff window.
- Expanded validation guidance for tray sync, Explorer restart, secondary handoff, and admin-only autostart scenarios.

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
