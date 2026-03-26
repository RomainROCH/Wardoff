# MVP validation matrix

Use this as the MVP acceptance checklist for Wardoff as it exists today. It keeps automated coverage, manual spot checks, admin-only validation, and disruptive VM-only checks separate.

## Scope and honesty rules

This matrix covers only the current MVP surface:

- Layer 1 interactive shutdown and sign-out blocking
- Layer 3 Update Orchestrator reboot-task protection
- Layer 4 best-effort remote shutdown abort polling
- sleep, hibernate, and display-idle blocking
- tray behavior
- CLI behavior
- IPC and single-instance behavior
- structured logging
- autostart

Keep validation language conservative:

- do **not** claim Wardoff blocks `shutdown /t 0 /f`
- keep local `shutdown.exe` handling documented as cautious, best-effort behavior rather than guaranteed MVP sign-off
- treat unsupported backlog items as out of scope for this checklist

## Quick environment notes

| Area | Notes |
| --- | --- |
| OS | Windows 10 or Windows 11 |
| Build target | `cargo build --release` then validate `target\release\wardoff.exe` |
| Logs | `%LOCALAPPDATA%\Wardoff\logs\wardoff.jsonl` |
| Admin rights | Required for Layer 3, Layer 4, Update Orchestrator task checks, and autostart task changes |
| Disposable VM | Required for shutdown, reboot, sign-out, sleep, hibernate, or remote-shutdown checks that could disrupt the host |

## Automated checks already covered by `tests\smoke_test.ps1`

| Check | What the script proves today | Admin needed |
| --- | --- | --- |
| Release build | `cargo build --release` succeeds | No |
| Release artifact | `target\release\wardoff.exe` exists | No |
| CLI help | `wardoff --help` exits `0` and mentions `wardoff` | No |
| CLI version | `wardoff --version` exits `0` and prints `wardoff 0.1.0` | No |
| Inactive status | `wardoff --status` exits `1` and returns `{"state":"inactive"}` when no instance is running | No |
| Hidden block startup | `Start-Process ... --block --hide` keeps the runtime alive | No |
| Active status JSON | `wardoff --status` exits `0`, returns `state=block`, and includes `layers.local_shutdown` as a boolean | No |
| Sleep/display request visibility | `powercfg /requests` mentions Wardoff while blocking is active, when the session allows that query | No |
| Structured logging | `%LOCALAPPDATA%\Wardoff\logs\wardoff.jsonl` exists, appends new lines, and contains valid JSON | No |
| Layer 3 status | Elevated status reports `layers.update=true` | Yes |
| Layer 4 status | Elevated status reports `layers.remote=true` | Yes |
| Update task access | `schtasks /query /tn Microsoft\Windows\UpdateOrchestrator\Reboot` completes | Yes |
| Autostart task lifecycle | `wardoff --autostart on` creates the `Wardoff` task and `--autostart off` removes it | Yes |
| Cleanup path | `wardoff --allow` plus process cleanup stops the background instance | No |
| Sleep/display cleanup | `powercfg /requests` no longer mentions Wardoff after cleanup, when the session allows that query | No |

The smoke test checks status visibility for local shutdown handling, but that is **not** the same as promising reliable interception of all local shutdown paths.

## Manual non-disruptive checks

Use a normal desktop session for these checks. They should not require actually shutting down, rebooting, sleeping, or hibernating the machine.

| Area | Check | Expected result |
| --- | --- | --- |
| Tray presence | Launch `wardoff` and confirm exactly one tray icon appears | Runtime is visible and does not create duplicate tray icons |
| Tray state | Toggle Block and Allow from the tray menu | State changes cleanly without crashing or leaving stale UI |
| CLI to primary runtime | With Wardoff already running, run `wardoff --status`, `wardoff --allow`, and `wardoff --block` from a second console | Secondary commands control the existing runtime instead of creating a second one |
| Single-instance behavior | Start Wardoff more than once from the repo build | Only one primary runtime remains active |
| IPC | Change state from CLI while the tray instance is running | The running instance reflects the requested state change |
| Logging | Run `wardoff --log --tail 10` after state changes | Recent structured log entries are readable from the CLI |
| Tray menu surface | Open the tray menu without invoking disruptive actions | Menu shows Block, Allow, Start with Windows, Shutdown, Reboot, Sleep, Hibernate, and Quit |
| Exit behavior | Quit from the tray after testing | Tray icon disappears and the runtime exits cleanly |

## Admin-only checks

Run these in an elevated session when you want manual confidence beyond the automated smoke coverage.

| Area | Check | Expected result |
| --- | --- | --- |
| Layer 3 | Start Wardoff in Block mode and run `wardoff --status` | Status reports `layers.update=true` |
| Layer 4 | Keep Wardoff running elevated in Block mode and run `wardoff --status` | Status reports `layers.remote=true` |
| Update task access | Run `schtasks /query /tn "Microsoft\Windows\UpdateOrchestrator\Reboot"` | Task query succeeds |
| Autostart on | Run `wardoff --autostart on`, then query `schtasks /query /tn "Wardoff"` | The `Wardoff` task exists |
| Autostart off | Run `wardoff --autostart off`, then query `schtasks /query /tn "Wardoff"` | The `Wardoff` task is removed |
| Honest non-admin behavior | Retry an admin-only command from a non-elevated console | Wardoff reports the requirement instead of pretending success |

## Disruptive manual checks for a disposable VM

Do these only in a disposable VM or similarly safe environment.

| Area | Check | Expected result |
| --- | --- | --- |
| Layer 1 interactive shutdown | With Wardoff in Block mode, attempt a normal interactive shutdown or sign-out from Windows | Windows gives Wardoff the chance to block the interactive path |
| Sleep blocking | With Wardoff in Block mode, trigger Sleep from Windows or the tray | The VM stays awake while blocking is active |
| Hibernate blocking | With Wardoff in Block mode, trigger Hibernate from Windows or the tray | The VM does not hibernate while blocking is active |
| Allow-mode release | Switch to Allow mode and retry Sleep or Hibernate | Windows proceeds normally once Wardoff is no longer blocking |
| Remote shutdown abort | From a second machine or remote management session, issue a timed remote shutdown while Wardoff is running elevated in Block mode | Wardoff attempts to abort the shutdown while Windows still exposes an abortable timeout window |
| Tray power actions | If validating tray Shutdown or Reboot directly, do it only in the VM | The action reaches Windows without risking the host workstation |

## Acceptance checklist

- [ ] `tests\smoke_test.ps1` passes, with any skips noted
- [ ] Manual non-disruptive checks completed
- [ ] Admin-only checks completed or explicitly deferred
- [ ] Disruptive VM-only checks completed or explicitly deferred
- [ ] Docs and PR wording stay within the current MVP and do not overstate local shutdown handling
