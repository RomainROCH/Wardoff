# Wardoff v0.2.1

The tag workflow prepares a draft only after validation; a maintainer reviews
the exact candidate and publishes it separately.

## Changes since v0.2.0

- Bound each newline-terminated control request to 4096 wire bytes and one total
  second. Malformed, oversized, incomplete and disconnected clients cannot hold
  the control reader indefinitely; status remains independently available.
- Fix a missed status wake-up between named-pipe instances that could leave
  shutdown waiting for the status worker. Coordinate instance publication with
  the stop request, preserving the separate control and status pipes.
- Share one two-second connection-retry budget between the read-only status pipe
  and its legacy control fallback. Unavailable-endpoint discovery targets about
  two seconds plus scheduling overhead; connected reply reads remain outside this
  budget, and ordinary control retries are unchanged.
- Add isolated native Windows singleton tests for primary/secondary claims,
  handle cleanup, bounded retries and reclaim after owner release. They do not
  start protection or validate an elevated relaunch, and do not fix the documented
  same-session mutex denial-of-service risk.
- Use process-owned system/display Power Requests for idle-power protection.
  Report activation errors synchronously, release on Allow/exit, and renew after
  the existing resume notification. Explicit Sleep/Hibernate are not vetoed.
- Fix autostart access checks to use identification tokens. Keep the existing
  trusted-path and same-user/elevation boundaries.
- Reject excessive log-tail requests with a readable error and bound allocation.
  Clean up runtime resources when the Windows message loop fails.
- Provide a passive physical-power validation kit to prepare a case matrix and
  collect local checkpoints. Follow the existing
  [validation matrix](MVP_VALIDATION_MATRIX.md); actual hardware acceptance remains
  separate.
- Isolate Windows smoke cleanup, consolidate developer instructions and checks,
  and validate release provenance, versions and the exact binary before creating
  a draft. No automatic public release or signing is added.

## Validation and limits

The draft includes `build-info.json` and `SHA256SUMS.txt` for its exact unsigned
MSVC binary and the Windows workflow run. Review the run and its explicit smoke
skips before publishing. A green run on another commit is not release evidence.

- An isolated status-shutdown regression reproduces the missed wake-up on the
  prior worker, then verifies joined threads and endpoint cleanup without a
  rescue connection after the correction. It does not establish the cause of the
  earlier CI stall.
- The passive validation kit passes 111 checks with diagnostics mocked. Those
  checks validate preparation, not actual sleep, display timeout, Hibernate or
  Modern Standby behavior.
- Eight historical disposable-VM idle-power cases passed for source `4041db0`
  and the earlier retained binary identified in
  [IDLE_POWER_VALIDATION.md](IDLE_POWER_VALIDATION.md). Those results do not qualify
  the new v0.2.1 binary. Physical S3/monitor, Hibernate and Modern Standby acceptance
  remains incomplete.
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
