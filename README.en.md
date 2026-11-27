# MirrorDock

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#local-development)
[![Built with](https://img.shields.io/badge/built%20with-Tauri%202%20%7C%20React%2019%20%7C%20scrcpy%204.1-24C8D8.svg)](#tech-stack-and-credits)

> **Language / 语言:** English · [中文](README.md)

MirrorDock is a **local-first** Android desktop mirroring and control tool for non-technical users. On Windows, macOS, or Linux, connect your own phone with a USB cable or a one-time Wi-Fi pairing and put it safely on your computer screen.

Video, audio, and input travel point-to-point only between your computer and your authorized phone. **No cloud relay, no uploads by default, no telemetry.**

Current version: `0.4.39` (desktop) · `0.3.2` (companion app). Latest release: <https://github.com/g-star1024/MirrorDock/releases/tag/v0.4.39-beta>

## Features

### Connection

- **USB**: Plug in and go. Automatic detection of devices, authorization state, and offline state, with full guided recovery in your language.
- **Wireless (Android 11+)**: One-time pairing-code setup, standalone connection addresses, trusted-device reconnection and "forget", plus on-screen QR pairing.
- **Unified device card**: One card per phone. USB, wireless, and stale offline entries are merged so the same phone never appears twice.
- **Revoke authorization**: Turn off the phone's debugging switch, drop all wireless connections, and clear local records from your computer with one click. The phone-side "Revoke USB debugging authorizations" still needs to be confirmed on the phone by you; removed devices do not come back after a restart.
- **Recent devices**: Remembers up to 8 recently used devices locally, with per-device removal and clear-all.
- **Companion app**: Ships with an Android companion app for same-network encrypted sessions and pairing guidance. After a successful QR scan on an already-paired wireless-debug phone, the desktop reconnects automatically. Also supports always-on channel reconnect without scanning, phone notification mirroring, and phone → computer file transfers.

### Mirroring and sessions

- High-framerate mirroring and keyboard/mouse control built on **scrcpy 4.1** (pinned version, supply-chain hash verified).
- **Session state machine**: idle / unauthorized / offline / paired / connecting / streaming / failed, reported honestly instead of collapsing everything into "connection failed".
- **Multiple phones at once**: One independent mirror window per phone. Star a device to pin it in the recent list (local display order only; no effect on connection or authorization). Settings can target one device when several are active, and the tray menu lists "End mirroring" per device.
- **Per-device desktop mode**: The desktop-mode switch and the app launched into the virtual display are remembered per device. The app picker shows friendly names (e.g. "Browser", "Gallery") instead of package names.
- **Capability detection**: Reports screen control (SDK ≥ 26) and system audio (SDK ≥ 30) support per device. Unknown stays unknown — never guessed.
- **Smart wake**: Reads the power policy and uses a harmless BACK key only in DIM state; zero injection otherwise.
- **Keep awake**: Prevents the phone from sleeping during a session (USB and Wi-Fi; Wi-Fi uses temporary charge simulation plus system settings, all restored on exit).
- **Lock-screen assistant**: Live locked/asleep status on the device card, remote screen-on, and right-click-to-wake inside the mirror window.
- **Password screen transparency**: When a PIN/pattern entry screen is detected (an Android security surface where mirroring is blocked by the platform), MirrorDock says so plainly and resumes the picture once unlocked — no bypass, no misdirection.
- **Automatic IME handoff on macOS**: When a third-party input method is detected (which intercepts raw keycodes), MirrorDock temporarily switches to the built-in ABC layout and restores your original IME when the session ends, including after a crash.

### Tools

- **Screenshot**: One-click capture of the current screen, PNG integrity checked, name collisions auto-suffixed.
- **Recording**: MP4 capture from inside the session; active recordings are protected from deletion.
- **File transfer**: Two-way transfer with the phone's `Download / MirrorDock` folder — browse, retrieve, and delete individual files (only that one file in that directory). The companion app can send files to the computer, with a live desktop notification and automatic list refresh.
- **Drag and drop**: Drop a file onto the main window — `.apk` installs to the ready phone (first one if several), anything else goes to the phone's send folder. Results are reported honestly in the corner, with reasons for failures.
- **APK install**: Pick an installer on your computer, or drag one in. Reinstall keeps app data; confirmation on the phone is always required.
- **Phone notification mirroring**: After enabling "Notification mirroring" in the companion app and granting notification access, new notifications appear live under Tools → Phone Notifications (up to 50, clearable; in-memory only, never written to disk or logs, no cloud; offline notifications are not backfilled).
- **Quick reply**: Notifications with a reply action (WeChat, SMS, etc.) can be answered directly from Tools → Phone Notifications. Text goes back over the encrypted channel and is sent by the companion app using the original reply action (500-character limit). Offline state is reported, never silently dropped; drafts stay in the current window. Requires desktop 0.4.4 / companion 0.2.5 or later.
- **Global shortcuts**: Screenshot / record / rotate work even in fullscreen mirror windows (registered only during a session).
- **Tray and window**: System tray menu, close-to-tray (configurable), hide Dock icon on macOS, start at login, dismissible status notifications.

### Help

- **Built-in help center**: 11 complete help articles plus a 16-entry feature knowledge base, bundled for offline use. Covers authorization, wireless setup, lock screen and keep-awake, black screens and blocked actions, file and notification mirroring, multi-device management, and more. Connection pages stay focused on actions; explanations live in the help center.
- **Website**: <https://g-star1024.github.io/MirrorDock/> (Chinese / English)

### Language

- **Interface language**: Settings → General → Interface language, with Simplified Chinese and English. Switching takes effect immediately and is remembered locally; it only affects the interface on this computer.
- **Website and docs**: The homepage, compatibility matrix, and this README are all available in Chinese and English.

## Security and privacy boundaries

MirrorDock's security model is a deliberate red line, and its differentiator:

- **Local-first**: All data (video, audio, clipboard, files) travels directly between your computer and phone. No cloud relay, no telemetry by default.
- **Supply-chain verification**: Bundled scrcpy and adb are checked against official SHA-256 hashes in CI; a hash mismatch stops the build. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- **Subprocess safety**: adb and scrcpy are called with fixed argument arrays, never shell strings; no device- or network-provided content is executed.
- **Least privilege**: Mirroring starts only for already-authorized devices. PIN/pattern screens are detected and reported honestly — **never bypassed**.
- **No sensitive logs**: Video frames, clipboard contents, pairing codes, and passwords are never logged; diagnostic logs record only command outcomes and device serials.
- **Signature verification**: Release signing uses ed25519, with the public key compiled into the binary and the private key never entering the build environment.

## System requirements

| Side | Requirement |
| --- | --- |
| Computer | Windows 10+ (x64) / macOS 12+ (Intel or Apple Silicon) / mainstream Linux distributions |
| Phone | Android 7.0+ (USB mirroring); Android 11+ (wireless debugging pairing) |
| Dependencies | None to install. Windows / macOS installers bundle scrcpy and adb; Linux packages bundle them too (built from source), but need distro runtime libraries: Ubuntu 25.04+ can use `sudo apt install libsdl3-0 libavcodec61 libavformat61 libavutil59 libswresample5 libusb-1.0-0`; Ubuntu 22.04/24.04 need SDL3 first (below). |

### Linux runtime libraries (the honest version)

scrcpy 4.x depends on **SDL3**, which is not yet in the official Ubuntu 22.04/24.04 repositories. If mirroring fails after installing MirrorDock:

```sh
# Option 1: upgrade to a distro that ships SDL3 (Ubuntu 25.04+ / Debian 13+ / Fedora 42+)
# Option 2: build SDL3 manually (~3 minutes):
#   git clone --depth 1 -b release-3.4.16 https://github.com/libsdl-org/SDL
#   cmake -S SDL -B SDL/build -DCMAKE_BUILD_TYPE=Release && cmake --build SDL/build -j4
#   sudo cmake --install SDL/build && sudo ldconfig
```

## Automatic updates

The client includes in-app updates (see "Settings → About MirrorDock → Check for updates"): startup or manual check → download → **minisign signature verification** (public key compiled in, private key never in the build environment) → install → restart. The update manifest (`latest.json`) comes from this repository's GitHub Releases.

## Local development

Prerequisites: Node.js ≥ 20, pnpm, Rust (stable), Tauri 2 platform dependencies, Android Platform Tools (`adb`).

```sh
pnpm install
pnpm tauri dev
```

During development, scrcpy runtime files live in the ignored `.tools/` directory (or set `MIRRORDOCK_SCRCPY_PATH`).

### Quality checks

```sh
# Frontend: TypeScript check + unit tests + build
pnpm build
pnpm test

# Backend: tests + lint
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
```

### Companion app

The Android companion app lives in `companion/` (Kotlin, minSdk 26 / targetSdk 34):

```sh
gradle -p companion :app:assembleDebug
```

## Project documentation

- [AGENTS.md](AGENTS.md) — product boundaries, engineering rules, collaboration (read before development)
- [DEVELOPMENT_TASKS.md](DEVELOPMENT_TASKS.md) — iterations and acceptance records
- [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) — third-party notices and supply-chain hashes

## Tech stack and credits

MirrorDock stands on the shoulders of many excellent open-source projects:

### Core dependencies

| Project | Purpose | License |
| --- | --- | --- |
| [scrcpy](https://github.com/Genymobile/scrcpy) (Genymobile) | Underlying mirroring and control engine, v4.1 bundled | Apache-2.0 |
| [Android Platform Tools](https://developer.android.com/tools/releases/platform-tools) (adb) | Device communication, state probing, controlled input | Apache-2.0 |
| [Tauri](https://tauri.app/) | Cross-platform desktop framework (v2, tray, global shortcuts, autostart) | MIT / Apache-2.0 |
| [React](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) | Frontend UI | MIT / Apache-2.0 |
| [Vite](https://vite.dev/) | Frontend build and dev server | MIT |
| [Rust](https://www.rust-lang.org/) | Backend logic and security boundaries | MIT / Apache-2.0 |
| [Kotlin](https://kotlinlang.org/) (JetBrains) | Android companion app | Apache-2.0 |

### Rust ecosystem

- [tokio](https://tokio.rs/) — async runtime (companion encrypted session)
- [rustls](https://github.com/rustls/rustls) / [tokio-rustls](https://github.com/rustls/tokio-rustls) — TLS 1.3 channel
- [rcgen](https://github.com/rustls/rcgen) — self-signed certificate generation (pairing fingerprint verification)
- [ed25519-dalek](https://github.com/dalek-cryptography/ed25519-dalek) — release signature verification
- [serde](https://serde.rs/) — serialization

### Frontend ecosystem

- [qrcode](https://github.com/soldair/node-qrcode) — wireless pairing QR codes
- [Vitest](https://vitest.dev/) + [Testing Library](https://testing-library.com/) — frontend tests
- [jsdom](https://github.com/jsdom/jsdom) — test environment

### Special thanks

- **Genymobile**: scrcpy is the foundation of this project, and its engineering quality and Apache-2.0 license make a local mirroring tool possible.
- **Android Open Source Project**: adb and the open device-protocol ecosystem.
- **Tauri community**: the practice and sustained maintenance of building lightweight desktop apps with web technology.

Full third-party license notices and supply-chain hashes: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## License

MirrorDock is released under the [Apache License 2.0](LICENSE), matching scrcpy.

The software is provided "as is", without warranty. By using it you acknowledge that mirroring is constrained by device-side security policies (e.g. FLAG_SECURE content cannot be captured), and MirrorDock does not and will not provide features that bypass device security mechanisms.
