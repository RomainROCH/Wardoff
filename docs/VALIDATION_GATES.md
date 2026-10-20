# Validation gates

Safe automated checks and integration/release qualification are separate results.
A safe check can pass while qualification remains **BLOCKED**. This guide does
not authorize native PowerRequest tests, system smoke, publication or deployment.
Local preparation does not authorize protected-branch merges or override any operation-specific refusal.

## Why the previous workflows cannot establish qualification

At source base `06b95136b28b4de75e779632c5b7f501c5b16f7e`, CI ran unfiltered
`cargo test`, a release build and `tests/smoke_test.ps1` on Windows. Unfiltered
tests include real PowerRequest acquisition. The smoke suite starts the real
Block/Allow runtime and can modify scheduled tasks. Calling that combination
"Full" would not make it compatible with exclusions of those effects.

The previous release workflow reacted to every pushed `v*` tag, built again, then
called `gh release create` without `--draft` and uploaded the executable with
`--clobber`. It had `contents: write`, no dependency on successful qualification,
and no check binding the uploaded bytes to an approved candidate. A local version
number or successful build did not establish permission to publish those bytes.

These historical definitions can be inspected at the source base above. The
current [CI workflow](../.github/workflows/ci.yml) and
[release workflow](../.github/workflows/release.yml) implement the gates below.

## Effects of the excluded validation paths

| Path | Effects and required access | Cleanup and remaining limits |
| --- | --- | --- |
| [Native PowerRequest test](../src/blocker/sleep.rs), `blocker::sleep::tests::activation_reports_an_acquired_request_before_returning` | Creates a real request object, sets SYSTEM/DISPLAY requests, renews them and releases them in ten cycles. It requests no elevation and causes no physical suspend or power-policy write. | Normal release attempts both clears and closes the private handle, including acquisition rollback. This remains a native power action even though it is brief. |
| [Smoke entrypoint](../tests/smoke_test.ps1) | Tests an explicit SHA256-bound existing binary, launches an owned runtime without detaching, sends Block/Allow commands and writes isolated application logs. No build occurs. | Guards refuse known process/task/mutex/pipe conflicts; only retained process handles are stopped. An exclusive disposable VM/session is still required because CLI forwarding cannot atomically pin a primary against concurrent actors. Native adapters remain untested; see [protocol](SMOKE_PROTOCOL.md). |
| [Block bootstrap](../src/main.rs) and [coordinator](../src/blocker/mod.rs) | Starts local ETW, UpdateOrchestrator monitoring, shutdown-abort polling, PowerRequests and session shutdown blocking. Explicit `--block` inherits the caller's token; the separate default no-argument path can request elevation. | Allow and normal shutdown deactivate layers; failed activation attempts rollback. Starting the real coordinator is not equivalent to an inert IPC fixture. |
| [Local ETW](../src/blocker/local.rs) and [shutdown abort](../src/blocker/abort.rs) | Starts a real-time Kernel-Process trace where allowed, and can call `AbortSystemShutdownW(None)`. Shutdown-abort code attempts to enable an already assigned shutdown privilege in its process token. | Normal ETW cleanup stops its session and closes the consumer. Shutdown-abort polling can cancel a real pending shutdown while active; it is not a harmless status query. |
| [UpdateOrchestrator](../src/blocker/update.rs) | When elevated and the Reboot task exists, can set its enabled state to false and recheck it periodically. Without elevation or a usable task, the layer skips. | Normal stop attempts to restore enabled state only when the task was initially enabled. Restoration errors are logged. A task initially disabled stays disabled. Abrupt process termination does not execute this restoration. |
| [Autostart](../src/autostart.rs) | Requires elevation; enabling also checks the executable and working directory against the interactive medium token. Can create/update `\Wardoff` with a highest-level current-user logon trigger, or delete it. | The corrected harness refuses an existing autostart task. It tracks a creation attempt before the command, captures XML/security/action/principal identity and deletes only an unchanged owned task. Conflicts and cleanup failures fail the run; simulated checks do not prove the native adapter. |
| Smoke's final forced termination | Re-enters Block, then force-kills the runtime without Allow first. | The harness then restores original Reboot Enabled only if definition/security still match, stops exact owned ETW names and verifies absence. Cleanup attempts independent resources and aggregates failures. This behavior has been simulated; real Windows cleanup still requires the authorized VM protocol. |

The smoke does not deliberately initiate physical shutdown, reboot, Sleep or
Hibernate. Its native effects above are still real. The workflow's Windows runner
label does not prove its effective elevation or the availability of a linked
interactive token; the script detects those properties at runtime and can skip
coverage. No physical power-transition acceptance follows from those skips.

## Automatic safe checks and the required CI result

CI retains `jobs.ci`, its Windows/stable matrix and its existing push/PR triggers.
A push to a feature branch alone does not match the declared branch filters;
opening a PR, including a draft, targeting `dev` or `main` does. Do not use that
distinction to bypass a gate or an existing refusal.

[validate-safe.ps1](../scripts/validate-safe.ps1) provides the safe validation
entrypoint. To inspect its plan without launching checks or product commands:

```powershell
pwsh -NoProfile -File scripts/validate-safe.ps1 -PlanOnly
```

The safe plan covers test inventory and fixtures, formatting, compilation,
the Rust suite with this exact exclusion:

```text
blocker::sleep::tests::activation_reports_an_acquired_request_before_returning
```

It also covers strict Clippy, the release build, and safe `--help`/`--version`
checks. The inventory and exclusion must be explicit; a renamed or missing native
test must not silently invalidate the filter. A filtered suite is reported as
safe validation, never as unfiltered or Full PASS.

At the end of CI, an `always()` step invokes
[assert-qualification.ps1](../scripts/assert-qualification.ps1) with
`-Scope Integration -SafeConclusion <safe-step outcome>`. It returns nonzero
**BLOCKED**: native PowerRequest, real system smoke, portable isolated CLI and
hardware acceptance are **NOT RUN**. A failed safe step cannot qualify either.
There is no skipped qualification job or `continue-on-error` that converts these
missing checks into a green required result. When `ci` is required by repository policy, preserving its context leaves that
requirement unsatisfied until the qualification contract is met. No branch
protection, ruleset or merge approval is changed by this preparation.

Any retained artifact is explicitly **NOT QUALIFIED**. It is evidence of the
reported safe checks, not a release candidate accepted for publication.

## Provenance and release fail-closed behavior

Record the event SHA, actual checkout commit/tree and, for PRs, head and base
SHAs. A PR can test a synthetic merge tree; retain that identity rather than
claiming it is the head tree without verifying equality. Bind a checked binary
to its SHA256 and the actual source tree used to build it.

The release workflow replaces the old writer with read-only qualification that
fails closed. There is no current `gh` publish command, draft writer or asset
replacement path. A tag is not authorization, and the blocked workflow must not
be described as a successful release.

Future qualification needs a trusted workflow revision and verifiable evidence
for the same source SHA/tree, binary hash, workflow run and run attempt. Required
validations must actually have run, with their environment and limits recorded.
A candidate JSON containing `PASS` is data, not authority to override the gate.
Verify the effective remote commit and its exact CI result before reporting an
integration as successful.

Candidate version and identity must distinguish new bytes from an existing
public release. A package bearing `0.2.0` does not establish that it is an existing
public `v0.2.0` asset. Do not overwrite a public asset or reuse a published tag to
claim a different binary. Release publication requires separate authorization.

## Native validation remains excluded

The following are existing entrypoints for future planning only. They were not
executed as part of this validation-gate change and are not authorized by this
document:

```powershell
cargo test --locked blocker::sleep::tests::activation_reports_an_acquired_request_before_returning -- --exact --nocapture --test-threads=1
# Native smoke requires explicit binary/hash/target parameters; see SMOKE_PROTOCOL.md.
```

Before considering execution, obtain authorization naming the disposable Windows
target and permitted native effects in [SMOKE_PROTOCOL.md](SMOKE_PROTOCOL.md).
The ownership and resource-cleanup harness has been prepared and tested with
simulations and inert children. Its native Task Scheduler/ETW adapters have not
been exercised. No manual native job is wired and no qualification is cleared
by that preparation. The existing gate remains intentionally fail-closed.

The next real validation is the exact-binary VM protocol after authorization.
The portable harness for the thirteen isolated CLI checks and authenticated
run/package binding remain separate unfinished qualifications. A VM run does
not establish physical power-transition acceptance.
