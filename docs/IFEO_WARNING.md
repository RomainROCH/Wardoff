# IFEO Warning

> **PLANNED — NOT YET IMPLEMENTED**

This document describes a possible future aggressive mode for Wardoff. It is not part of the current supported implementation.

Current reality:

- Wardoff's documented MVP does **not** include IFEO
- Wardoff does **not** currently ship IFEO-based `shutdown.exe` interception as a user-facing feature
- Wardoff still does **not** promise to block local `shutdown /t 0 /f`

## What IFEO is

Image File Execution Options (IFEO) is a Windows registry feature that can alter how a named executable starts. The most relevant mechanism here is the `Debugger` value, which can redirect process launch through another executable.

For `shutdown.exe`, the registry path would be:

`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\shutdown.exe`

Conceptually:

```text
HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\shutdown.exe
  Debugger = "C:\Program Files\Wardoff\wardoff-proxy.exe"
```

That approach would let a proxy inspect or block `shutdown.exe` before the original process fully runs.

## Why the project has considered it

Wardoff's hardest local-shutdown case is the fast forced path, especially:

```text
shutdown /t 0 /f
```

Reactive techniques run into a race there. IFEO is attractive because it moves the interception point earlier, before `shutdown.exe` proceeds normally.

## Why it is not in the MVP

IFEO is intentionally excluded from the current safe MVP because it is:

- aggressive
- admin-only
- security-sensitive
- easy to get wrong if cleanup is incomplete

It also tends to attract scrutiny from:

- EDR products
- antivirus tooling
- enterprise administrators
- security reviewers

## Security and operational concerns

Specific concerns include:

- IFEO is commonly associated with persistence and defense-evasion tradecraft
- a stale `Debugger` value can break the normal `shutdown.exe` path
- cleanup must restore prior state correctly, not just delete blindly
- enablement must be explicit and reversible

## Requirements for any future implementation

If Wardoff ever ships this feature, it should require all of the following:

- explicit opt-in
- very clear warning text
- elevation
- structured logging for enable, disable, and use
- robust cleanup on disable and uninstall
- restoration of any previous legitimate IFEO state

## Relationship to current local-shutdown behavior

The repository may contain ETW-based local-shutdown code, but that is separate from this document.

This file is only about the later-phase aggressive option:

- ETW local interception is not the same as IFEO
- IFEO remains planned
- IFEO is not implemented today

## Bottom line

Wardoff should only ship IFEO if it can do so transparently, reversibly, and with explicit user consent.

Until then, this document is design guidance only.
