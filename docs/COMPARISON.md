# Comparison

> Status note: Wardoff is currently a planning and documentation scaffold. The comparison below explains the gap the project intends to fill; it is not a claim that Wardoff already ships those capabilities today.

## Summary table

| Product | Source model | Stack | Main mechanisms | What is missing or limited | Current status |
| --- | --- | --- | --- | --- | --- |
| Wardoff | MIT, open-source project | Rust (planned) | Layered architecture: standard shutdown blocking, Update Orchestrator task control, remote abort polling, future ETW/IFEO | No runtime implementation yet; local `shutdown /t 0 /f` remains a hard user-space limit without the future aggressive mode | Planning/documentation stage |
| ShutdownBlocker | Closed freeware | .NET Framework 4.0 | Standard shutdown blocking plus IFEO handling for `shutdown.exe` | Closed implementation, last known update in March 2017, no open audit trail for cleanup or admin-boundary handling | Mature but stagnant |
| ShutdownGuard | MIT, open source | Pure C built with MinGW | Classic shutdown blocking with DLL injection | Archived as `UNSUPPORTED` since 2014, older technique, not aimed at modern Update Orchestrator behavior | Archived |
| Don't Sleep | Closed freeware; reverse engineering forbidden | Closed-source Windows utility (stack not auditable) | Power-state prevention focused on sleep/standby/hibernate/display behavior | Does not block `shutdown.exe`, no Windows Event Log integration, implementation cannot be audited | Actively maintained |
| PreventTurnOff | Closed freeware; reverse engineering forbidden | Closed-source Windows utility (stack not auditable) | Simplified version of the Don't Sleep-style prevention approach | Same limits as Don't Sleep, reduced scope, no Event Log, no `shutdown.exe` interception | Actively maintained, simplified |

## Detailed notes

### Wardoff

Wardoff's planned differentiators are architectural transparency and modern Windows-specific scope:

- open source under MIT
- explicit documentation of all shutdown layers and their limits
- focus on `Microsoft\Windows\UpdateOrchestrator\Reboot` rather than older update-era assumptions
- scriptable CLI and structured logging plans for sysadmin use

Important reality check:

- the repository does not yet implement the runtime blocker
- the comparison is about intended positioning, not shipped parity

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

Wardoff's planned difference is not "more magic," but better transparency: clear documentation, explicit admin boundaries, and honest discussion of what user space still cannot do.

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

Gaps relative to Wardoff's plan:

- archived and unsupported
- based on an older technical era
- not positioned around modern Windows Update reboot behavior
- no emphasis on current structured observability goals

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

Wardoff overlaps with the "stay awake" use case through planned `SetThreadExecutionState(...)` handling, but its primary aim is broader shutdown control and documentation.

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

The plan behind Wardoff is not just "another blocker." It is meant to be:

- open and auditable
- explicit about the difference between MVP and later phases
- honest about `shutdown /t 0 /f`
- focused on Windows 10/11 update behavior, especially `UpdateOrchestrator\Reboot`
- scriptable and observable for sysadmins, not only desktop users

That positioning matters because the current alternatives tend to force a trade-off:

- open but abandoned
- maintained but closed
- good at sleep prevention but weak on shutdown transparency
- able to hook aggressively but without modern, documented boundaries

Wardoff's current repository stage is still early, but the documentation is intentionally setting those expectations now rather than after code exists.
