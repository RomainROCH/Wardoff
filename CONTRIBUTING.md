# Contributing to Wardoff

Thanks for helping shape Wardoff.

This repository is currently in a planning and documentation phase. `PLAN.md` is the technical source of truth, and `.github/copilot-instructions.md` defines scope guardrails. Before changing anything, read both files and keep the distinction between the MVP and later phases. A minimal Rust scaffold may also be present, but placeholder code should not be described as implemented shutdown functionality.

## Development prerequisites

For code work on Windows, use:

- Windows 10 or Windows 11
- Rust stable with the MSVC toolchain (`x86_64-pc-windows-msvc`)
- Visual Studio Build Tools 2022 or equivalent MSVC C/C++ build tools
- Git

Administrator rights are expected for any future work that touches Windows Update task control or other elevation-boundary features. Do not hide missing elevation; surface it clearly.

## Building

At the time of writing, this repository may contain planning documents and a minimal Rust scaffold.

When the Rust workspace is present, the expected local workflow is:

```powershell
cargo build
cargo check
```

If no `Cargo.toml` is checked in yet, there is nothing to compile. If a placeholder workspace is present, treat successful compilation as a scaffold check, not proof that shutdown-blocking behavior exists.

## Testing

Use Windows 10+ or Windows 11 for validation. A disposable VM is strongly recommended for any shutdown, reboot, sleep, hibernate, or Task Scheduler experiments.

For documentation-only changes:

- proofread for clear English
- verify links and file paths
- keep claims aligned with the current repository state

For code changes once the workspace exists:

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

Do not test disruptive shutdown scenarios on a machine you cannot afford to interrupt.

## Conventions

- Keep all documentation in English.
- Treat `PLAN.md` as the product and architecture source of truth.
- Keep MVP work limited to the safe, official-API scope currently described in the plan.
- Document limitations honestly, especially around `shutdown /t 0 /f` and administrator boundaries.
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
6. Do not mix MVP work with v1.0 or later features unless the plan is explicitly updated.

Small, focused pull requests are easier to review and safer for a Windows system utility.
