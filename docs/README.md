# Documentation

Choose the page for your task. There is no need to read every document before a
small change.

## Start here

| Need | Read |
| --- | --- |
| Install and use Wardoff | [Project README](../README.md) |
| Set up a development environment | [Contributing](../CONTRIBUTING.md) |
| Find code, tests and common pitfalls | [Development guide](DEVELOPMENT.md) |
| Start an agent session | [Agent entrypoint](../AGENTS.md) |
| Understand current scope and priorities | [Plan](../PLAN.md) |

## Maintained contracts

| Topic | Owner |
| --- | --- |
| Architecture, threads and resource lifecycle | [Architecture](ARCHITECTURE.md) |
| Shutdown layers and Windows limits | [Shutdown layers](WINDOWS_SHUTDOWN_LAYERS.md) |
| Automated and manual acceptance | [Validation matrix](MVP_VALIDATION_MATRIX.md) |
| Why aggressive interception is excluded | [IFEO warning](IFEO_WARNING.md) |
| Sponsorship, signed binaries and paid support | [Business model](BUSINESS_MODEL.md) |
| Reporting security issues | [Security policy](../SECURITY.md) |
| Questions and support | [Support](../SUPPORT.md) |
| Released changes | [Changelog](../CHANGELOG.md) |

## Evidence, comparisons and drafts

These pages have their own date, scope or draft status. Re-check findings against
the code you are changing; they do not redefine product or architecture contracts.

- [Security audit](SECURITY_AUDIT.md): findings and residual risks at the audited state.
- [Resource benchmark](benchmarks/2026-09-29-v0.2.0/README.md): protocol, raw data,
  limitations and reproduction tools for one machine and binary.
- [Product comparison](COMPARISON.md): comparative background.
- [Release notes draft](RELEASE_NOTES_DRAFT.md): unpublished release copy.
- [Outreach drafts](OUTREACH_DRAFTS.md): unpublished community messages.

Keep each rule in its owning document and update links when moving a page.
Claude and Copilot entrypoints route to the same agent guide; they are not
independent copies of project policy.
