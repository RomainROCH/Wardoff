# Copilot instructions

This repository is an active Windows-only Rust application. `PLAN.md` remains the source of truth for product scope and MVP boundaries, but the repo now also contains a checked-in Cargo project, source tree, and a PowerShell smoke test script.

## Read this first

When you start work in this repo, read in this order:

1. `README.md` for the current MVP snapshot
2. `PLAN.md` for current boundaries and prioritized next steps
3. `docs/ARCHITECTURE.md` for the code map
4. `docs/WINDOWS_SHUTDOWN_LAYERS.md` for detailed shutdown-layer behavior, especially Layer 2 ETW/local-shutdown nuance and limits
5. `CONTRIBUTING.md` for workflow and documentation expectations
6. this file for agent-specific guardrails

Also read `docs/BUSINESS_MODEL.md` before changing any README, changelog, contributor guidance, support copy, or other repo messaging that touches monetization, signed binaries, sponsorship, consulting/support, or commercialization decisions.

## Architecture authority guardrail

- Treat `docs/ARCHITECTURE.md` as the canonical architecture source of truth.
- Do not "align `docs/ARCHITECTURE.md` to the code" just because implementation drift exists.
- Only update that file to record intentional architectural changes, implementation deviations plus rationale, impossibilities discovered during implementation, or current status.

## Business-model authority guardrail

- Treat `docs/BUSINESS_MODEL.md` as the canonical business-model source of truth.
- Do not invent or "normalize" sponsorship, funding, signed-binary, paid-support, consulting, or commercialization messaging from scattered repo hints.
- Read `docs/BUSINESS_MODEL.md` before changing README, changelog, contributor guidance, release messaging, funding/support copy, or any other documentation that could imply a business decision.

## How to answer "what's next?"

If a user asks only **"what's next?"**, answer from repo docs instead of inventing a roadmap:

1. summarize the shipped MVP from `README.md`
2. state that the immediate priority is MVP hardening, validation, and documentation clarity from `PLAN.md`
3. mention later-phase backlog items only as backlog, not as shipped or currently committed work
4. stay conservative about local `shutdown.exe` handling and never promise that Wardoff stops `shutdown /t 0 /f`

## Repository status and commands

- The repo contains `Cargo.toml`, `src\`, and `tests\smoke_test.ps1`.
- Preferred build / validation commands:
  - `cargo check`
  - `cargo test`
  - `cargo build --release`
- `cargo build --release` produces `target\release\wardoff.exe`, typically around ~1.5 MiB on this branch.
- `cargo check` and `cargo test` should pass on this branch.
- The dedicated smoke test entry point is `powershell -ExecutionPolicy Bypass -File .\tests\smoke_test.ps1`.
- Do not add CI workflows, release automation, or packaging work unless explicitly requested.

## High-level architecture

- `Wardoff` is a Windows-only Rust utility for preventing or aborting unwanted shutdown, reboot, sleep, hibernate, and related power transitions. The intended target is `x86_64-pc-windows-msvc`.
- The product has two control surfaces that should stay aligned:
  - a tray/background app with Block/Allow state and optional hidden mode
  - a CLI exposing `--block`, `--allow`, `--status`, `--hide`, `--log`, `--tail`, `--autostart on|off`, and `--version`, with `--status` expected to produce JSON for scripting
- Keep the read-only CLI contract explicit: `--help`, `--version`, `--status`, and `--log --tail N` must remain non-elevating/local-or-read-only paths, while the default no-argument startup may still use the existing self-elevation path when needed.
- Keep the IPC split explicit when documenting CLI behavior: the session-scoped `WardoffControl` pipe is the state-changing path and the session-scoped `WardoffStatus` pipe is the dedicated read-only status path.
- Logging and observability are first-class:
  - rotating JSON lines file logs
  - shared counters and metadata such as total blocked attempts, last blocked attempt, and likely source
- Shutdown handling is intentionally layered:
  1. standard interactive shutdown blocking through a message-only window that handles `WM_QUERYENDSESSION`, calls `ShutdownBlockReasonCreate()`, and raises shutdown priority with `SetProcessShutdownParameters()`
  2. local `shutdown.exe` protection is not part of the current documented MVP surface
  3. Windows Update reboot protection by disabling the scheduled task `Microsoft\Windows\UpdateOrchestrator\Reboot` and re-checking it periodically when that task exists
  4. remote shutdown protection via repeated `AbortSystemShutdown(NULL)` polling
- Be explicit that Layer 3 can skip cleanly on editions such as LTSC where `\Microsoft\Windows\UpdateOrchestrator\Reboot` is absent; that is normal behavior, not a failure.
- For detailed layer-by-layer behavior and the cautious wording around ETW/local shutdown handling, read `docs/WINDOWS_SHUTDOWN_LAYERS.md`.
- Sleep/hibernate/display blocking is a separate toggle and is expected to use `SetThreadExecutionState(...)`.
- Autostart is handled via Task Scheduler rather than the `Run` registry key so elevated scenarios can be handled correctly.

## Implemented module structure

- `src/main.rs`: CLI dispatch, bootstrap, message loop, tray coordination, shutdown cleanup
- `src/cli.rs`: clap definitions, requested-action mapping, JSON status payloads
- `src/blocker/mod.rs`: blocker coordinator and shared power-action helpers
- `src/blocker/shutdown.rs`: Layer 1 shutdown/sign-out handling and shutdown block reason ownership
- `src/blocker/local.rs`: current local shutdown worker implementation; keep its user-facing documentation conservative and aligned with MVP scope decisions
- `src/blocker/update.rs`: UpdateOrchestrator task monitoring and restore logic
- `src/blocker/remote.rs`: Layer 4 `AbortSystemShutdownW(None)` polling loop
- `src/blocker/sleep.rs`: `SetThreadExecutionState(...)` blocker
- `src/blocker/abort.rs`: shared shutdown-abort privilege and result helpers
- `src/autostart.rs`: scheduled-task autostart management
- `src/instance.rs` and `src/ipc.rs`: single-instance ownership plus the named-pipe control and read-only status paths
- `src/logger\`: human logging plus rotating structured JSONL logging
- `src/tray\`: tray icon/menu surface and tray action plumbing
- `src/windows_util.rs`: elevation checks and relaunch helpers
- `src/config.rs`: app data and path helpers

## Project-specific conventions from `PLAN.md`

- Re-read `PLAN.md` before making major structural decisions. Use it as the roadmap and scope boundary document, while `README.md` stays the quickest current-state summary.
- Preserve the distinction between MVP and later phases:
  - MVP is the safe, official-API release: standard shutdown blocking, UpdateOrchestrator handling, remote abort loop, tray basics, CLI basics, Task Scheduler autostart, sleep/hibernate/display blocking, and simple file logging
  - aggressive IFEO mode, Windows Event Log, profiles, timer, and toast notifications belong to later phases unless the plan is updated
- Be explicit about elevation boundaries. Update protection and aggressive `shutdown.exe` interception are planned as admin-only features; tray, UI, and CLI flows should surface that requirement clearly rather than failing silently.
- Prefer official Windows APIs first. Aggressive behavior is opt-in and should carry strong warnings because it may trigger EDR tooling.
- The plan explicitly targets `Microsoft\Windows\UpdateOrchestrator\Reboot`, not the older `MusNotification` approach.
- Do not hide platform limits. The plan is explicit that `shutdown /t 0 /f` cannot be blocked from user space; future code and docs should log that case honestly instead of claiming success.
- Keep the project scriptable and sysadmin-friendly:
  - machine-readable CLI output for status
  - structured JSONL log data that can be queried from PowerShell or other tooling
  - Task Scheduler usage over ad-hoc startup hooks
- Keep README/architecture/copilot guidance aligned with the documented exit-code contract in README rather than inventing new CLI result semantics.
- UI semantics that matter in the current MVP: Block state is visually red and Allow is green.

## Scope guardrails

These rules apply to EVERY change in this repo. Re-read them before starting any task.

### What is IN scope (MVP)
Only these features belong in the current phase:
- Layer 1: ShutdownBlockReasonCreate + WM_QUERYENDSESSION + SetProcessShutdownParameters
- Layer 3: Task Scheduler UpdateOrchestrator\Reboot disable/re-check
- Layer 4: AbortSystemShutdown polling loop
- Sleep/hibernate/screensaver blocking via SetThreadExecutionState
- Tray icon: Block/Allow toggle, right-click menu (Block, Allow, Shutdown, Reboot, Sleep, Hibernate, Quit)
- CLI: --block, --allow, --status (JSON output), --hide, --log, --tail, --autostart on|off, --version
- Auto-start via Task Scheduler
- File logging: rotating JSON lines (timestamp, event type, source, action, success/failure)
- README.md with technical documentation

### What is OUT of scope (v1.0 or later — do NOT implement)
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
3. **One branch per fix/feature when a task actually requires repo changes.** If the task is explicitly read-only or branchless, follow that instruction instead of creating a branch. Otherwise create a focused branch such as `feat/description` or `fix/description`, implement the change, then provide instructions to merge into the dev branch.
4. **No scope creep into v1.0 features.** If a task seems to require a v1.0 feature, flag it explicitly: "This requires [feature X] which is marked as v1.0 in PLAN.md. Should I proceed?"
5. **Document limitations honestly.** If something cannot be done (e.g., blocking `shutdown /t 0 /f`), document it in comments and README instead of implementing a hacky workaround.
6. **Admin boundaries are explicit.** Features requiring elevation must check for admin rights and fail with a clear message, not silently degrade.
7. **Document user-facing status conservatively.** Do not describe IFEO, ETW local shutdown interception, Windows Event Log, toasts, timers, profiles, or settings as shipped user-facing features unless the plan is updated and the task explicitly asks for that.
8. **Test what you build.** After implementing a feature, compile it (`cargo build --release`) and verify it runs. If it requires Windows APIs that can only be tested at runtime, document what to test manually.

### Code conventions
- All public items must have `///` doc comments
- Use `log` crate macros (info!, warn!, error!) for all operational messages
- No `unwrap()` or `expect()` in production code paths — use proper error handling
- Module structure must match the layout defined in this file (see Implemented module structure above)
- Minimum Rust edition: 2021
- Target: x86_64-pc-windows-msvc only
