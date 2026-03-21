# Copilot instructions

This repository is currently in planning mode. The only checked-in project artifact is `PLAN.md`, and it is the source of truth for product scope and architecture until a Rust workspace is added. Treat the details below as planned design, not implemented code.

## Repository status and commands

- There is no checked-in `Cargo.toml`, source tree, CI workflow, or test suite yet.
- There are therefore no verified build, test, lint, or single-test commands to run today.
- Do not assume standard Rust commands exist until the workspace is scaffolded and committed.

## High-level architecture

- `Wardoff` is planned as a Windows-only Rust utility for preventing or aborting unwanted shutdown, reboot, sleep, hibernate, and related power transitions. The intended target is `x86_64-pc-windows-msvc`.
- The product has two control surfaces that should stay aligned:
  - a tray/background app with Block/Allow state, settings, notifications, and optional hidden mode
  - a CLI exposing `--block`, `--allow`, `--status`, `--hide`, `--log`, and `--aggressive`, with `--status` expected to produce JSON for scripting
- Logging and observability are first-class:
  - rotating JSON lines file logs
  - Windows Event Log integration
  - shared counters and metadata such as total blocked attempts, last blocked attempt, and likely source
- Shutdown handling is intentionally layered:
  1. standard interactive shutdown blocking through a message-only window that handles `WM_QUERYENDSESSION`, calls `ShutdownBlockReasonCreate()`, and raises shutdown priority with `SetProcessShutdownParameters()`
  2. local `shutdown.exe` protection, planned first via ETW detection plus `AbortSystemShutdown()`, with an opt-in aggressive IFEO mode later
  3. Windows Update reboot protection by disabling the scheduled task `Microsoft\Windows\UpdateOrchestrator\Reboot` and re-checking it periodically
  4. remote shutdown protection via repeated `AbortSystemShutdown(NULL)` polling
- Sleep/hibernate/display blocking is a separate toggle and is expected to use `SetThreadExecutionState(...)`.
- Autostart is planned via Task Scheduler rather than the `Run` registry key so elevated scenarios can be handled correctly.

## Project-specific conventions from `PLAN.md`

- Re-read `PLAN.md` before making major structural decisions. It currently stands in for a README, architecture doc, and roadmap.
- Preserve the distinction between MVP and later phases:
  - MVP is the safe, official-API release: standard shutdown blocking, UpdateOrchestrator handling, remote abort loop, tray basics, CLI basics, Task Scheduler autostart, sleep/hibernate/display blocking, and simple file logging
  - ETW monitoring, aggressive IFEO mode, Windows Event Log, profiles, timer, and toast notifications belong to later phases unless the plan is updated
- Be explicit about elevation boundaries. Update protection and aggressive `shutdown.exe` interception are planned as admin-only features; tray, UI, and CLI flows should surface that requirement clearly rather than failing silently.
- Prefer official Windows APIs first. Aggressive behavior is opt-in and should carry strong warnings because it may trigger EDR tooling.
- The plan explicitly targets `Microsoft\Windows\UpdateOrchestrator\Reboot`, not the older `MusNotification` approach.
- Do not hide platform limits. The plan is explicit that `shutdown /t 0 /f` cannot be blocked from user space; future code and docs should log that case honestly instead of claiming success.
- Keep the project scriptable and sysadmin-friendly:
  - machine-readable CLI output for status
  - log and event data that can be queried from PowerShell or SIEM tooling
  - Task Scheduler usage over ad-hoc startup hooks
- UI semantics in the plan matter: Block state is visually red, Allow is green, and profiles are named `Gaming`, `Work`, `Update Shield`, and `Custom`.

## Scope guardrails

These rules apply to EVERY change in this repo. Re-read them before starting any task.

### What is IN scope (MVP)
Only these features belong in the current phase:
- Layer 1: ShutdownBlockReasonCreate + WM_QUERYENDSESSION + SetProcessShutdownParameters
- Layer 3: Task Scheduler UpdateOrchestrator\Reboot disable/re-check
- Layer 4: AbortSystemShutdown polling loop
- Sleep/hibernate/screensaver blocking via SetThreadExecutionState
- Tray icon: Block/Allow toggle, right-click menu (Block, Allow, Shutdown, Reboot, Sleep, Hibernate, Quit)
- CLI: --block, --allow, --status (JSON output), --hide
- Auto-start via Task Scheduler
- File logging: rotating JSON lines (timestamp, event type, source, action, success/failure)
- README.md with technical documentation

### What is OUT of scope (v1.0 or later — do NOT implement)
- ETW monitoring (ferrisetw, Microsoft-Windows-Kernel-Process)
- IFEO aggressive mode
- Windows Event Log integration (EventLog provider)
- Toast notifications
- Timer functionality
- Profiles (Gaming, Work, Update Shield, Custom)
- Settings window / GUI beyond tray menu
- Winget/Scoop/Chocolatey packaging
- CI/CD pipelines, GitHub Actions workflows
- Release workflows, signing, packaging scripts
- Benchmarks, performance testing infrastructure

### Behavior rules for every commit
1. **No unrequested work.** Do not add features, files, configs, or infrastructure not explicitly asked for. No CI configs. No release workflows. No architectural refactoring beyond the current task.
2. **PLAN.md is the source of truth.** If a task contradicts PLAN.md, stop and ask. Do not silently deviate.
3. **One branch per fix/feature.** Create a branch named `feat/description` or `fix/description`, implement the change, then provide instructions to merge into the dev branch.
4. **No scope creep into v1.0 features.** If a task seems to require a v1.0 feature, flag it explicitly: "This requires [feature X] which is marked as v1.0 in PLAN.md. Should I proceed?"
5. **Document limitations honestly.** If something cannot be done (e.g., blocking `shutdown /t 0 /f`), document it in comments and README instead of implementing a hacky workaround.
6. **Admin boundaries are explicit.** Features requiring elevation must check for admin rights and fail with a clear message, not silently degrade.
7. **Test what you build.** After implementing a feature, compile it (`cargo build`) and verify it runs. If it requires Windows APIs that can only be tested at runtime, document what to test manually.

### Code conventions
- All public items must have `///` doc comments
- Use `log` crate macros (info!, warn!, error!) for all operational messages
- No `unwrap()` or `expect()` in production code paths — use proper error handling
- Module structure must match the layout defined in this file (see Module architecture below)
- Minimum Rust edition: 2021
- Target: x86_64-pc-windows-msvc only