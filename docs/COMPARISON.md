# Comparison

> Status note: Wardoff now ships an MVP-level runtime on the current branch. The comparison below reflects what is actually implemented today, not just the long-term plan.

## Summary table

| Product | Source model | Stack | Main mechanisms | What is missing or limited | Current status |
| --- | --- | --- | --- | --- | --- |
| Wardoff | MIT, open-source project | Rust | Layer 1 interactive blocking, Layer 3 Update Orchestrator task control, Layer 4 remote abort polling, sleep/display blocking, tray, CLI, JSONL logs, autostart, single-instance IPC | No Layer 2 local `shutdown.exe` interception yet; no ETW, no IFEO, no Windows Event Log, and current builds do **not** block local `shutdown /t 0 /f` | MVP runtime implemented |
| ShutdownBlocker | Closed freeware | .NET Framework 4.0 | Standard shutdown blocking plus IFEO handling for `shutdown.exe` | Closed implementation, last known update in March 2017, no open audit trail for cleanup or admin-boundary handling | Mature but stagnant |
| ShutdownGuard | MIT, open source | Pure C built with MinGW | Classic shutdown blocking with DLL injection | Archived as `UNSUPPORTED` since 2014, older technique, not aimed at modern Update Orchestrator behavior | Archived |
| Don't Sleep | Closed freeware; reverse engineering forbidden | Closed-source Windows utility (stack not auditable) | Power-state prevention focused on sleep/standby/hibernate/display behavior | Does not block `shutdown.exe`, no Windows Event Log integration, implementation cannot be audited | Actively maintained |
| PreventTurnOff | Closed freeware; reverse engineering forbidden | Closed-source Windows utility (stack not auditable) | Simplified version of the Don't Sleep-style prevention approach | Same limits as Don't Sleep, reduced scope, no Event Log, no `shutdown.exe` interception | Actively maintained, simplified |

## Detailed notes

### Wardoff

Wardoff's current differentiators are now partly implemented rather than only planned:

- open source under MIT
- explicit documentation of all shutdown layers and their limits
- working tray/runtime with Block and Allow state
- scriptable CLI with `--block`, `--allow`, `--status`, `--hide`, `--log`, `--tail`, and `--autostart on|off`
- structured rotating JSONL logs
- single-instance coordination with named-pipe IPC
- focus on `Microsoft\Windows\UpdateOrchestrator\Reboot` rather than older update-era assumptions

Important reality check:

- the current branch implements the safe MVP layers only: Layer 1, Layer 3, Layer 4, plus sleep/display blocking
- Layer 2 is still missing, so local `shutdown.exe` interception is not shipped
- current Wardoff therefore does **not** block local `shutdown /t 0 /f`
- Windows Event Log integration, toast notifications, timers, profiles, settings UI, and packaging polish are still future work

### ShutdownBlocker

Known profile from the project brief:

- closed freeware
- built on .NET Framework 4.0
- last known update: March 2017
- uses IFEO for `shutdown.exe`

What that means in practice:

- it addresses a real gap, especially around local `shutdown.exe`
- but the implementation is not auditable
- its age raises questions about modern Windows 10/11 assumptions, maintenance, and cleanup behavior

Wardoff's current difference is not "more magic," but better transparency: the safe layers are implemented in the open, the current limitations are documented, and the riskier `shutdown.exe` interception path is still explicitly deferred.

### ShutdownGuard

Known profile from the project brief:

- open source under MIT
- pure C
- built with MinGW
- archived as `UNSUPPORTED` since 2014
- used DLL injection

Strengths:

- open-source lineage
- understandable historical reference point for shutdown blocking

Gaps relative to Wardoff's current direction:

- archived and unsupported
- based on an older technical era
- not positioned around modern Windows Update reboot behavior
- no emphasis on current structured observability, tray/runtime coordination, or named-pipe control

### Don't Sleep

Known profile from the project brief:

- closed freeware
- actively maintained
- reverse engineering forbidden
- does not block `shutdown.exe`
- no Windows Event Log integration

Practical interpretation:

- useful when the goal is "keep the machine awake"
- not a full answer to shutdown/reboot control
- impossible to audit deeply because the implementation is closed

Wardoff now overlaps with this use case through implemented `SetThreadExecutionState(...)` handling, but Wardoff's broader aim remains layered shutdown control and transparent documentation.

### PreventTurnOff

Known profile from the project brief:

- simplified version of Don't Sleep
- same closed/freeware model and reverse-engineering restriction
- same core limitations

Practical interpretation:

- even narrower scope than Don't Sleep
- not a substitute for transparent shutdown handling
- still does not solve `shutdown.exe` interception or Windows Event Log visibility

## Why Wardoff is positioned differently

Wardoff is not just "another blocker." The current branch already tries to be:

- open and auditable
- explicit about the difference between shipped MVP behavior and later phases
- honest about `shutdown /t 0 /f`
- focused on Windows 10/11 update behavior, especially `UpdateOrchestrator\Reboot`
- scriptable and observable for sysadmins, not only desktop users

That positioning matters because the current alternatives tend to force a trade-off:

- open but abandoned
- maintained but closed
- good at sleep prevention but weak on shutdown transparency
- able to hook aggressively but without modern, documented boundaries

Wardoff still has meaningful gaps, especially around local `shutdown.exe`, but it has moved beyond the planning-only stage and already covers a practical safe-MVP subset.
