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

Source checks, isolated IPC fixtures and local package checks are separate
evidence. The retained local package belongs to source
`06b95136b28b4de75e779632c5b7f501c5b16f7e`, with executable SHA256
`0e3a4fe985e5945c78e65fd78006d38698117a8c98f8459d111308008b027c4f`.
It is a verification artifact, not a public release. This CI/documentation
preparation does not replace that package or qualify a new executable.

The extracted production-namespace package was checked with read-only help and
version commands. Connected CLI cases used a separately built source copy with
three IPC/mutex prefixes changed. They do not establish connected behavior of
the production-namespace executable. A portable in-repository CLI fixture and
its CI evidence are still required.

Native PowerRequest acquisition and the real system smoke remain **NOT RUN**
under the current scope. Physical S3/monitor, Hibernate and Modern Standby
acceptance remains incomplete. Full/release qualification stays blocked while
the required native, system, portable CLI, exact-package and hardware evidence
is missing. A Safe PASS, a benchmark or a VM result for another hash does not
qualify the future download.

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
