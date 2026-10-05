# Wardoff v0.2.1 (draft, unreleased)

This candidate has not been published. The tag workflow prepares a draft only
after validation; a maintainer must review and publish it separately.

## Changes since v0.2.0

- Bound each newline-terminated control request to 4096 wire bytes and one total
  second. Malformed, oversized, incomplete and disconnected clients cannot hold
  the control reader indefinitely; status remains independently available.
- Use process-owned system/display Power Requests for idle-power protection.
  Report activation errors synchronously, release on Allow/exit, and renew after
  the existing resume notification. Explicit Sleep/Hibernate are not vetoed.
- Fix autostart access checks to use identification tokens. Keep the existing
  trusted-path and same-user/elevation boundaries.
- Reject excessive log-tail requests with a readable error and bound allocation.
  Clean up runtime resources when the Windows message loop fails.
- Isolate Windows smoke cleanup, consolidate developer instructions and checks,
  and validate release provenance, versions and the exact binary before creating
  a draft. No automatic public release or signing is added.

## Validation and limits

The draft includes `build-info.json` and `SHA256SUMS.txt` for its exact unsigned
MSVC binary and the Windows workflow run. Review the run and its explicit smoke
skips before publishing. A green run on another commit is not release evidence.

- Disposable-VM idle-power results are recorded in
  [IDLE_POWER_VALIDATION.md](IDLE_POWER_VALIDATION.md); physical S3/monitor,
  Hibernate and Modern Standby acceptance remains incomplete.
- Missing UpdateOrchestrator Reboot tasks and unavailable interactive linked
  tokens cause documented smoke skips, not successful coverage of those paths.
- Same-session mutex denial of service, the status pipe DACL and other residual
  local hardening debt remain documented in [SECURITY_AUDIT.md](SECURITY_AUDIT.md).
- Forced zero-second shutdown and explicit power actions are not guaranteed
  blockable. This remains a source-first utility for self-administered machines.
- Official signed downloads and paid support are not available. The source
  remains MIT and support remains community-only.

The resource benchmark retained in the repository covers its dated v0.2.0
artifact; it has not been rerun or requalified for this candidate.
