# MirrorDock Engineering Iron Rules

## Mandatory start gate

- Before every implementation, test, review, package, or release task, read this file and the relevant section of `Android桌面镜像工具产品规划.md`; identify and load applicable project or user-level Skills before acting.
- Missing development or verification dependencies may be installed promptly at user/project scope. Record name, version, and purpose. Never silently add production cloud services, permissions, daemons, or telemetry to unblock work.
- Do not use destructive scaffold commands in a non-empty workspace unless their overwrite behavior has been inspected and all existing project artifacts are preserved.

## Product and safety contract

- Build for non-technical users. Default UI explains prerequisites, why they matter, and a concrete recovery action; never expose raw ADB or codec details as the primary path.
- USB and same-LAN wireless ADB with explicit owner authorization are the MVP high-performance paths. Do not claim that connecting a cable alone grants full control.
- Never bypass lock screens, DRM/`FLAG_SECURE`, protected system pages, MDM policies, or consent. No unattended general-purpose control, hidden sessions, default cloud recording, or default telemetry.
- Sessions, recording, audio, clipboard, file transfer, and optional accessibility control must be visible and revocable. Keep screen/audio/input local by default and never log screen frames, clipboard, typed secrets, or access tokens.
- scrcpy is permitted only with Apache-2.0-compliant provenance, version, NOTICE/license, SBOM, and upgrade evidence. Use fixed arguments and direct process invocation; never execute device-provided or network-provided commands/binaries.

## Quality and releases

- After every verified development task, update `DEVELOPMENT_TASKS.md` in the same change. Mark only fully evidenced tasks with `✅`; leave incomplete or externally blocked tasks unchecked and state the missing evidence.
- Treat unauthorized, offline, paired, connecting, streaming, and failed states as distinct recoverable states. Do not present only “connection failed”.
- Capability-gate input, audio, wireless pairing, screen-off behavior, and codecs. Test USB and Wi-Fi independently; verify changes to transport/input/permission paths automatically and on a physical device when available.
- Before release: inspect SBOM, notices, licenses, vulnerability results, artifact signatures, channel requirements (mainland China, Google Play, enterprise sideload), and rollback behavior. A safety, policy, or license block cannot be waived by product metrics.

## Team and skills

- Use `$mirrordock-development` for MirrorDock work. Load the narrowest relevant Skill named in its instructions and use the role boundaries in `agents/TEAM_AGENTS.md` for cross-specialty work.
- Each handoff reports affected platforms/channels, permissions/data, changed files, verification evidence, compatibility impact, license/policy impact, and open risks.
