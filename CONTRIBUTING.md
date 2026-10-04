# Contributing to Wardoff

Start with the [README](README.md) for the product and the
[development guide](docs/DEVELOPMENT.md) to find code and tests for your task.
Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).
Agents start at [AGENTS.md](AGENTS.md); humans and agents use the same workflow.

## Set up and check the project

Use Windows 10 or 11, Git, PowerShell 5.1 or newer, and Rust stable targeting
`x86_64-pc-windows-msvc`. Install MSVC C++ build tools and a Windows SDK.
The normal development checks do not require administrator rights.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
```

For Rust changes, finish with the full check, including strict Clippy and the
release build:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Mode Full
```

The [development guide](docs/DEVELOPMENT.md#validation) lists individual commands,
focused test filters, documentation checks and troubleshooting.

## Runtime validation is separate

The [validation matrix](docs/MVP_VALIDATION_MATRIX.md) owns runtime acceptance.
The smoke suite stops repo-built Wardoff processes, starts Block/Allow runtimes,
appends logs, and can change Task Scheduler entries. Run it in an isolated
Windows test environment where those effects are acceptable:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tests/smoke_test.ps1
```

Administrator rights enable additional coverage. A missing UpdateOrchestrator
Reboot task is an expected skip on some Windows editions. Use a disposable VM
for shutdown/reboot/sign-out and suitable test hardware or a capable VM for
physical sleep/hibernate. Record skips and untested behavior rather than
equating unit tests with end-to-end acceptance.

## Scope and authority

- [PLAN.md](PLAN.md) owns product scope and priorities.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) owns intended architecture. Record
  intentional changes with their reason; report drift instead of silently
  rewriting the design to fit it.
- [docs/BUSINESS_MODEL.md](docs/BUSINESS_MODEL.md) owns monetization, sponsorship,
  signed binaries and paid support. Read it before changing related messaging.
- [docs/WINDOWS_SHUTDOWN_LAYERS.md](docs/WINDOWS_SHUTDOWN_LAYERS.md) explains layer
  limits. Do not promise to stop `shutdown /t 0 /f` or explicit sleep/hibernate.

Keep IFEO, Event Log, toasts, timers, profiles, settings and packaging as backlog
unless the task explicitly changes that scope. Existing CI/release workflows
are infrastructure to maintain when requested, not evidence of a new product
feature. Do not add CI, release automation or benchmarks as incidental cleanup.

## Branches and commits

Unless the task specifies a starting point, branch from `dev`, use a focused
feature/fix branch (agents may use `codex/<topic>`), and target `dev` for
integration. Releases reach `main` from the integrated state. Preserve existing
work when continuing from another branch and state which base you used.

Use atomic Conventional Commits: `type: imperative subject`, lowercase with no
trailing period. Explain the change and validation; reference an issue only
when one exists. A local change does not by itself authorize publication.

## Code and documentation conventions

- Keep documentation, code comments and user-visible copy in English.
- Use Rust 2021, `rustfmt` and Clippy. Keep the Windows x64 MSVC target.
- Handle recoverable production errors rather than using `unwrap()` or
  `expect()`; test assertions and fail-fast build-script diagnostics differ.
- Document public Rust items and non-obvious Win32 ownership/thread constraints.
- Use `log` macros for operational messages and the existing structured logger
  for events; preserve machine-readable CLI output and exit codes.
- Preserve red Block / green Allow tray semantics, the non-elevating read-only
  CLI, session-scoped IPC, admin boundaries and transactional cleanup.
- Keep pure decisions separate from Windows effects when that makes the change
  easier to test. Avoid framework layers or module splits without a concrete need.
- Discuss new project dependencies with the maintainer. Keep protection free
  of telemetry, tracking and paywalls.
- Link to an existing documentation owner instead of duplicating its rules.
  Use relative Markdown links and fenced code blocks with language tags.
- Keep dated evidence and unpublished drafts clearly labeled. Do not turn
  measurements from one machine into a general guarantee.

## Review checklist

Explain delivered behavior, affected modules, tests run and remaining
manual/admin checks. Update relevant docs when a contract changes. Report
architecture deviations, security implications and product limits explicitly.
Use the [pull request template](.github/PULL_REQUEST_TEMPLATE.md) for a PR.
