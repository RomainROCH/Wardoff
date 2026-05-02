# Wardoff v0.2.0

## What's new since v0.1.0-mvp

### Fixed
- Fixed non-elevated CLI launch paths so read-only commands behave like normal PowerShell and console commands, with more honest exit codes and status output.
- Fixed status and control IPC edge cases across elevation boundaries, including same-user control-pipe access, mutex access-denied handling, and inactive/handoff reporting.
- Fixed tray/runtime rough edges around delayed startup, secondary handoff recovery, wake-capable tray actions, allow-mode teardown, and recovery-failure reporting.
- Fixed autostart tray sync wording so admin-only behavior is reported honestly.
- Fixed Block mode not restoring after tray-initiated sleep/hibernate wake.
- Fixed five pre-release security blockers: writable-path autostart, log-directory symlink abuse, named-pipe squatting, mutex squatting, WM_ENDSESSION spoofing. IPC scoped to Windows session.

### Improved
- Added best-effort Layer 2 local `shutdown.exe` detection and abort handling in Block mode, while keeping the documented limits conservative.
- Improved Windows launch UX so long-lived runtime launches detach more cleanly from shell sessions and background startup behaves more predictably.
- Hardened the release manifest and non-elevating read-only CLI dispatch to reduce privilege surprises and keep read-only flows safer.
- Tightened smoke and validation coverage for read-only CLI paths and process cleanup timing.
- Added internal security audit (docs/SECURITY_AUDIT.md).
- Console window no longer flashes on startup.

### Documentation
- Added GitHub issue forms for bug reports, compatibility reports, documentation issues, and feature requests.
- Added `docs/MVP_VALIDATION_MATRIX.md` and expanded manual validation guidance for tray behavior, Explorer restart recovery, handoff cases, and admin-only paths.
- Refreshed README, contributing guidance, shutdown-layer docs, CLI exit-code docs, and repo guidance to better reflect the current conservative MVP surface.
- Cleaned up repo documentation links and removed the old session-tracker status file.

## Known limits
- Layer 3 (UpdateOrchestrator) is automatically skipped on LTSC editions that lack the `UpdateOrchestrator\Reboot` scheduled task. This is normal.
- Sleep/hibernate blocking requires hardware support for S1-S3 standby states. Hyper-V VMs typically do not support these.
- `shutdown /t 0 /f` is not guaranteed blockable. This is a Windows limitation, not a Wardoff bug.
- The product is source-first: no installer, no package manager distribution yet

## How to test
- Clone the repo and run `cargo build --release`
- Binary: `target\release\wardoff.exe`
- Requires Windows 10 or later
- Some features require admin rights (layers 2-4, autostart)

## How to report issues
- Bugs: use the bug report issue form
- Compatibility: use the compatibility report form
- Questions and feedback: use GitHub Discussions
