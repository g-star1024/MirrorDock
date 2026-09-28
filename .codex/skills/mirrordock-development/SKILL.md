---
name: mirrordock-development
description: Build, test, review, package, or release MirrorDock's Tauri/Rust desktop app, ADB/scrcpy integration, Android companion app, connection UX, compatibility, or channel-specific work. Do not use for unrelated repositories.
---

# MirrorDock Development

Deliver the smallest safe improvement for non-technical users connecting their own Android device on Windows, macOS, or Linux.

## Start gate

1. Read repository `AGENTS.md`, `DEVELOPMENT_TASKS.md`, `agents/TEAM_AGENTS.md`, and the current implementation/tests. The internal product plan is maintained **outside this repository** and is not distributed with it.
2. Load every narrow matching user-level Skill. Android manifest/IPC uses `android-permissions-security` and relevant Intent security; Play delivery uses `play-policy-insights`; Android test setup uses `testing-setup`; Gradle commands use `gradle-run`; Kotlin concurrency uses `kotlin-concurrency-and-flow`; Compose tests use `compose-ui-testing-patterns`; browser/webview UI E2E uses `playwright`; security reviews use the matching security Skill.
3. State affected desktop platforms, Android versions, connection mode, permissions/data, and acceptance evidence before editing.

## Product constraints

- ADB/scrcpy is the preferred MVP path, but requires explicit device-owner debugging authorization. Preserve consent, local-first data handling, and protected-content restrictions.
- Keep discovery, authorization, session state, media, input, and UI behind distinct interfaces. Use fixed direct subprocess arguments; no shell interpolation.
- scrcpy integration requires auditable source/version/license/NOTICE/SBOM evidence. Never claim generic support before runtime capability checks.
- If device testing, signing credentials, policy review, or OEM support is unavailable, record it as an explicit unverified surface.

## Done

Run the narrowest relevant automated checks, exercise a success path and a realistic recovery path, and report commands, unverified surfaces, compatibility, policy/license effect, and rollback behavior.
