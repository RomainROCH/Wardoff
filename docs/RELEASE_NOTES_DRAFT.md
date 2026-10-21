# Wardoff development release notes

> Unpublished development notes; no future version or tag has been selected.
> The public v0.2.0 was released on 3 May 2026. Development sources still
> declare Cargo version `0.2.0`; local packages are `NOT_PUBLIC_RELEASE`.

For the published download, use
[v0.2.0](https://github.com/RomainROCH/Wardoff/releases/tag/v0.2.0) and its
[matching source reference](https://github.com/RomainROCH/Wardoff/blob/2fc4066ae0e48678e5b7efe3ec8289bbcae76525/README.md),
with the corrected power limits below. These notes describe later development,
not a replacement for that asset.

## Development changes since public v0.2.0

- Use owned system/display Power Requests for idle-power protection. Activation
  failures reach the coordinator synchronously; Allow/exit releases the request.
  The existing resume callback renews the pair while Wardoff remains in Block.
- Bound connected status replies to 65536 wire bytes through the first LF,
  counting CRLF, or nonempty JSON at EOF within the limit. A fresh one-second
  absolute I/O budget begins after connection; the legacy Status request write
  shares it. Connection retries remain separate. Transport errors after
  connection fail without trying the other endpoint.
- Bound control requests to LF within 4096 wire bytes, counting CRLF, and one
  absolute second. Nonblocking byte reads poll at 10 ms; arriving fragments do
  not renew the deadline. Empty, malformed, oversized, disconnected and
  unterminated requests are rejected. JSON at EOF without LF is rejected,
  unlike the preserved EOF compatibility for status replies.
- Preserve the separate status worker and same-user/elevation rules. The control
  server remains sequential; these read bounds are not overall UI, command or
  valid-reply-write deadlines. Scheduling and cancellation can add overhead.
- Correct inactive-status documentation and align the embedded assembly version
  with Cargo/CLI/PE `0.2.0` / `0.2.0.0`. This selects no new release version.
- Prepare explicit Safe source checks and separate qualification gates; see
  [VALIDATION_GATES.md](VALIDATION_GATES.md).

## Validation and package status

The current local candidate was built from
`183d798c61d4e252f1847a873bbc3ceec8f71b43`: 1,925,632-byte executable, SHA256
`534db29520b902e602cd80d9b39c4fd768f464e329e33b9365aaa901c85feb60`.
It still reports `wardoff 0.2.0` and is `NOT_PUBLIC_RELEASE`. The locked offline
MSVC release build passed; dependency inspection found no external CRT imports.
The earlier `06b9513` package and its separate namespace-fixture evidence remain
historical artifacts; neither is substituted for these bytes.

Two separate native runs on these exact production-namespace bytes in an
offline disposable Windows 11 VM establish the following limited coverage:

- The first run passed 16/19 counted checks: help/version, inactive and running
  JSON status, Block/Allow, JSON logs, PowerRequests acquisition, release,
  reacquisition and absence after owned runtime termination. Its three failures
  concern autostart ownership/restoration/lifecycle; three additional checks
  were skipped. Layer 4 was observed in status, without a real shutdown test.
- The subsequent targeted autostart run used the corrected comparator from
  `da4ab5d5aebcb39ea97d7314a84808c18162de1a`. Creation, task identity, removal
  and cleanup passed. Captured COM action/working-directory paths carry the
  extended DOS prefix; normalization accepts their canonical local spelling
  while retaining principal and exact task-definition/security checks.

These are complementary results, **not one wholly passing full smoke**.
The targeted driver's isolated `localappdata` directory was missing: two
logging errors (OS error 3) are retained in its stderr. Its on/off commands
exited 0 and task cleanup passed; that run does not validate logging. The first
run's JSON-log checks remain separate evidence. Guest idle policies were
restored, the VMs closed, and parent images preserved.

Not established: task execution at logon, the two non-elevated native status
paths skipped by the elevated run, Layer 3 on a system with its actual Reboot
task (absent in this guest), and physical shutdown, S3/monitor, Hibernate or
Modern Standby behavior. The status flags and PowerRequests observations do not
prove those transitions. Full qualification remains unestablished. A limited
experimental distribution would need an explicit scope/version/publication
decision; this local package creates no release or publication authorization.

## Product limits

- Protection concerns idle sleep and automatic display timeout, subject to
  Windows power policy. Explicit Sleep/Hibernate, lid and power-button actions
  are not vetoed. Earlier blanket sleep/hibernate wording was too broad.
- `shutdown /t 0 /f` is not guaranteed blockable.
- UpdateOrchestrator protection needs the required rights and task. Its absence
  on some LTSC editions is an expected limitation.
- Same-session mutex denial of service, the status pipe's default DACL and other
  local hardening debt remain in [SECURITY_AUDIT.md](SECURITY_AUDIT.md).
- No installer or package-manager distribution is provided. These notes create
  no signed-download, support or commercial offer; see
  [BUSINESS_MODEL.md](BUSINESS_MODEL.md).

Before any future public download, reconcile its unused version/tag, exact
source/tree, binary digest, matching documentation, tests and explicit limits.
Preserve existing tags and assets. See the [validation gates](VALIDATION_GATES.md).
