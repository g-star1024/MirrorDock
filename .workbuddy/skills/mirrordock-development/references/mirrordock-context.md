# MirrorDock 项目上下文速查

> 本文件是 `mirrordock-development` 技能的参考材料。权威来源始终是仓库内的 `AGENTS.md`、`Android桌面镜像工具产品规划.md`、`DEVELOPMENT_TASKS.md`、`agents/TEAM_AGENTS.md`；本文件仅作快速定位，若与仓库文档冲突，以仓库文档为准。

## 1. 仓库结构与职责

| 路径 | 职责 |
| --- | --- |
| `AGENTS.md` | 工程铁律（启动闸门、产品与安全契约、质量与发布、团队与技能） |
| `Android桌面镜像工具产品规划.md` | 产品决策、MVP 范围、核心体验与边界、阶段验收表、当前 Backlog |
| `DEVELOPMENT_TASKS.md` | 任务清单与验收状态；每次改动须同步更新 |
| `agents/TEAM_AGENTS.md` | 6 个角色边界、跨角色集成规则、阻断权限 |
| `docs/POC_ACCEPTANCE.md` | POC 真机验收表（USB / Wi-Fi / 性能稳定性） |
| `docs/POC_TEST_REPORT.md` | 真机测试已完成的运行记录 |
| `THIRD_PARTY_NOTICES.md` | scrcpy 来源、版本、哈希、许可证与发布前义务 |
| `src/App.tsx`、`src/App.css` | React 前端（单页，含设备/会话/无线诊断 UI） |
| `src-tauri/src/lib.rs` | Rust 后端：ADB 探测、scrcpy 启动、无线配对与可信设备 |
| `src-tauri/capabilities/default.json` | Tauri 能力（当前仅 `core:default`、`opener:default`） |
| `scripts/measure-session.sh` | 真机会话测量脚本，输出到被忽略的 `test-runs/` |
| `.tools/scrcpy/<platform>/` | 开发期 scrcpy 运行时（**被 Git 忽略**，不随发行物分发） |

## 2. Tauri 命令面（`src-tauri/src/lib.rs`）

| 命令 | 说明 |
| --- | --- |
| `check_adb_devices` | 探测 ADB 设备与授权/离线状态，返回结构化结果 |
| `start_mirroring` | 对已授权序列号启动 scrcpy（固定参数直接调用） |
| `stop_mirroring` | 结束当前会话并终止镜像进程；无会话时返回 `session_not_running` |
| `mirror_session` | 读取当前会话状态（前端轮询用） |
| `pair_wireless_device` | Android 11+ 无线调试配对（一次性配对码） |
| `connect_wireless_device` | 用连接地址连接无线设备 |
| `list_trusted_wireless_devices` | 列出本机可信无线设备 |
| `forget_trusted_wireless_device` | 忘记可信设备并发起 ADB 断开 |

新增命令时同步检查：`generate_handler!` 注册、能力配置、结构化错误码、前端可恢复状态展示。

### 会话状态机（`SessionPhase`）

`idle` · `unauthorized` · `offline` · `paired` · `connecting` · `streaming` · `failed`

七个阶段彼此可区分，**禁止**塌缩成单一的"连接失败"。失败态必须携带结构化 `error`（`code` + `message` + `recovery`）。

`MirrorSession.first_frame`（`unknown` / `reached`）单独表达首帧到达情况：**`streaming` 只表示进程在运行，不代表首帧已到达**。当前尚未实现端到端首帧探针，因此只会产生 `unknown`；`reached` 预留给真实探针，无证据时不得上报。

### 错误码契约（`AppError.code`）

对前端稳定，修改修复文案时不要改动错误码：

```
device_not_selected / device_unauthorized / device_offline / device_not_connected
adb_missing / adb_unavailable
mirror_runtime_missing / mirror_start_failed / mirror_exited / mirror_stop_failed
session_unavailable / session_busy / session_not_running
invalid_rotation / endpoint_invalid / pairing_code_invalid / pairing_failed
connect_failed / connect_not_ready
trusted_list_unavailable / trusted_list_unreadable / trusted_list_write_failed
```

### 可替换的运行时接口

- `AdbRuntime`：`list_devices` / `pair` / `connect` / `disconnect`
- `MirrorRuntime`：`is_available` / `start` → `Box<dyn MirrorProcess>`
- `MirrorProcess`：`try_wait` / `kill`

三者都是 trait，测试中用 fake 实现即可验证完整会话状态机，**不需要真机**。新增业务逻辑时应优先写成接受 `&AppRuntimes` 与 `&SessionStore` 的自由函数（如 `start_mirroring_with`、`stop_mirroring_with`），由 Tauri 命令层薄封装，以便单测覆盖。

## 3. 构建与验证命令

```sh
# 前置：Node.js + pnpm、Rust、Tauri 桌面依赖、Android Platform Tools（adb）
pnpm install
pnpm tauri dev              # 桌面开发运行
pnpm build                  # 前端：tsc + vite build（最窄前端检查）
cargo test --manifest-path src-tauri/Cargo.toml   # Rust 单元测试
pnpm tauri build            # 桌面发行构建（打包/签名尚未完成）
scripts/measure-session.sh <serial>   # 真机会话测量 → test-runs/
```

涉及 Gradle 时**必须**走 `gradle-run` 技能的包装器，不得直接流式输出完整构建日志。

## 4. 阶段验收标准（摘自产品规划）

| 阶段 | 交付 | 退出标准 |
| --- | --- | --- |
| POC（1–3 周） | 三系统 USB/Wi-Fi 原型、10 台真机报告 | 目标机型 USB 首帧 < 3 秒，稳定交互 |
| Alpha（4–8 周） | 核心连接、会话 UI、录制、诊断、签名包 | 30 台矩阵首连成功率 ≥80% |
| Beta（9–12 周） | 文件/剪贴板/预设、引导与崩溃诊断 | P95 首帧 < 5 秒；60 分钟会话不崩溃 |
| v1（13–16 周） | 付费、帮助中心、兼容性页、支持流程 | 渠道、隐私、安全、供应链门禁通过 |

目标平台：桌面 Windows 10/11、macOS 13+、Ubuntu 22.04+；Android 8+ 画面与控制，Android 11+ 系统音频（受应用捕获策略限制）。

## 5. 当前实况与阻塞项（截至 2026-09-28）

已真机验证：Android 13 / Xiaomi Redmi M2104K10AC 同局域网无线配对 + 连接 + 实际镜像成功；动态画面峰值约 35 FPS（macOS x86_64，Metal，1072×2400）。

**尚未验证（属外部阻塞，不得伪造验收）**：

- USB 首帧 / 键鼠 / 拔线恢复 / 撤销授权恢复（P0-05）
- 无线重连、忘记设备、网络切换恢复（P0-06 部分）
- 首帧秒数、输入时延、掉线、60 分钟稳定性（P0-07 部分）
- 30 台设备矩阵、三桌面系统 Alpha 报告（A1-07）
- 发行打包、NOTICE、SBOM、构建哈希与签名验证（A1-08）
- 三渠道（国内 / Google Play / 企业侧载）隐私、权限、签名、更新与支持材料（R3-02）
- v1 Android 伴侣 App 全部内容（C4-*）

## 6. scrcpy 供应链要点

- 上游 `Genymobile/scrcpy`，Apache-2.0，开发基线 **v4.1**。
- macOS x86_64 官方静态包 SHA-256：`ee2a7223bc8dbdc4f482db1134bcf441178dafb833492b71ca4c22090c58ce72`；**解包前必须校验**。
- 开发模式只从经校验的 `.tools/scrcpy` 或显式 `MIRRORDOCK_SCRCPY_PATH` 查找；发行模式不得依赖开发目录。
- 固定参数、直接执行、不使用 shell；**绝不执行设备或网络提供的内容/二进制**。
- 升级 scrcpy 后必须重新验证镜像、输入、音频与许可证义务。
