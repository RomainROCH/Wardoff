# Contributing to Wardoff

Thanks for contributing to Wardoff.

This repository now contains a working Windows MVP runtime, but the docs should remain conservative: describe what is clearly implemented and supported, and do not turn later-phase ideas into shipped features.

## Prerequisites

For local development on Windows, use:

- Windows 10 or Windows 11
- Rust stable
- MSVC Build Tools for the `x86_64-pc-windows-msvc` target
- Git

Administrator rights are required for some manual validation paths and some runtime features, especially Update Orchestrator task handling and autostart task changes.

## Build

The main build command to use and document is:

```powershell
cargo build --release
```

Useful supporting commands:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
```

## Smoke test

Run the repo smoke test with:

```powershell
powershell -ExecutionPolicy Bypass -File tests\smoke_test.ps1
```

Use a disposable VM for disruptive shutdown, reboot, sleep, hibernate, or Task Scheduler validation.

## Current implementation boundaries

Keep documentation and PR descriptions aligned with the current supported MVP surface:

- interactive shutdown/sign-out blocking is implemented
- Update Orchestrator reboot-task protection is implemented
- remote shutdown abort polling is implemented
- sleep/hibernate/display-idle blocking is implemented
- tray, CLI, logging, IPC, autostart, and single-instance coordination are implemented

Do **not** claim the following as implemented unless your change really adds and validates them:

- IFEO interception
- Windows Event Log integration
- toast notifications
- timers
- profiles
- settings UI

Be especially careful with local `shutdown.exe` wording:

- do not promise that Wardoff blocks `shutdown /t 0 /f`
- do not turn experimental or cautiously documented behavior into a marketing claim

## Branch workflow

Use this workflow unless a maintainer tells you otherwise:

1. branch from `dev`
2. use a branch name such as `feat/xxx` or `fix/xxx`
3. merge completed work back into `dev`
4. open the PR to `main` from the appropriate integrated branch state

Keep changes focused and reviewable.

## Pull request checklist

In each PR:

- explain what changed
- explain what you tested
- call out any admin requirement
- call out any behavior that is intentionally still planned rather than shipped
- keep docs and code wording consistent

## Coding and documentation conventions

- keep documentation in English
- prefer `cargo build --release` when describing the real build process
- use fenced code blocks with language tags
- use `rustfmt` and `clippy`
- avoid `unwrap()` and `expect()` in production paths where failure should be surfaced cleanly
- add `///` comments to public Rust items where appropriate
- keep claims honest and source-verifiable

Small, accurate PRs are much easier to review than broad speculative rewrites.
