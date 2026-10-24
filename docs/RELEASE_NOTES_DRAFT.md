# Wardoff v0.2.1

> Integration preparation, not a qualified download. No new tag or public release
> is created by this checkpoint. Cargo and the manifest retain the existing
> development versions `0.2.1` / `0.2.1.0`.

The tag workflow can prepare a draft only after its required validation. These
notes do not establish that such a draft or a matching qualified binary exists.

## Integration checkpoint: 23 October 2026

The source base is `main` at `8c459778b4fc726ef6b62d03e150adfcbbac3eda`,
plus the reviewed session-end diagnostic correction
`82a78cd3062061f78cf822d628e5becd15679061`. This preparation adds the MSVC
static-CRT target configuration already reviewed in `180cdfa`, without rebuilding
or changing the Rust sources. It preserves the newer main-line status connection
budget, shutdown wake-up fix, runtime policy and release/developer checks.

The separate branch ending at `8e55940` descends from the older `dev` base
`465c24e`. Replacing main with that snapshot would discard later main-line work.
Its connected status-reply bounds and isolated smoke harness remain separate
integration inputs; they are not implicitly included or validated here. Existing
main control-request bounds do not need to be implemented again. PR #6 remains
a distinct draft, as does PR #10's diagnostic tracing. PR #14's developer fixture
correction remains separate from this source checkpoint.

| Existing evidence | Exact scope | What it does not qualify |
| --- | --- | --- |
| 13 synthetic Layer 1 tests, formatting and docs passed on 21 October | `82a78cd`; refusal versus reported session outcome, parameters and unchanged cleanup guards | Windows Update reboot prevention, native session-end delivery, or a binary built with this new target configuration |
| Offline MSVC build and import inspection | `183d798`, version `0.2.0`; 1,925,632 bytes; SHA256 `534db29520b902e602cd80d9b39c4fd768f464e329e33b9365aaa901c85feb60` | This main-based `0.2.1` candidate or its session diagnostic correction |
| Native disposable-VM run followed by a targeted autostart run | Same `183d798` bytes: 16/19 counted smoke checks passed, three failed and three additional skips; separate autostart creation/identity/removal/cleanup passed with comparator `da4ab5d` | One wholly passing full smoke, actual logon execution, the skipped non-elevated paths, a present Layer 3 Reboot task, or physical power transitions |

The targeted autostart driver also retained two logging errors from its missing
isolated local-appdata directory. Its successful task lifecycle does not qualify
that logging path. The VM evidence is retained separately and is not rerun or
promoted to hardware acceptance by this reconciliation.

Before integration/release, the exact assembled source/tree still needs its
applicable safe checks and matching CI, then an exact MSVC binary with verified
version, manifest, imports, SHA256 and package. Native/manual acceptance must be
bound to those bytes and the chosen release scope; the existing VM results do
not transfer automatically. The `82a78cd` branch had no check runs when inspected
on 23 October. PR #14's successful historical run tested a synthetic merge checkout
`0838ac14`, not this source or the portable candidate.

The `ci/explicit-validation-gates` proposal is also separate from main. Its
`assert-qualification.ps1` always fails closed and declares the authenticated
run/artifact importer `NOT_IMPLEMENTED`; completing tests alone cannot clear it.
This checkpoint neither merges that proposal nor bypasses the existing main
release provenance checks. No candidate binary or package is qualified here.

## Changes since v0.2.0

- Distinguish a Layer 1 refusal from a reported cancellation. Count only a
  same-window `WM_ENDSESSION(FALSE)` following a refusal; a session ending despite
  refusal is not counted. Preserve both messages' parameters and stop attributing
  cleanup to an unproven human action. This corrects diagnostics, not demonstrated
  Windows Update reboot prevention.
- Request static MSVC CRT linkage. The new main-based binary still needs build,
  dependency and clean-environment startup verification.
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

An eventual qualified draft must include `build-info.json` and `SHA256SUMS.txt` for its exact unsigned
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
