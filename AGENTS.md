# Working on Wardoff

Wardoff is a Windows-only Rust application for shutdown protection and idle-power
requests. Keep the tray and CLI usable by people; agent guidance belongs in the
development docs, not in the product interface.

## Find the owner before editing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow, then use the
[task-to-code map](docs/DEVELOPMENT.md#task-to-code-map) to open only the relevant
modules and tests. [docs/README.md](docs/README.md) indexes the rest.

| Decision | Canonical source |
| --- | --- |
| Product scope and next work | [PLAN.md](PLAN.md) |
| Intended architecture and intentional deviations | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Supported commands and user-visible limits | [README.md](README.md) |
| Validation and Windows-dependent acceptance | [docs/MVP_VALIDATION_MATRIX.md](docs/MVP_VALIDATION_MATRIX.md) |
| Business, sponsorship, signing and paid support | [docs/BUSINESS_MODEL.md](docs/BUSINESS_MODEL.md) |

Report material conflicts. Do not silently redefine architecture to match code
drift or promote roadmap items into shipped features. Read the business-model
document when the task touches that subject. Dated audits, benchmarks and drafts
are evidence or working copy, not a replacement for these owners.

## Build and validate

Use Windows 10/11, Rust stable with the MSVC x64 toolchain, Git and PowerShell
5.1 or newer. Run from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Mode Full
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Mode Docs
```

Default `Fast` checks documentation, formatting, compilation and unit tests.
`Full` also runs strict Clippy and builds the release binary. `Docs` only checks
local documentation links and encoding. The scripts stop on failure and never
launch Wardoff. Details and focused tests live in
[DEVELOPMENT.md](docs/DEVELOPMENT.md#validation).

`tests/smoke_test.ps1` is a separate, state-changing integration suite: it stops
repo-built Wardoff processes, runs Block/Allow and can change scheduled tasks.
Use an isolated Windows test environment; see the validation matrix. A successful
build or unit test is not evidence of actual shutdown, suspend or tray behavior.

## Preserve the important boundaries

- `src/main.rs` owns effects and the application loop;
  `src/runtime_policy.rs` owns pure startup, command and tray decisions.
- Read-only CLI actions must stay non-elevating. Keep control and status pipes
  separate and session-scoped; preserve same-user access checks.
- Preserve layer activation rollback, handle ownership, cleanup and wake restore.
- Keep Layer 2 ETW wording conservative. Do not promise forced zero-second
  shutdown blocking or an explicit Sleep/Hibernate veto.
- Follow [CONTRIBUTING.md](CONTRIBUTING.md) for conventions and branch workflow.
  Do not add dependencies, CI/release infrastructure or later-phase product
  features as incidental cleanup.

## Finish a change

Run focused checks while iterating and `Full` for Rust changes. For docs-only
changes use `Docs`; for developer-tooling changes also run
`tests/developer_checks.ps1`. Record failures and skipped Windows/manual checks.
Update the owning doc when its contract changes and keep links working. Do not
duplicate this guide into harness-specific files.
