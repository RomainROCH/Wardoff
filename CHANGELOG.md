# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Changed
- Explicitly documented `docs/ARCHITECTURE.md` as the canonical architecture authority and added guardrails against silently rewriting it to match incidental implementation drift.
- Explicitly documented `docs/BUSINESS_MODEL.md` as the canonical business-model authority and linked repo guidance back to it for monetization and commercialization messaging.
- Hardened `\\.\pipe\WardoffControl` with explicit local-only security that grants the current user SID, SYSTEM, and Builtin Administrators access so the same interactive user can send `--block`/`--allow` across elevation boundaries, while remaining access-denied cases now surface a clear Wardoff message instead of raw `os error 5`.
- Clarified in README, architecture notes, and agent guidance that Layer 3 is skipped normally on editions such as LTSC when `\Microsoft\Windows\UpdateOrchestrator\Reboot` is absent.
- Aligned docs on the non-elevated read-only CLI contract, the `\\.\pipe\WardoffControl` and `\\.\pipe\WardoffStatus` split, default startup honesty, and current exit-code wording.
- Clarified in the docs that `wardoff --status` is quick when a runtime is active but can currently take roughly 4 seconds to return `{"state":"inactive"}` when no instance is running because the client exhausts sequential status-pipe and control-pipe retry loops.
- Refreshed the documentation truth-source and read order so new contributors and agents can identify the current MVP status, documentation entrypoints, and likely next work from the repository alone.
- Hardened Allow-mode deactivation so switching to Allow clears Block-mode behavior more cleanly.
- Tightened autostart tray-sync honesty so elevated task changes are reflected accurately in the tray menu state.
- Improved secondary handoff recovery when a follower reaches Wardoff during a primary restart or handoff window.
- Expanded validation guidance for tray sync, Explorer restart, secondary handoff, and admin-only autostart scenarios.
- Switched Windows release-manifest embedding from manual linker flags to `winres` resource compilation and added a smoke guard that asserts the built release binary still embeds `requestedExecutionLevel level="asInvoker"`.
- Made the release binary console-friendly for direct PowerShell CLI use, while detaching or hiding console state for long-lived tray/runtime launches so read-only commands stay inline without regressing background UX.

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
