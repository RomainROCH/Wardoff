# Contributing to Wardoff

Thanks for helping shape Wardoff.

This repository now contains a working MVP-level Windows runtime in addition to the planning documents. `PLAN.md` remains the product and architecture source of truth for scope, but documentation and code changes should describe the current implementation accurately: the safe MVP layers are present, including Layer 2 standard ETW monitoring on this branch, while aggressive IFEO / Event Log / toast / timer / profile / settings work is still future scope.

## Development prerequisites

For code work on Windows, use:

- Windows 10 or Windows 11
- Rust stable with the MSVC toolchain (`x86_64-pc-windows-msvc`)
- Visual Studio Build Tools 2022 or equivalent MSVC C/C++ build tools
- Git

Administrator rights are required for some current functionality as well, especially Layer 3 Update Orchestrator task control and autostart task changes. Do not hide missing elevation; surface it clearly and document the degraded behavior honestly.

## Building

The expected local workflow is:

```powershell
cargo build
cargo check
```

For a release-style local build:

```powershell
cargo build --release
```

## Testing

Use Windows 10+ or Windows 11 for validation. A disposable VM is strongly recommended for any shutdown, reboot, sleep, hibernate, or Task Scheduler experiments.

For documentation-only changes:

- proofread for clear English
- verify links and file paths
- keep claims aligned with the current repository state

For code changes:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
```

Add manual Windows notes for anything that cannot be meaningfully covered by automated tests, especially:

- standard interactive shutdown blocking
- Windows Update reboot handling
- remote shutdown behavior when a timeout is present
- non-admin versus admin behavior
- sleep, hibernate, and display-idle prevention
- tray behavior, including hidden versus visible startup
- CLI interactions with the primary instance over named-pipe IPC
- Task Scheduler autostart behavior

Do not test disruptive shutdown scenarios on a machine you cannot afford to interrupt.

## Conventions

- Keep all documentation in English.
- Treat `PLAN.md` as the product and architecture source of truth.
- Keep MVP work limited to the safe, official-API scope currently described in the plan unless the plan is deliberately updated.
- Document limitations honestly, especially around `shutdown /t 0 /f` and administrator boundaries.
- Do not describe aggressive IFEO, Windows Event Log, toast notifications, timers, profiles, or a settings window as implemented unless you actually add them.
- Use `rustfmt` and `clippy` for Rust code.
- Avoid `unwrap()` and `expect()` in production paths.
- Add `///` doc comments to public Rust items.
- Target `x86_64-pc-windows-msvc`.

## Pull request process

1. Read `PLAN.md` and `.github/copilot-instructions.md` before starting.
2. Keep the branch and pull request narrowly scoped.
3. Use branch names such as `feat/description` or `fix/description`.
4. Explain what changed, what was tested, and what remains planned.
5. Call out admin requirements, platform limits, and any manual verification steps.
6. Do not mix the current safe MVP with v1.0-or-later features unless the plan is explicitly updated.

Small, focused pull requests are easier to review and safer for a Windows system utility.
