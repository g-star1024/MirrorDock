# MirrorDock

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#本地开发)
[![Built with](https://img.shields.io/badge/built%20with-Tauri%202%20%7C%20React%2019%20%7C%20scrcpy%204.1-24C8D8.svg)](#技术栈与致谢)

MirrorDock 是一个**本地优先**的 Android 桌面镜像与控制工具，面向非技术用户：在 Windows、macOS 和 Linux 上，用一根数据线或一次无线配对，把自己的手机安全地投到电脑屏幕上。

画面、音频与控制仅在电脑与已授权手机之间点对点传输。**不依赖云端、不默认上传任何数据、不收集遥测。**

当前版本：`0.4.7`（桌面端）· `0.3.0`（伴侣 App）。

## 功能特性

### 连接

- **USB 连接**：即插即用，自动检测设备、授权与离线状态，全程中文引导与恢复建议。
- **无线连接（Android 11+）**：一次性配对码配对、独立连接地址连接，支持可信设备一键重连与忘记；本机二维码扫码辅助。
- **统一设备卡片**：一台手机一张卡、状态即操作，无论 USB、无线还是离线残影都自动归并，同一台手机不会出现多张卡片。
- **取消授权**：在电脑端一键关闭手机的调试开关、断开全部无线连接并清理本机记录（手机端的「撤销 USB 调试授权」按引导由本人在手机上确认）；被移除的设备不会在重启后再次出现。
- **最近设备**：本地记忆最近使用过的设备（上限 8 台），支持逐条移除与一键清空。
- **伴侣 App**：随附 Android 伴侣应用，用于同网加密会话与配对引导；对已配对无线调试的手机，扫码成功后电脑自动回连，直接进入连接列表。支持常驻通道免扫码一键重连（顺序要求：先在电脑开启常驻通道、再用手机扫码）、手机通知镜像，以及手机→电脑发送文件。

### 镜像与会话

- 基于 **scrcpy 4.1**（固定版本、供应链哈希校验）的高帧率镜像与键鼠控制。
- **会话状态机**：idle / unauthorized / offline / paired / connecting / streaming / failed 七态如实呈现，不把复杂问题塌缩成一句「连接失败」。
- **多台设备同时镜像**：每台手机一个独立镜像窗口、互不干扰；点设备旁的星标即可收藏，收藏项在最近设备列表置顶（只影响本机显示排序，不动连接与授权）；多会话时设置页可选择「应用到哪台设备」只改动选中那一台；托盘菜单逐台列出「结束某设备的镜像」。
- **桌面模式按设备设置**：桌面模式开关与「虚拟屏启动的应用」按设备独立记忆，多台设备互不影响；应用候选下拉按应用名称展示（如「浏览器」「相册」），点选即可，不必看懂包名。
- **能力判定**：按设备 Android 版本如实报告画面控制（SDK ≥ 26）与系统音频（SDK ≥ 30）支持情况，读不到就显示「未知」，不猜测。
- **屏幕唤醒智能化**：读电源策略，DIM（变暗）状态用 BACK 键无损拉回，正常状态零注入。
- **保持唤醒**：会话期间阻止手机熄屏锁屏（USB 与无线均生效；无线通过临时充电模拟与系统设置实现，结束时逐项还原）。
- **锁屏助手**：实时报告锁屏/熄屏状态，在设备卡上以便签形式呈现；支持远程点亮屏幕，镜像窗口内右键同样可以点亮。
- **密码页透明提示**：检测到 PIN/图案输入页（Android 安全表面，镜像黑屏属平台级保护）时，明确提示「此画面受系统安全保护，无法镜像，请在手机上输入密码」，解锁后画面自动恢复——不绕过、不误导。
- **macOS 输入法自动托管**：检测到第三方输入法（微信输入法、搜狗等，会拦截镜像输入的原始键码）时，镜像开始前临时切换到系统自带输入法并提示，会话结束（含退出与崩溃恢复）后自动还原，全程无需手动操作。

### 工具

- **截图**：设备当前画面一键保存，PNG 完整性校验，撞名自动顺延。
- **录屏**：会话内录制为 MP4，运行中的录像受删除保护。
- **文件传输**：与手机的「下载 / MirrorDock」发送区互传文件，支持查看、取回与逐条删除（只删该目录内的这一个文件）；手机伴侣 App 可反向「发送文件到电脑」，到达时桌面实时提示并刷新列表。
- **拖拽传文件**：把文件直接拖进主窗口——`.apk` 安装到当前就绪手机（多个时只装第一个），其他文件进入手机发送区；结果在右下角如实汇报，失败项注明原因。
- **APK 安装**：选择电脑上的安装包一键安装（覆盖安装、保留数据），也可直接拖拽安装；安装需手机端本人确认。
- **手机通知镜像**：手机伴侣 App 开启「通知镜像」并授予系统「读取通知」权限后，新通知实时显示在「工具 → 手机通知」（最多 50 条、可清空；仅内存展示，不落盘、不入日志、不经云端；离线期间的通知不补发）。
- **通知快捷回复**：自带回复栏的通知（微信、短信等聊天应用）可在「工具 → 手机通知」里直接回复；内容经加密通道送回手机，由伴侣 App 以原通知的回复动作发出（回复文字上限 500 字）。手机伴侣不在线时明确提示，不静默丢失；草稿与提示仅留在当前窗口。需桌面端与伴侣 App 均为 0.4.4 / 0.2.5 及以上版本。
- **全局快捷键**：镜像窗口全屏时仍可触发截图 / 录制 / 旋转（仅在会话进行中注册）。
- **托盘与窗口**：系统托盘快捷菜单、关闭窗口最小化到托盘（可配置）、macOS 隐藏 Dock 图标、开机自启；状态区通知支持手动关闭。

### 帮助

- **内置帮助中心**：11 篇完整帮助文档随应用分发，离线可查，覆盖授权原理、无线连接、锁屏与保持唤醒、黑屏与屏蔽操作、文件与通知镜像、多设备管理等常见问题；连接页只留操作，说明性内容全部收进帮助中心。
- **官方主页**：<https://g-star1024.github.io/MirrorDock/>

## 安全与隐私边界

MirrorDock 的安全模型是一条明确的红线，也是产品差异化所在：

- **本地优先**：所有数据（画面、音频、剪贴板、文件）仅在电脑与手机之间直接传输，无云端中转，无默认遥测。
- **供应链校验**：随包分发的 scrcpy / adb 均按官方 SHA-256 校验（流水线强制，哈希不符即终止构建），详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
- **子进程安全**：adb / scrcpy 以固定参数数组直接调用，不经过 shell 拼接，不执行设备或网络提供的内容。
- **最小授权**：仅对已授权设备启动镜像；对锁屏凭据界面（密码 / 图案输入页）只做检测与如实提示，**不做任何绕过**。
- **隐私不落日志**：屏幕帧、剪贴板内容、配对码、密码等敏感数据绝不写入日志；诊断日志仅记录命令成败与设备序列号。
- **签名验证**：发行渠道签名采用 ed25519，公钥编译进二进制，签发私钥永不进入构建环境。

## 系统要求

| 端 | 要求 |
| --- | --- |
| 电脑 | Windows 10+（x64）/ macOS 12+（Intel 或 Apple Silicon）/ 主流 Linux 发行版 |
| 手机 | Android 7.0+（USB 镜像）；Android 11+（无线调试配对） |
| 依赖 | 无需安装。Windows / macOS 安装包内置 scrcpy 与 adb；Linux 安装包同样内置 scrcpy 与 adb（源码编译随包），但需要发行版提供运行库：Ubuntu 25.04+ 可 `sudo apt install libsdl3-0 libavcodec61 libavformat61 libavutil59 libswresample5 libusb-1.0-0`，Ubuntu 22.04/24.04 需先获取 SDL3（见下）。 |

### Linux 运行库说明（诚实版本）

scrcpy 4.x 依赖 **SDL3**，Ubuntu 22.04/24.04 的官方仓库尚未收录。安装 MirrorDock 后若镜像无法启动：

```sh
# 方式一：升级到 Ubuntu 25.04+ / Debian 13+ / Fedora 42+ 等已收录 SDL3 的发行版
# 方式二：手动编译 SDL3（约 3 分钟）：
#   git clone --depth 1 -b release-3.4.16 https://github.com/libsdl-org/SDL
#   cmake -S SDL -B SDL/build -DCMAKE_BUILD_TYPE=Release && cmake --build SDL/build -j4
#   sudo cmake --install SDL/build && sudo ldconfig
```

## 自动更新

客户端内置应用内更新（可关闭思路见「设置 → 关于 MirrorDock → 检查更新」）：启动检查或手动检查 → 发现新版本后下载 → **minisign 数字签名校验**（公钥编译进二进制，私钥不进构建环境）→ 安装 → 重启完成升级。更新包清单（latest.json）来自本仓库 GitHub Release。

## 本地开发

前置条件：Node.js ≥ 20、pnpm、Rust（stable）、Tauri 2 平台依赖、Android Platform Tools（`adb`）。

```sh
pnpm install
pnpm tauri dev
```

开发期 scrcpy 运行时置于忽略的 `.tools/` 目录（或通过 `MIRRORDOCK_SCRCPY_PATH` 环境变量指定）。

### 质量检查

```sh
# 前端：TypeScript 编译 + 单元测试 + 构建
pnpm build
pnpm test

# 后端：测试 + 静态检查
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
```

### 伴侣 App

Android 伴侣应用位于 `companion/`（Kotlin，minSdk 26 / targetSdk 34）：

```sh
gradle -p companion :app:assembleDebug
```

## 项目文档

- [AGENTS.md](AGENTS.md) — 产品边界、工程铁律与协作规则（开发前必读）
- [DEVELOPMENT_TASKS.md](DEVELOPMENT_TASKS.md) — 迭代任务与验收记录
- [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) — 第三方组件声明与供应链哈希

## 技术栈与致谢

MirrorDock 站在众多优秀开源项目的肩膀上，向以下项目与社区致谢：

### 核心依赖

| 项目 | 用途 | 许可证 |
| --- | --- | --- |
| [scrcpy](https://github.com/Genymobile/scrcpy)（Genymobile） | 设备镜像与控制的底层引擎，v4.1 随包分发 | Apache-2.0 |
| [Android Platform Tools](https://developer.android.com/tools/releases/platform-tools)（adb） | 设备通信、状态探测与受控注入 | Apache-2.0 |
| [Tauri](https://tauri.app/) | 跨平台桌面应用框架（v2，含托盘、全局快捷键、自启插件） | MIT / Apache-2.0 |
| [React](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) | 前端界面 | MIT / Apache-2.0 |
| [Vite](https://vite.dev/) | 前端构建与开发服务 | MIT |
| [Rust](https://www.rust-lang.org/) | 后端逻辑与安全边界实现 | MIT / Apache-2.0 |
| [Kotlin](https://kotlinlang.org/)（JetBrains） | Android 伴侣 App | Apache-2.0 |

### Rust 生态

- [tokio](https://tokio.rs/) — 异步运行时（伴侣 App 加密会话）
- [rustls](https://github.com/rustls/rustls) / [tokio-rustls](https://github.com/rustls/tokio-rustls) — TLS 1.3 加密通道
- [rcgen](https://github.com/rustls/rcgen) — 自签证书生成（配对指纹校验）
- [ed25519-dalek](https://github.com/dalek-cryptography/ed25519-dalek) — 发行签名验证
- [serde](https://serde.rs/) — 序列化框架

### 前端生态

- [qrcode](https://github.com/soldair/node-qrcode) — 无线配对二维码
- [Vitest](https://vitest.dev/) + [Testing Library](https://testing-library.com/) — 前端测试
- [jsdom](https://github.com/jsdom/jsdom) — 测试环境

### 特别感谢

- **Genymobile 团队**：scrcpy 是本项目得以存在的基石，其工程质量与 Apache-2.0 授权让本地镜像工具成为可能。
- **Android Open Source Project**：adb 与底层设备协议的开放生态。
- **Tauri 社区**：用 Web 技术构建轻量桌面应用的工程实践与持续维护。

完整的第三方许可声明与供应链哈希见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 许可证

本项目以 [Apache License 2.0](LICENSE) 发布，与 scrcpy 保持一致。

软件按「现状」提供，不含任何担保。使用本项目即表示您了解并同意：镜像能力受设备端安全策略约束（如 FLAG_SECURE 内容不可捕获），MirrorDock 不提供、也不会提供绕过设备安全机制的功能。
