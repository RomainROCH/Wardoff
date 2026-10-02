# MVP validation matrix

Use this as the MVP acceptance checklist for Wardoff as it exists today. It keeps automated coverage, manual spot checks, admin-only validation, and disruptive VM-only checks separate.

## Scope and honesty rules

This matrix covers only the current MVP surface:

- Layer 1 interactive shutdown and sign-out blocking
- Layer 3 Update Orchestrator reboot-task protection
- Layer 4 best-effort remote shutdown abort polling
- idle-sleep and automatic display-timeout prevention, subject to Windows policy
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
| Trusted install path | Required if you expect `wardoff --autostart on` to create or update the `Wardoff` task; from an untrusted or user-writable path Wardoff should refuse safely and leave the scheduled-task state unchanged |
| Disruptive tests | Use a disposable VM for shutdown/reboot/sign-out checks; use prepared test hardware or a capable VM for sleep/hibernate checks |

## Automated checks already covered by `tests\smoke_test.ps1`

| Check | What the script proves today | Admin needed |
| --- | --- | --- |
| Release build | `cargo build --release` succeeds | No |
| Release artifact | `target\release\wardoff.exe` exists | No |
| CLI help | `wardoff --help` exits `0` and mentions `wardoff` | No |
| CLI version | `wardoff --version` exits `0` and prints the package version | No |
| Inactive status | `wardoff --status` exits `1` and returns `{"state":"inactive"}` when no instance is running | No |
| Hidden block startup | `Start-Process ... --block --hide` keeps the runtime alive | No |
| Active status JSON | `wardoff --status` exits `0`, returns `state=block`, and includes `layers.local_shutdown` as a boolean | No |
| Sleep/display request visibility | `powercfg /requests` contains the Wardoff reason for both requests in Block; it does not prove a physical transition was prevented | Query may require elevation |
| Block/Allow/Block lifecycle | `layers.sleep` is true/false/true; requests disappear in Allow while the same runtime remains alive, then acquisition is reported again in Block | Query may require elevation |
| Abrupt termination | The owned test runtime is killed directly from Block, without Allow; its requests disappear | Query may require elevation |
| Structured logging | `%LOCALAPPDATA%\Wardoff\logs\wardoff.jsonl` exists, appends new lines, and contains valid JSON | No |
| Layer 3 status | Elevated status reports `layers.update=true` when `\Microsoft\Windows\UpdateOrchestrator\Reboot` exists; if the task is absent, note the normal skip/defer case instead of treating it as a failure | Yes |
| Layer 4 status | Elevated status reports `layers.remote=true` | Yes |
| Update task access | `schtasks /query /tn Microsoft\Windows\UpdateOrchestrator\Reboot` either succeeds or shows the task is absent on this machine; absence is a normal Layer 3 skip/defer case | Yes |
| Autostart task lifecycle | From a trusted admin-writable location, `wardoff --autostart on` creates the `Wardoff` task and `--autostart off` removes it; from an untrusted or user-writable path Wardoff refuses safely and leaves the task state unchanged | Yes |
| Cleanup path | Direct termination from Block stops the owned background instance; a final cleanup helper handles test failures | No |
| Sleep/display cleanup | `powercfg /requests` no longer mentions Wardoff after cleanup, when the session allows that query | No |

The smoke test checks status visibility for local shutdown handling, but that is **not** the same as promising reliable interception of all local shutdown paths.

For an explicit cross target, set `CARGO_BUILD_TARGET=x86_64-pc-windows-msvc`
in the test process environment. The smoke script uses that target for its build
and resolves `target/x86_64-pc-windows-msvc/release/wardoff.exe` accordingly.

## Manual non-disruptive checks

Use a normal desktop session for these checks. They should not require actually shutting down, rebooting, sleeping, or hibernating the machine.

| Area | Check | Expected result |
| --- | --- | --- |
| Tray presence | Launch `wardoff` and confirm exactly one tray icon appears | Runtime is visible and does not create duplicate tray icons |
| Tray state | Toggle Block and Allow from the tray menu | State changes cleanly without crashing or leaving stale UI |
| Tray and CLI sync | With Wardoff running, alternate `wardoff --allow`, `wardoff --block`, and tray Block/Allow toggles while checking `wardoff --status` after each change | Tray state, status JSON, and the active mode stay aligned with the latest request without leaving stale Block-mode behavior behind |
| CLI to primary runtime | With Wardoff already running, run `wardoff --status`, `wardoff --allow`, and `wardoff --block` from a second console | Secondary commands control the existing runtime instead of creating a second one |
| Single-instance behavior | Start Wardoff more than once from the repo build | Only one primary runtime remains active |
| IPC | Change state from CLI while the tray instance is running | The running instance reflects the requested state change |
| Secondary handoff recovery | During a controlled primary restart or handoff window, run a secondary CLI command from another console | Wardoff remains controllable and settles back to a single primary runtime instead of staying unavailable or leaving two active runtimes |
| Logging | Run `wardoff --log --tail 10` after state changes | Recent structured log entries are readable from the CLI |
| Tray menu surface | Open the tray menu without invoking disruptive actions | Menu shows Block, Allow, Start with Windows, Shutdown, Reboot, Sleep, Hibernate, and Quit |
| Explorer restart / tray recreation | Restart Explorer while Wardoff keeps running, then wait for the shell to return | The tray icon is recreated once Explorer returns, matches the current state, and still opens the expected menu |
| Exit behavior | Quit from the tray after testing | Tray icon disappears and the runtime exits cleanly |

## Admin-only checks

Run these in an elevated session when you want manual confidence beyond the automated smoke coverage.

| Area | Check | Expected result |
| --- | --- | --- |
| Layer 3 | Start Wardoff in Block mode, check whether `\Microsoft\Windows\UpdateOrchestrator\Reboot` exists, then run `wardoff --status` | If the task exists, status reports `layers.update=true`; if the task is absent, record the normal skip/defer case |
| Layer 4 | Keep Wardoff running elevated in Block mode and run `wardoff --status` | Status reports `layers.remote=true` |
| Update task access | Run `schtasks /query /tn "Microsoft\Windows\UpdateOrchestrator\Reboot"` | Task query succeeds, or the machine reports the task is absent and you record Layer 3 as a normal skip/defer case |
| Autostart on | From a trusted admin-writable install location, run `wardoff --autostart on`, then query `schtasks /query /tn "Wardoff"` | The `Wardoff` task exists |
| Autostart off | From a trusted admin-writable install location, run `wardoff --autostart off`, then query `schtasks /query /tn "Wardoff"` | The `Wardoff` task is removed |
| Autostart safe refusal from untrusted path | Run the same autostart command from an untrusted or user-writable path | Wardoff refuses with the trusted-location warning and does not create, delete, or silently alter the `Wardoff` task |
| Autostart tray/menu honesty | In an elevated session, change autostart with `wardoff --autostart on|off` or the tray `Start with Windows` checkbox, then reopen the tray menu and confirm with `schtasks /query /tn "Wardoff"` | The tray checkbox reflects the real scheduled-task state after the change, and after an untrusted-location refusal it does not pretend autostart changed |
| Honest non-admin behavior | Retry an admin-only command from a non-elevated console | Wardoff reports the requirement instead of pretending success |

## Disruptive manual checks for a disposable VM

Do these only in a disposable VM or similarly safe environment.

| Area | Check | Expected result |
| --- | --- | --- |
| Layer 1 interactive shutdown | With Wardoff in Block mode, attempt a normal interactive shutdown or sign-out from Windows | Windows gives Wardoff the chance to block the interactive path |
| Windows explicit Sleep limit | With Wardoff in Block mode, trigger Sleep from the Windows power menu on a sleep-capable test machine | Windows may sleep; Wardoff does not veto the request. After resume, check that Block's idle-power requests are acquired again |
| Windows explicit Hibernate limit | With Wardoff in Block mode, trigger Hibernate on a test machine where it is already available | Windows may hibernate; Wardoff does not veto the request. After resume, check the mode, requests and logs |
| Tray Sleep wake-restore | With Wardoff in Block mode, trigger Sleep from the tray menu, then wake the VM | Wardoff temporarily switches to Allow so Sleep can proceed, then returns to Block after wake and records the wake restore behavior with a `block_restored_after_wake` log event |
| Tray Hibernate wake-restore | With Wardoff in Block mode, trigger Hibernate from the tray menu, then resume the VM | Wardoff temporarily switches to Allow so Hibernate can proceed, then returns to Block after resume and records the wake restore behavior with a `block_restored_after_wake` log event |
| Allow-mode release | Switch to Allow while the runtime remains alive, then wait for idle sleep/display timeout | Wardoff's requests are absent; Windows follows its existing settings and any requests from other applications |
| Remote shutdown abort | From a second machine or remote management session, issue a timed remote shutdown while Wardoff is running elevated in Block mode | Wardoff attempts to abort the shutdown while Windows still exposes an abortable timeout window |
| Tray power actions | If validating tray Shutdown or Reboot directly, do it only in the VM | The action reaches Windows without risking the host workstation |

## Idle-power behavior and Windows limits

This is the supported contract, not a claim that every row was physically tested.
`blocked` means automatic inactivity prevention while Windows honors the requests;
it never means a universal veto. Request visibility alone is not a sleep test.

| Transition / environment | Previous thread execution-state backend | Power Request backend | Guarantee |
| --- | --- | --- | --- |
| Automatic idle sleep on S3 | Idle prevention | System-required request | **blocked**, subject to policy |
| Automatic display timeout | Display-idle prevention | Display-required request, paired with system-required | **blocked**, subject to policy |
| Automatic hibernation | No general S4 veto | No general S4 veto | **cannot guarantee**; preventing initial idle sleep can indirectly avoid a later S3-to-S4 transition |
| Explicit Start-menu Sleep | User request takes precedence | User request takes precedence | **unsupported** as a veto |
| Explicit Hibernate | No documented veto | No documented veto | **unsupported** as a veto |
| Power button configured for Sleep | No guaranteed veto | No guaranteed veto | **unsupported** as a veto |
| Lid configured for Sleep | No guaranteed veto | No guaranteed veto | **unsupported** as a veto |
| Traditional S3 | Idle prevention | Idle prevention | Requires a capable hardware test |
| Modern Standby / AC | No universal claim from the old backend | Same request pair, governed by Windows | **best effort**; validate on Modern Standby hardware |
| Modern Standby / DC | No unlimited guarantee established | System-required requests end five minutes after the sleep timeout expires | **best effort**, not five minutes after activating Block |
| Critical battery / thermal or other safety transition | No guarantee | No guarantee | **cannot guarantee** |
| Screen saver, lock or explicit display-off action | No guarantee | Outside the idle-display contract | **unsupported** |
| Policy ignores requests / existing request override | No guarantee | No guarantee | **cannot guarantee**; Wardoff does not change the policy |
| Away Mode | Not requested | Not requested | No **redirected/substituted** transition is claimed |

### Microsoft evidence and rejected alternatives

- [SetThreadExecutionState](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate) documents the inactivity flags, continuous lifetime, explicit-user-sleep limit and lack of screen-saver blocking. Periodic refresh does not turn this into a veto.
- [PowerSetRequest](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powersetrequest) defines the request types, explicit-sleep termination and Modern Standby battery timeout. Execution-required requests concern process lifetime and add no general sleep veto.
- [PowerCreateRequest](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powercreaterequest) and [PowerClearRequest](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powerclearrequest) define object ownership and balanced counts. [Process termination](https://learn.microsoft.com/en-us/windows/win32/procthread/terminating-a-process) closes process handles. The combination supports cleanup on crash for Wardoff's private, non-shared handle; verify this separately from orderly cleanup.
- [Allow away mode](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/sleep-settings-allow-away-mode) can disable Away Mode even when requested. On eligible S3 systems it substitutes apparent sleep with video/audio off, not a veto; it has no Modern Standby coverage or documented general Hibernate veto. It is inappropriate for the single predictable Block/Allow contract and is not enabled. Reading capabilities and AC/DC policy could diagnose eligibility; Wardoff does not add that machinery for an excluded feature.
- [Allow system required requests](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/sleep-settings-allow-system-required-requests) and [powercfg](https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/powercfg-command-line-options) explain policy rejection, request listing and overrides. A successful API call does not override those settings.
- [Hibernate idle timeout](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/sleep-settings-hibernate-idle-timeout) describes the separate later hibernation transition.
- [PBT_APMQUERYSUSPEND](https://learn.microsoft.com/en-us/windows/win32/power/pbt-apmquerysuspend) and [PBT_APMQUERYSUSPENDFAILED](https://learn.microsoft.com/en-us/windows/win32/power/pbt-apmquerysuspendfailed) lost support in Vista; neither is a modern veto.
- [GetPwrCapabilities](https://learn.microsoft.com/en-us/windows/win32/api/powerbase/nf-powerbase-getpwrcapabilities) and [SYSTEM_POWER_CAPABILITIES](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-system_power_capabilities) provide a documented capability query if later diagnostics need one. No architecture-specific backend is needed now: `powercfg /a` identifies the test machine, and the runtime uses the same two requests without a capability-detection subsystem.

### Hardware procedure (manual, not CI)

Use a prepared test PC, or a disposable VM only if it exposes the required power
states. Do not run disruptive tests on a workstation with unsaved work. Do not
enable Hibernate, change button/lid actions, set request overrides or edit a
power plan merely to make a row pass; use an already suitable test setup.

1. Record Windows build, hardware, AC/DC source, binary hash and commit. Capture
   `powercfg /a`, `/getactivescheme`, `/query`, `/requestsoverride` (without
   arguments, read-only) and `/requests`. Record other applications' requests.
2. In Allow, confirm Wardoff remains alive with `layers.sleep=false` and no
   Wardoff requests. Establish the existing idle/display behavior without
   keyboard, mouse or remote-session activity that would reset the timers.
3. Enter Block. Confirm `layers.sleep=true` and the reason
   `Wardoff Block mode: prevent idle sleep and automatic display timeout.`
   in both requests. Wait past the configured idle/display timeouts; record
   actual behavior, not just request listing. Repeat Block/Allow/Block.
4. Request Sleep from Windows, then wake the machine. Check mode, request pair
   and logs; repeat for an already configured Sleep button, lid, and Hibernate.
   Explicit sleep proceeding is expected, not a failed veto test. Record whether
   the first observed resume renews the pair and whether subsequent notifications
   leave exactly one pair. No real transition is proven by synthetic messages.
5. Test the existing tray Sleep/Hibernate flow from Block and from Allow. Block
   should temporarily become Allow, then restore on wake; Allow should remain
   Allow. Check `block_restored_after_wake` and request presence. Test an explicit
   Allow command cancelling a pending restore where the timing permits it.
6. Include unattended/maintenance wake separately. This change retains the
   existing resume notification semantics; it does not promise a new
   presence-aware or display-aware policy. Record unexpected display behavior
   rather than silently widening the resume system.
7. Repeat applicable cases on Modern Standby with AC and DC, including an AC/DC
   change. Observe beyond the configured sleep timeout plus five minutes. Never
   refresh in a loop to evade Windows' battery policy. Record missing hardware
   as **not tested**, not as a pass or a claim that Modern Standby is unsupported.
8. Quit normally directly from Block, then separately terminate an owned test
   process directly from Block. Verify its requests disappear in both cases.
   Compare the power settings captured in step 1; no power setting should change.
   Existing UpdateOrchestrator/autostart behavior is outside this sleep-layer
   guarantee and should be tested separately in its documented environment.

Power-request acquisition should also be exercised non-elevated; these APIs do
not document a special elevation requirement. Elevation needed to inspect
`powercfg /requests` is a diagnostic requirement, not a runtime requirement.

## Acceptance checklist

- [ ] `tests\smoke_test.ps1` passes, with any skips noted
- [ ] Manual non-disruptive checks completed
- [ ] Admin-only checks completed or explicitly deferred
- [ ] Disruptive VM-only checks completed or explicitly deferred
- [ ] Tray sync, Explorer restart, handoff, and autostart tray/menu honesty checks completed or explicitly deferred
- [ ] Docs and PR wording stay within the current MVP and do not overstate local shutdown handling
