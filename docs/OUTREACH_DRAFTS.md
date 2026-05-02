# Outreach drafts

Internal reference drafts for a small early-adopter outreach wave. Keep these aligned with `README.md`, `PLAN.md`, `docs/EARLY_ADOPTER_OUTREACH_PLAN.md`, and `docs/BUSINESS_MODEL.md`. Here's the project : https://github.com/RomainROCH/Wardoff

## r/rust

**Title:** `Wardoff — a multi-layer Windows shutdown blocker in Rust (open source, MIT)`

I built Wardoff as a Windows-only Rust utility for a narrow problem: keeping a machine in a visible Block state when I do not want Windows to shut down, sign out, sleep, or reboot unattended.

The current MVP is source-first and already has a tray app plus CLI. Today it does interactive shutdown/sign-out blocking, protects the `\Microsoft\Windows\UpdateOrchestrator\Reboot` task when that task exists, does best-effort remote shutdown abort polling, blocks sleep / hibernate / display idle, and writes structured JSONL logs. It is meant for technically comfortable Windows users, not as a polished mass-market app.

I am trying to keep the claims narrow and honest. I do **not** claim Wardoff stops `shutdown /t 0 /f`, and I do not present local `shutdown.exe` handling as solved. Right now the useful part is the conservative Windows behavior, the visible Block/Allow state, and the scriptable control surface.

I would especially value feedback from Rust people who care about Windows API edge cases, runtime/tray/CLI design, and docs clarity for source-first tools. I am looking for criticism and practical feedback, not stars. Here's the project : https://github.com/RomainROCH/Wardoff

## r/sysadmin

**Title:** `Open-source tool to block unattended shutdowns/reboots on Windows (Wardoff)`

I built Wardoff because I wanted something simple and inspectable for the old problem of leaving a job running and coming back to find Windows restarted or went to sleep at the wrong time.

It is a Windows-only, source-first MIT project with a tray app and CLI. The current MVP gives me a visible Block/Allow state, autostart through Task Scheduler, machine-readable status output, and structured JSONL logs. It currently covers interactive shutdown/sign-out blocking, Update Orchestrator reboot-task protection when that scheduled task exists, best-effort remote shutdown abort polling, and sleep / hibernate / display-idle blocking.

I am deliberately keeping the wording narrow: I do **not** claim it stops `shutdown /t 0 /f`, and I am not presenting it as a polished enterprise product. The core behavior stays free and open; signed binaries are planned later as a convenience, not a paywall.

I am looking for early feedback from people who actually manage Windows machines. Would this be useful? What am I missing? Where would you expect it to fail, confuse people, or need better logging or admin/non-admin behavior? Here's the project : https://github.com/RomainROCH/Wardoff

## r/windows

**Title:** `Wardoff — stop Windows from restarting when you don't want it to (free, open-source)`

I made a small Windows utility called Wardoff for times when I want a machine to stay in a clear Block state instead of shutting down, signing out, sleeping, or rebooting at the wrong moment.

The current MVP is simple on purpose: tray icon, right-click to switch between Block and Allow, plus a CLI if I want status or logs. It is Windows-only and right now you need to build it from source, so I am mainly looking for technically comfortable early adopters rather than trying to do a broad launch.

I want to keep the limits explicit. I do **not** claim it can stop `shutdown /t 0 /f`, and I am not pretending this is a polished installer-style app yet. What it already does is give me a visible state, basic control, autostart support, and logs I can inspect when something behaves differently than expected.

If you try tools like this on Windows and have opinions about where the rough edges usually are, I would like that feedback. Here's the project : https://github.com/RomainROCH/Wardoff

## Hacker News (Show HN)

**Title:** `Show HN: Wardoff – Open-source Windows shutdown blocker in Rust`

I built Wardoff, a Windows-only Rust utility for keeping a machine in a visible Block state when I do not want shutdown, sign-out, sleep, hibernate, or certain reboot paths to interrupt work. The current MVP has a tray app, CLI control, structured JSONL logs, interactive shutdown/sign-out blocking, Update Orchestrator reboot-task protection when that task exists, remote shutdown abort polling, and sleep / hibernate / display-idle blocking.

I am keeping the claims narrow: it is source-first today, aimed at technically comfortable early adopters, and it does **not** claim to stop `shutdown /t 0 /f`. Source is MIT and free. Signed binaries are planned later as a convenience purchase, not a paywall. I would value feedback on Windows edge cases, docs clarity, and whether the current tray + CLI shape is the right one. Here's the project : https://github.com/RomainROCH/Wardoff
