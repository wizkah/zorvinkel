# Security Policy

Zorvinkel is in early development (v0.1, pre-release). It handles Wi-Fi credentials, device trust and an authenticated, encrypted channel (Noise XX over BLE and IP), so security reports are taken seriously.

## Supported versions

Only the latest release and the current `main` branch receive security fixes.

## Reporting a vulnerability

Please do **not** report vulnerabilities in public issues, pull requests or discussions.

Use GitHub's private vulnerability reporting instead: open the **Security** tab of this repository and choose **Report a vulnerability**.

Please include:

- the affected component (`zorvinkeld`, `zorvinkel-client`, `zorvinkel-admin`, protocol, packaging scripts),
- the version or commit,
- steps to reproduce, and what you expected compared to what happened,
- the impact as you see it (for example impersonating a device, bypassing registration, leaking Wi-Fi credentials, privilege escalation through the root helpers).

## What to expect

- You will get a first response within **14 days**.
- I will confirm whether the report is a vulnerability, keep you informed about the fix, and credit you in the release notes if you wish.
- Please give me reasonable time to fix the issue before you disclose it. I ask for coordinated disclosure, with a default of 90 days from your report, shorter if a fix is released earlier.

This is a hobby project run by one person; there is no bug bounty.

## Scope

In scope: the handshake and secure channel, the trust model (registration, grants, blocking), storage of keys and credentials, the local administration tool, and the privileged helpers installed by `packaging/`.

Out of scope: the simulation mode, and issues that require physical access to an already unlocked device or a compromised operating system.
