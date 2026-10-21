# Contributing to Wardoff

Thanks for contributing to Wardoff. Please also review the
[Code of Conduct](CODE_OF_CONDUCT.md).

This repository now contains a working Windows MVP runtime, but the docs should remain conservative: describe what is clearly implemented and supported, and do not turn later-phase ideas into shipped features.

## Read this first

If you are new to the repo, read these before making changes:

1. `README.md` for the current MVP summary
2. `PLAN.md` for scope boundaries and the current "what's next" answer
3. `docs/ARCHITECTURE.md` for the code map

## Business-model messaging guardrail

If your change touches sponsor, funding, signed-binary, paid-support, consulting, or commercialization messaging, read `docs/BUSINESS_MODEL.md` first and keep the wording aligned with it. Do not invent offers or promises ad hoc.

## Prerequisites

For local development on Windows, use:

- Windows 10 or Windows 11
- Rust stable
- MSVC Build Tools for the `x86_64-pc-windows-msvc` target
- Git

Administrator rights are required for some manual validation paths and some runtime features, especially Update Orchestrator task handling and autostart task changes.

## Safe source validation

Use the explicit Safe entry point for source checks that do not acquire native
PowerRequests or start the protection runtime:

```powershell
pwsh -NoProfile -File scripts/validate-safe.ps1
```

Its exact coverage, receipts and remaining qualification requirements are in
[docs/VALIDATION_GATES.md](docs/VALIDATION_GATES.md). Report excluded checks by
name. A Safe PASS is not Full, runtime, hardware, package or release acceptance.
The current CI/documentation preparation does not qualify a new binary; retained
package evidence remains bound to its recorded source SHA and executable hash.

## Build

To build development sources with the locked dependency graph:

```powershell
cargo build --release --locked
```

The binary is `target\release\wardoff.exe`. Cargo currently declares `0.2.0`,
which does not make this development build the existing public v0.2.0 asset.
A successful build alone does not qualify a release or authorize publication.

The checked-in `.cargo/config.toml` enables `crt-static` for
`x86_64-pc-windows-msvc`, so the intended single-executable distribution does not
need a separate MSVC runtime installation. Windows system DLLs remain dependencies.
`RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` override Cargo's target flags; if either
is set, retain `-C target-feature=+crt-static` in those flags.

After building, inspect the exact executable's imports (for example,
`dumpbin /dependents target\release\wardoff.exe` from an MSVC developer shell).
Reject a portable package that still imports `VCRUNTIME*.dll` or `MSVCP*.dll`,
then check `--help` and `--version` in a clean Windows guest before the authorized
native smoke. Static linkage must be verified on the resulting bytes; the source
setting alone does not establish that a package starts or that its protections work.

## Native and system validation

Unfiltered `cargo test` includes
`blocker::sleep::tests::activation_reports_an_acquired_request_before_returning`,
which acquires and releases real PowerRequests. It is explicitly excluded from
Safe. The separate `tests/smoke_test.ps1` starts Block/Allow runtimes and can
modify scheduled tasks; it is not a read-only source check.

Run those operations only with explicit authorization for their effects in the
chosen isolated environment. Use a disposable VM for disruptive shutdown,
reboot or Task Scheduler cases, and suitable hardware for physical sleep,
display timeout, Hibernate or Modern Standby validation. Consult
[the validation matrix](docs/MVP_VALIDATION_MATRIX.md) and
[the qualification gates](docs/VALIDATION_GATES.md); record every skip.

Full/release qualification remains blocked while required native, system smoke,
portable CLI, exact-package and hardware evidence is missing. Preserve the
distinction between the production executable and a CLI test build with changed
IPC/mutex prefixes. Neither source tests nor another binary's VM/benchmark
results qualify the future package. Do not weaken required gates, move tags or
overwrite published assets to make delivery proceed.

## Current implementation boundaries

Keep documentation and PR descriptions aligned with the current supported MVP surface:

- interactive shutdown/sign-out blocking is implemented
- Update Orchestrator reboot-task protection is implemented
- remote shutdown abort polling is implemented
- idle-sleep and automatic display-timeout prevention are implemented; explicit Sleep/Hibernate remain controlled by Windows
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

The native smoke now requires PowerShell 7, an explicit existing executable and
its expected SHA256; it never rebuilds. Run only its simulated safety tests
locally. The exact commands, disposable-guest prerequisites and effects needing
separate authorization are in [SMOKE_PROTOCOL.md](docs/SMOKE_PROTOCOL.md).
