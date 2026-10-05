# Development guide

This guide helps humans and agents make a focused change without reading the
whole repository. [CONTRIBUTING.md](../CONTRIBUTING.md) owns workflow and coding
conventions; [ARCHITECTURE.md](ARCHITECTURE.md) owns the design.

## First working checkout

Use Windows 10/11 with Rust stable, the `x86_64-pc-windows-msvc` toolchain, MSVC
C++ build tools, a Windows SDK, Git and PowerShell 5.1+. From the repository root:

```powershell
rustc -Vv
cargo -V
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
```

The first Cargo invocation downloads locked dependencies. Keep `Cargo.lock`
checked in; routine validation uses `--locked` so it cannot silently resolve a
different dependency set. Cargo commands use the configured target directory.
The default release output is `target/release/wardoff.exe`; an explicit
`CARGO_BUILD_TARGET` adds a target-triple subdirectory.

Building does not launch Wardoff. `cargo run` does: default startup can ask for
elevation and arm protection. For runtime testing, first read the
[validation matrix](MVP_VALIDATION_MATRIX.md).

## Task-to-code map

Open the owner and its tests first. Expand to the companion when the change
crosses that boundary. Unit tests live in `#[cfg(test)]` modules beside the code.

| Task | First code | Companion / acceptance |
| --- | --- | --- |
| CLI arguments, conflicts, JSON status | [cli.rs](../src/cli.rs) | [main.rs](../src/main.rs) dispatch; README command contract |
| Startup mode, tray visibility, command mapping | [runtime_policy.rs](../src/runtime_policy.rs) | `run_main`, `handle_primary_runtime_request` in main |
| Elevation, console detach, relaunch | [windows_util.rs](../src/windows_util.rs) | `prepare_default_launch`, `prepare_runtime_console_launch` in main; smoke manifest/CLI checks |
| Mode transitions, activation rollback | [blocker/mod.rs](../src/blocker/mod.rs) | `Application::set_mode` in main; affected layer |
| Interactive shutdown and Windows callbacks | [blocker/shutdown.rs](../src/blocker/shutdown.rs) | main callbacks; VM-only shutdown acceptance |
| Idle-power requests, cleanup and resume | [blocker/sleep.rs](../src/blocker/sleep.rs) | runtime policy wake tests; main `handle_power_broadcast`; physical resume checks |
| Update reboot task | [blocker/update.rs](../src/blocker/update.rs) | validation matrix admin/absent-task cases |
| Remote abort / local ETW | [blocker/remote.rs](../src/blocker/remote.rs), [blocker/local.rs](../src/blocker/local.rs) | [abort.rs](../src/blocker/abort.rs); shutdown-layer limits |
| Control/status transport, retries, access checks | [ipc.rs](../src/ipc.rs) | main `handle_ipc_request`; security audit residual risks |
| Session names and singleton handoff | [session_scope.rs](../src/session_scope.rs), [instance.rs](../src/instance.rs) | main `forward_request_to_primary`; handoff acceptance |
| Tray menu, Explorer restart, icon | [tray/mod.rs](../src/tray/mod.rs), [tray/icon.rs](../src/tray/icon.rs) | main `handle_tray_action`; manual tray checks |
| Autostart / trusted executable path | [autostart.rs](../src/autostart.rs) | runtime policy checkbox fallback; elevated smoke checks |
| JSONL events, rotation and tail | [logger/mod.rs](../src/logger/mod.rs) | [log_tail_cli.rs](../tests/log_tail_cli.rs) exercises real CLI with isolated log fixtures |
| Release manifest | [build.rs](../build.rs), [wardoff.manifest](../wardoff.manifest) | release build and smoke manifest check |
| Developer checks and docs links | [check.ps1](../scripts/check.ps1), [check-docs.ps1](../scripts/check-docs.ps1) | [developer_checks.ps1](../tests/developer_checks.ps1) |
| Safe smoke setup and cleanup | [smoke_test.ps1](../tests/smoke_test.ps1) | [smoke_safety.ps1](../tests/smoke_safety.ps1); validation matrix |

## Runtime flow and invariants

`cli::parse_cli` determines intent. `main::run_main` handles read-only status/log
before runtime bootstrap. Runtime commands claim the session mutex: a secondary
forwards to IPC; a primary prepares elevation/console behavior and calls
`bootstrap`, then `run`.

The main application thread owns mode changes and cleanup. Pipe workers and the
tray thread send work back to it. Preserve this ownership when changing callbacks:
the global application pointer is only valid while `run` owns the application
and the callbacks are registered.

`runtime_policy` contains decisions without Windows effects: initial mode/tray
surface, secondary command mapping, wake restoration and autostart checkbox
fallback. It must not acquire handles, spawn threads, access tasks or log events.
Effects remain in the existing runtime and subsystem owners.

The two pipes have different contracts: status reads must remain available
without elevation; control commands retain same-user/session access checks.
The ordinary inactive status path checks the session mutex and returns without
pipe retries. If the mutex is occupied but neither pipe responds, the sequential
fallback can take about four seconds; that case is tracked in [PLAN.md](../PLAN.md).

## Validation

| Scope | Command | What it establishes |
| --- | --- | --- |
| Normal iteration | `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1` | Docs, formatting, compilation and unit tests |
| Rust change ready for review | `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Mode Full` | Fast checks, strict Clippy, release build |
| Documentation only | `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Mode Docs` | Local Markdown destinations exist; valid UTF-8 without NUL bytes |
| Developer scripts | `powershell -NoProfile -ExecutionPolicy Bypass -File tests/developer_checks.ps1` | Checker success/failure paths in temporary fixtures |
| Smoke safety helpers | `powershell -NoProfile -ExecutionPolicy Bypass -File tests/smoke_safety.ps1` | Guard and cleanup behavior with fake processes/tasks; no runtime launched |
| Runtime integration | `powershell -NoProfile -ExecutionPolicy Bypass -File tests/smoke_test.ps1` | State-changing Windows integration; isolated environment |

The check script resolves the repository from its own location, stops at the
first failed command and returns a nonzero exit code. It does not install tools
or run the smoke suite. Cargo tests include brief real Windows Power Request
acquisition/release and token checks, plus read-only CLI subprocesses with
isolated log fixtures. They do not launch the long-lived runtime, edit scheduled
tasks or cause a physical power transition.

Individual Cargo commands, useful for focused diagnosis:

```powershell
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --release --locked
```

Examples of focused tests:

```powershell
cargo test --locked cli::tests
cargo test --locked runtime_policy::tests
cargo test --locked blocker::sleep::tests
cargo test --locked ipc::tests
cargo test --locked instance::tests
cargo test --locked session_scope::tests
cargo test --locked autostart::tests
cargo test --locked --test log_tail_cli
```

The docs checker covers tracked and untracked, non-ignored Markdown. It checks
local link destinations, not heading fragments, external websites or factual
accuracy. Review those explicitly when changing them. Dated benchmark scripts
remain separate from development checks.

Both [CI](../.github/workflows/ci.yml) and the
[release workflow](../.github/workflows/release.yml) run the same Full check,
developer-tooling fixtures, smoke-safety fixtures and the real smoke suite on
Windows. Full includes the brief native Power Request test and does not run the
state-changing smoke suite. A failed required check prevents draft creation.

CI retains one standard Windows job; tag releases separate read-only validation
from a draft-only writer, with no added matrix or scheduled runs. The checked
binary is reused by smoke without compilation. See [RELEASING.md](RELEASING.md)
for provenance, exact-artifact gates and manual publication. Standard GitHub-hosted runners are
[free for public repositories](https://docs.github.com/en/actions/concepts/billing-and-usage).
CI binary artifacts expire after seven days. Reassess billing before making the
repository private or moving to a larger or paid runner.

## Common problems

| Symptom | Next step |
| --- | --- |
| Linker or resource compiler missing | Check MSVC C++ tools and Windows SDK; use the MSVC toolchain |
| Building from Linux/WSL fails | Build on Windows; this repository intentionally uses Windows-only APIs |
| Release executable locked | Quit the repo-built app deliberately, or use a separate `CARGO_TARGET_DIR`; do not kill an unrelated runtime |
| Cargo waits for its build lock | Let the owning build finish; avoid concurrent builds sharing a target directory |
| Formatting failure | Run `cargo fmt --all`, then inspect the diff |
| `--locked` rejects dependency resolution | Check whether the task intentionally changed Cargo.toml; update and review the lockfile only for an intended dependency change |
| Smoke refuses an existing runtime or autostart task | Use an isolated environment with neither present; do not remove someone else's state to make the test pass |
| Unit tests pass but tray/power behavior is uncertain | Use the specific manual row in the validation matrix and report the environment |

## Keep the next change easy

Update this map when an owner moves. Put a short module comment beside
non-obvious lifecycle boundaries and test pure decisions near their owner.
Keep measured evidence separate from intended contracts. A human should be
able to use the same instructions and review the same evidence as an agent.
