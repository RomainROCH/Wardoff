# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

Target version: `0.2.1`. This is a local candidate, not a published release.

### Changed
- Gate release tags on matching package/lock/manifest versions, main-branch provenance and successful Windows checks for the exact commit. Build and transfer the same checked binary into a draft only; publication remains manual.
- Bound newline-terminated control IPC requests to 4096 bytes and one total second, preserving independent status availability and rejection/recovery behavior.
- Validate autostart path access with identification tokens, and record disposable-VM idle-power acceptance without claiming physical hardware coverage.
- Bounded `--log --tail N` to 100000 lines and made line-buffer reservation incremental and fallible, so extreme values return a readable error instead of a capacity-overflow panic.
- Ensure the runtime unregisters callbacks and shuts down its resources even when the Windows message loop fails; preserve both errors if cleanup also fails.
- Make the Windows smoke suite refuse existing runtime/autostart state and restrict cleanup to resources created by the test.
- Align CI and release validation with local documentation, formatting, strict Clippy, Rust tests and PowerShell fixtures, using the existing standard Windows jobs.
- Replaced the sleep layer's periodic `SetThreadExecutionState` worker with owned system/display Power Requests and an identifiable Wardoff reason. Activation failures now reach the coordinator synchronously, and Allow/exit release the request object.
- Kept the existing tray wake-restore flow and added only a request-pair renewal to the existing resume callback when the runtime remains in Block, based on the documented termination of requests at user-initiated sleep. No additional resume state machine, display notifications or polling was introduced.
- Corrected sleep claims throughout maintained docs, CLI help and runtime logs: protection concerns idle sleep and automatic display timeout, subject to Windows policy. Explicit Sleep/Hibernate are not vetoed; Modern Standby on battery has additional limits. The original v0.1.0 hibernation claim below was too broad.

## [0.2.0] - 2026-05-03

### Changed
- Hardened the session-scoped `WardoffControl` and `WardoffStatus` pipes against same-session squatting by claiming first pipe instances with bounded startup retries, failing closed until both IPC servers own their initial instances, and probing the singleton before `wardoff --status` opens IPC endpoints.
- Scoped the singleton mutex plus the `WardoffControl` and `WardoffStatus` named pipes to the current Windows session so cross-session mutex or pipe squatting no longer breaks Wardoff's single-instance model across all sessions.
- Hardened structured JSONL logging under `%LOCALAPPDATA%\Wardoff\logs` so Wardoff now creates its managed log directories one component at a time, refuses reparse points in the managed log tree and rotated files, and uses reparse-aware no-follow opens for active and read-only log file access.
- Hardened highest-runlevel autostart registration so Wardoff canonicalizes its executable path, refuses `--autostart on` from user-writable install locations, and derives the scheduled-task principal from the process token instead of `USERNAME`/`USERDOMAIN`.
- Hardened Layer 1 `WM_ENDSESSION` cleanup so the hidden session window now ignores spoof-like end-session messages unless Windows reports an actual shutdown or sign-out is in progress.
- Tray-initiated sleep and hibernate requests now restore Block mode automatically on wake when Wardoff was in Block mode before the tray action, and emit a `block_restored_after_wake` structured log event. The hidden Layer 1 session window now explicitly registers for suspend/resume notifications via `RegisterSuspendResumeNotification`; without that registration, since Windows 8 `WM_POWERBROADCAST` with `PBT_APMSUSPEND`/`PBT_APMRESUMEAUTOMATIC`/`PBT_APMRESUMESUSPEND` is no longer broadcast to top-level windows automatically, so the previous restore-after-wake state machine never fired.
- Explicitly documented `docs/ARCHITECTURE.md` as the canonical architecture authority and added guardrails against silently rewriting it to match incidental implementation drift.
- Explicitly documented `docs/BUSINESS_MODEL.md` as the canonical business-model authority and linked repo guidance back to it for monetization and commercialization messaging.
- Hardened `\\.\pipe\WardoffControl` with explicit local-only security that grants the current user SID, SYSTEM, and Builtin Administrators access so the same interactive user can send `--block`/`--allow` across elevation boundaries, while remaining access-denied cases now surface a clear Wardoff message instead of raw `os error 5`.
- Clarified in README, architecture notes, and agent guidance that Layer 3 is skipped normally on editions such as LTSC when `\Microsoft\Windows\UpdateOrchestrator\Reboot` is absent.
- Aligned docs on the non-elevated read-only CLI contract, the `\\.\pipe\WardoffControl` and `\\.\pipe\WardoffStatus` split, default startup honesty, and current exit-code wording.
- Clarified that `wardoff --status` returns inactive without pipe retries when the session mutex is free. Roughly four seconds of sequential pipe retries remain possible when the mutex is occupied but both IPC endpoints are unavailable.
- Refreshed the documentation truth-source and read order so new contributors and agents can identify the current MVP status, documentation entrypoints, and likely next work from the repository alone.
- Hardened Allow-mode deactivation so switching to Allow clears Block-mode behavior more cleanly.
- Tightened autostart tray-sync honesty so elevated task changes are reflected accurately in the tray menu state.
- Improved secondary handoff recovery when a follower reaches Wardoff during a primary restart or handoff window.
- Expanded validation guidance for tray sync, Explorer restart, secondary handoff, and admin-only autostart scenarios.
- Switched Windows release-manifest embedding from manual linker flags to `winres` resource compilation and added a smoke guard that asserts the built release binary still embeds `requestedExecutionLevel level="asInvoker"`.
- Made the release binary console-friendly for direct PowerShell CLI use, while detaching or hiding console state for long-lived tray/runtime launches so read-only commands stay inline without regressing background UX.
- Hidden the console window immediately at startup when Wardoff owns a freshly allocated console (double-click, autostart spawn, elevated relaunch child) so the brief console flash is no longer visible before the runtime bootstrap reaches its existing late hide. Inherited shell consoles are still left untouched so read-only CLI commands keep writing to the user's terminal.
- Aligned the pre-release docs on private reporting wording, community-only support availability, session-scoped IPC wording, and the unreleased status of the `v0.2.0` draft notes.

## [0.1.0] - 2026-03-22

### Added
- Initial Windows MVP runtime
- Layer 1 interactive shutdown and sign-out blocking
- Layer 3 protection for `\Microsoft\Windows\UpdateOrchestrator\Reboot`
- Layer 4 best-effort remote shutdown abort polling
- Idle-sleep and display-timeout prevention via `SetThreadExecutionState(...)` (wording corrected: the original release description overstated hibernation protection)
- Tray UI with Block, Allow, Shutdown, Reboot, Sleep, Hibernate, and Quit actions
- CLI support for `--block`, `--allow`, `--hide`, `--status`, `--log`, `--tail`, and `--autostart`
- Structured rotating JSONL logging
- Single-instance coordination and named-pipe IPC control
- Task Scheduler autostart integration
- Release manifest and elevation flow for the Windows binary
