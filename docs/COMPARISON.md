# Comparison

> Status note: this comparison reflects Wardoff's current 0.1.0-era MVP claims, not later-phase plans.

## Summary table

| Product | Source model | Current focus | What Wardoff can honestly claim today | Important gaps or planned items |
| --- | --- | --- | --- | --- |
| Wardoff | MIT, open source | Transparent shutdown and power-state control for modern Windows | Interactive shutdown/sign-out blocking, Update Orchestrator reboot-task protection, remote abort polling, sleep/hibernate/display-idle blocking, tray UI, CLI, JSONL logs, single-instance IPC, autostart | No IFEO mode, no Windows Event Log integration, and no promise to stop local `shutdown /t 0 /f` |
| ShutdownBlocker | Closed freeware | Traditional shutdown blocking utility | Comparable motivation around shutdown prevention | Closed implementation, older maintenance history, and behavior cannot be audited here |
| ShutdownGuard | MIT, open source | Historical shutdown-blocking approach | Open-source reference point | Archived and unsupported |
| Don't Sleep | Closed freeware | Keep-awake / power-state prevention | Strong overlap on sleep-prevention use cases | Not positioned as a transparent layered shutdown-control tool |
| PreventTurnOff | Closed freeware | Simpler keep-awake utility | Similar overlap on power-state prevention | Narrower scope and closed implementation |

## Wardoff

Wardoff's current MVP is best described as a conservative, transparent Windows runtime rather than a "blocks everything" utility.

### What the MVP really includes today

- Layer 1 interactive shutdown and sign-out blocking
- Layer 3 protection for `\Microsoft\Windows\UpdateOrchestrator\Reboot`
- Layer 4 best-effort remote shutdown abort polling
- sleep, hibernate, and display-idle blocking via `SetThreadExecutionState(...)`
- tray controls for Block, Allow, Shutdown, Reboot, Sleep, Hibernate, and Quit
- CLI control surface for `--block`, `--allow`, `--hide`, `--status`, `--log`, `--tail`, and `--autostart`
- structured rotating JSONL logs
- single-instance coordination plus named-pipe control channel
- Task Scheduler autostart management

### What Wardoff does **not** currently claim as shipped user-facing MVP behavior

- aggressive IFEO interception
- Windows Event Log integration
- toast notifications
- timers
- profiles
- settings UI

### Important nuance about ETW and IFEO

Wardoff's repository currently contains ETW-based local shutdown code in `src/blocker/local.rs`, but the project is deliberately **not** treating that path as a documented 0.1.0 headline feature.

Why this matters:

- ETW local-shutdown handling is reactive and race-sensitive
- the project still does **not** promise to stop local `shutdown /t 0 /f`
- current top-level user-facing docs keep that boundary conservative on purpose

IFEO is even further from the current MVP:

- it is a planned later-phase design
- it is not implemented as a supported feature
- it should be discussed as future aggressive interception, not current product behavior

So the current short version is:

- **ETW local interception exists in repo code, but is not currently documented as a supported headline MVP feature**
- **IFEO is planned and not implemented**

## ShutdownBlocker

Based on the project notes, ShutdownBlocker appears to be:

- closed freeware
- associated with older `shutdown.exe` interception approaches
- last notably updated years ago

Compared with that, Wardoff's current advantage is not that it claims broader magic. The difference is that Wardoff documents its limits openly:

- it explains what each layer covers
- it keeps admin boundaries explicit
- it does not market the hardest local forced-shutdown case as solved

## ShutdownGuard

ShutdownGuard remains useful as a historical open-source reference, but it is archived and unsupported.

Wardoff differs by focusing on:

- current Windows 10/11 behavior
- observable runtime state
- tray plus CLI coordination
- explicit update-reboot handling

## Don't Sleep

Don't Sleep is still relevant when the main goal is simply to prevent:

- sleep
- standby
- hibernate
- display idle

Wardoff now overlaps with that use case through implemented execution-state blocking, but Wardoff's core value proposition is broader:

- shutdown-aware runtime state
- CLI and automation support
- documented limits
- modern Windows update-reboot awareness

## PreventTurnOff

PreventTurnOff sits even closer to the lightweight keep-awake end of the spectrum.

Compared with Wardoff, it is better thought of as:

- a simpler power-state utility
- not a layered shutdown-control runtime
- not an auditable open-source implementation

## Bottom line

Wardoff's current 0.1.0 positioning is:

- open and auditable
- conservative in what it claims
- useful today for interactive shutdown blocking, update-task protection, remote abort polling, and sleep prevention
- intentionally cautious about local `shutdown.exe`

That last point is the most important comparison decision:

- Wardoff does **not** currently market ETW local interception as a shipped flagship feature
- Wardoff does **not** claim IFEO today
- Wardoff does **not** claim to stop `shutdown /t 0 /f`

That honesty is part of the product positioning, not a gap in the docs.
