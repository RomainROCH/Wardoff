# IFEO Warning

> **PLANNED — NOT YET IMPLEMENTED**

This document describes a future aggressive mode for Wardoff. It is not enabled, shipped, or implemented in the current repository.

## What IFEO is

Image File Execution Options (IFEO) is a Windows registry mechanism that can change how a specific executable starts. The most relevant value is `Debugger`, which tells Windows to launch another executable instead of starting the original process directly.

For the local `shutdown.exe` path, the relevant registry location is:

`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\shutdown.exe`

Conceptual example:

```text
HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\shutdown.exe
  Debugger = "C:\Program Files\Wardoff\wardoff-proxy.exe"
```

With that configuration, Windows starts the proxy first. The proxy can then inspect the arguments, log the event, decide whether to block it, and optionally forward the original command.

## Why Wardoff would consider it

In the planned architecture, ETW plus `AbortSystemShutdown(...)` is useful when there is still a timeout window. It is not fast enough for a local `shutdown /t 0 /f` once `shutdown.exe` is already running.

That is why IFEO is being considered:

- for the specific local `shutdown.exe` path, it is the only planned user-space interception point before the executable actually starts
- it can stop the launch before Windows enters the fast forced-shutdown path
- it gives the project a way to handle the exact command line that ordinary reactive techniques cannot catch in time

Important limitation:

- IFEO is not a universal power-control solution
- it only applies to the `shutdown.exe` executable path
- it does not replace deeper kernel-level interception

## Why it is controversial

IFEO is powerful, but it is also widely associated with persistence and defense-evasion techniques.

Specific concerns:

- MITRE ATT&CK classifies this area under **T1546.012 — Image File Execution Options Injection**
- many EDR and antivirus products monitor or alert on IFEO changes
- administrators may treat unexpected IFEO entries as suspicious until proven otherwise
- bad cleanup can leave the target executable permanently redirected
- enterprise policy may forbid this class of behavior entirely

In other words, IFEO is legitimate Windows functionality, but it lives in a security-sensitive part of the platform.

## Planned safeguards

If Wardoff ever adds this mode, it should be handled with strict boundaries:

- opt-in only
- clearly marked as aggressive
- administrator rights required
- explicit warning in the UI and CLI before enablement
- structured logging when the setting is enabled, disabled, or used
- clean removal on disable or uninstall
- restoration of any previous legitimate IFEO state instead of blind deletion

The default experience should remain the safe, official-API path. IFEO should never be silently enabled.

## Cleanup expectations

Proper cleanup is mandatory for this feature to be acceptable.

At minimum, the future implementation should:

- remove the `Debugger` value it created when the mode is turned off
- restore any pre-existing value if Wardoff replaced one
- log cleanup success or failure
- fail loudly if elevation is missing instead of pretending the cleanup happened

This is one reason the feature is deferred: if the project cannot guarantee safe cleanup, it should not ship the mode at all.

## Position in the roadmap

According to the current plan:

- IFEO is **not** part of the safe MVP
- it belongs to a later phase as an aggressive, opt-in mode
- it exists specifically because `shutdown /t 0 /f` is otherwise beyond what the planned user-space layers can stop reliably

That roadmap placement is intentional. The project prefers transparent limitations over pretending that a risky feature is harmless.
