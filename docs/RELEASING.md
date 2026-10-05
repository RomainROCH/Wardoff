# Releasing Wardoff

The supported flow is **tag -> Windows validation -> exact binary -> draft ->
manual publication**. A tag does not authorize an automatic public release.
The current `0.2.1` source version is provisional; no matching release is shipped.

## Why drafts rather than automatic publication

Automatic builds and draft preparation remove repetitive packaging work. Keep
publication manual for this small source-first project: binaries are unsigned,
some interactive UAC cases are skipped on hosted runners, and physical S3,
Hibernate and Modern Standby acceptance is incomplete. A maintainer must review
the actual evidence and wording before making a binary available publicly.

Fully manual packaging would avoid automation but make source/asset mismatches
easier. Fully automatic public releases would give a green job more authority
than the evidence supports. The draft flow preserves automation's useful part
without adding certificates, services, secrets, paid runners or commercial claims.
See [BUSINESS_MODEL.md](BUSINESS_MODEL.md) and [SUPPORT.md](../SUPPORT.md).

## Before requesting a tag

1. Integrate the reviewed candidate through `dev` into `main`. Keep other agents'
   worktrees and unique branches intact.
2. Choose a stable `vX.Y.Z` with no leading zeroes, prerelease or build suffix.
   Match `[package]` in Cargo.toml, the `wardoff` entry in Cargo.lock and manifest
   assembly version `X.Y.Z.0`; keep `asInvoker` and `uiAccess=false`.
3. Update [CHANGELOG.md](../CHANGELOG.md) and
   [RELEASE_NOTES_DRAFT.md](RELEASE_NOTES_DRAFT.md) from the previous published tag.
   Keep dated benchmark and VM evidence bound to their original binary.
4. Wait for the exact commit's latest **push/main** CI run and its Windows steps
   to succeed. A PR run, another branch/SHA, a newer failed rerun, missing evidence
   or a skipped required step cannot satisfy the gate. Do not filter failed runs
   away or add `continue-on-error` to make publication proceed.
5. Obtain authorization for the actual integration/push/tag and its draft effect.
   Local preparation and local success do not authorize those external actions.

The tag's commit must be reachable from current remote `main`; it need not be
the tip. The remote tag must still peel to the checked-out commit, including an
annotated tag. Git/API errors fail closed. If CI is pending, rerun the tag workflow
only after it succeeds; there is no bypass or polling service.

## What the workflow does

[release.yml](../.github/workflows/release.yml) uses two standard Windows jobs.
The validation job has only `contents: read` and `actions: read`. It validates
provenance and versions, runs Full checks, developer fixtures, smoke safety and
release guards, then the state-changing Windows smoke in the disposable hosted
runner. Native Power Request tests are not excluded there. Full builds with
`--locked`; smoke uses `-UseExistingBinary` and cannot compile a replacement.

The hash is taken before smoke and must match afterward. Package checks verify
help/version, the real embedded manifest resource, Windows file version and MSVC
host. They copy that binary without rebuilding it and produce:

- `wardoff.exe`: the exact unsigned checked binary;
- `SHA256SUMS.txt`: its SHA-256;
- `build-info.json`: tag, full source SHA, main CI run/attempt, build run URL,
  Rust version/target, hash, unsigned status, tests and known limits.

The draft job depends on successful validation. Only this job receives
`contents: write`, with `actions: read` for evidence. It downloads the immutable
artifact ID emitted by validation, verifies the binary hash and metadata, and
rechecks provenance and CI. It then resolves the remote tag again immediately
before `gh release create --verify-tag --draft`. `--verify-tag` alone proves only
existence, as documented in the [GitHub CLI manual](https://cli.github.com/manual/gh_release_create).

Any existing release **or draft** for the tag blocks creation. There is no asset
clobber, automatic draft edit or publication command. After a failed upload,
inspect a partial draft manually; rerunning will refuse to overwrite it. The
write-job check matters because read-only API responses can omit drafts.

## Review before manual publication

Verify the draft tag still identifies the recorded SHA, download the actual
assets and check their hash, inspect the exact run's tests and explicit smoke
skips, and ensure the release notes match its scope. Keep hardware/UAC and local
security debt visible. Successful VM/request checks do not establish universal
power behavior; see [IDLE_POWER_VALIDATION.md](IDLE_POWER_VALIDATION.md),
[MVP_VALIDATION_MATRIX.md](MVP_VALIDATION_MATRIX.md) and
[SECURITY_AUDIT.md](SECURITY_AUDIT.md). Publication requires separate authorization.

## Limits of repository-only guards

These guards apply to the workflow committed at the new tag. An older tag can
carry an older workflow, and a maintainer able to change workflows can remove
checks. Repository scripts are not a security boundary against that maintainer.
External tag rules/permissions could address that separately, but no account or
branch/tag protection settings are changed by this work.

There is a small race between the final remote-tag read and draft creation;
manual review must check the tag again before publication. Drafts remain editable
on GitHub. Neither SHA-256 nor these checks is an Authenticode signature or an
attestation service. Do not add a PAT or broader permission to bypass a denied
release creation; inspect the integration and GitHub error instead.

## Local verification without system effects

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Mode Docs
powershell -NoProfile -ExecutionPolicy Bypass -File tests/developer_checks.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File tests/smoke_safety.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File tests/release_guards.ps1
```

These fixture suites use fake metadata/processes and owned temporary files, not
network mutation or the protection runtime. `Fast`/`Full` also execute a real
Power Request test; the full smoke starts Block/Allow and can edit tasks. When
those effects are not authorized locally, run individual Cargo checks and
explicitly exclude only
`blocker::sleep::tests::activation_reports_an_acquired_request_before_returning`.
Record that exclusion as NOT RUN. It does not replace the complete hosted checks
required for a draft. Local fixture success is not hosted workflow acceptance.
