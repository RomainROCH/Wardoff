# Security Policy

Wardoff is a Windows-native utility that intentionally runs elevated for some features and integrates with Windows shutdown and power-management APIs. Security reports that could expose users to abuse of that elevated position are taken seriously.

## Supported versions

| Version | Status |
| --- | --- |
| `dev` branch | Supported |
| `v0.1.0-mvp` (latest tagged release) | Supported |

Older versions are not supported for security reporting.

## Reporting a vulnerability

Please report suspected vulnerabilities privately:

- GitHub private vulnerability reporting / Security Advisories: use it if it is enabled for this repository
- If that private GitHub path is not enabled yet, do not post vulnerability details publicly; wait for a private reporting channel to be arranged with the maintainer

Please include affected version or commit, reproduction steps, impact, and any proof-of-concept details needed to validate the report.

Best-effort acknowledgment: within 48 hours.

## What counts as a security issue

For Wardoff, security issues include paths such as:

- privilege escalation beyond intended behavior
- IPC named-pipe hijacking or unauthorized command/control
- tray spoofing that could mislead users about Wardoff state or actions
- any path where Wardoff's elevation could be abused or turned into unintended code execution, persistence, or system control

## What is not, by itself, a vulnerability

Wardoff intentionally runs elevated for some operations and hooks into Windows shutdown and power APIs. That behavior is part of the product design and is not, by itself, a security vulnerability.
