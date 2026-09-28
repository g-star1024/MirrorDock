---
name: mirrordock-development
description: MirrorDock（Android 桌面镜像工具）全部工程工作的入口技能——实现/测试/评审/打包/发布/诊断 Tauri+Rust 桌面端、ADB 与 scrcpy 集成、USB 与无线调试配对、首次连接引导与故障恢复、设备会话状态机、Android 伴侣 App、兼容性矩阵、三渠道（国内 / Google Play / 企业侧载）构建与合规时使用。不适用于其他仓库。
---

# MirrorDock Development

为**非技术用户**在自己电脑上连接**自己的** Android 手机，交付**最小且安全**的改进。

## 启动闸门（每次必做，不可跳过）

1. 读取仓库 `AGENTS.md`、`Android桌面镜像工具产品规划.md` 的相关章节、`DEVELOPMENT_TASKS.md`、`agents/TEAM_AGENTS.md`，以及当前实现与测试代码。
2. **动手前先声明**：受影响的桌面平台（Windows / macOS / Linux）、Android 版本范围、连接方式（USB / Wi-Fi / 无）、涉及的权限与数据、本次将产出的验收证据。
3. 按下表加载**最窄匹配**的技能（不要一次性全载）。

## 技能路由表

| 工作所在面 | 加载技能 |
| --- | --- |
| 执行 `gradle` / `./gradlew`，或诊断 Gradle 构建、测试、lint 失败 | `gradle-run` |
| AndroidManifest 组件、自定义权限、Service/Receiver/Provider、IPC 边界 | `android-permissions-security` |
| 处理外部传入 Intent、`getParcelableExtra`、PendingIntent、Intent 重定向 | `android-intent-security` |
| 伴侣 App 的协程作用域、StateFlow/SharedFlow、Channel、取消与生命周期 | `kotlin-concurrency-and-flow` |
| Compose UI 测试、截图测试、语义断言、baseline 录制 | `compose-ui-testing-patterns` |
| Android 测试策略、测试框架接入、覆盖率与测试脚手架 | `testing-setup` |
| Google Play 政策、Data Safety、账号删除、Accessibility API 申报 | `play-policy-insights` |
| 浏览器 / WebView 端到端 UI 流程自动化 | `playwright` |
| 某个模块或功能的语言/框架级安全审查（Python / JS-TS / Go） | `security-best-practices` |
| 架构级威胁建模、信任边界、资产与滥用路径枚举 | `security-threat-model` |
| 处理当前分支 PR 的评审意见 | `gh-address-comments` |
| 修复失败的 GitHub Actions PR 检查 | `gh-fix-ci` |

注意：Rust 侧（`src-tauri/`）与 Rust 代码审查**没有**对应技能，不要为凑路由而加载无关技能；直接按 `AGENTS.md` 与本文约束工作。

## 产品约束（红线）

- ADB + scrcpy 是 MVP 首选路径，但**必须**有设备所有者的显式调试授权。保持"用户知情同意 + 数据本地优先 + 受保护内容受限"三条不变。
- 发现、授权、会话状态、媒体、输入、UI 必须分属**不同接口**。子进程**只允许固定参数直接调用**，禁止 shell 拼接与插值。
- 只在序列号已处于已授权（`device`）状态时启动 scrcpy；不得对未知或未授权序列号发起会话。
- 会话状态必须区分：未授权 / 离线 / 配对中 / 连接中 / 镜像中 / 失败。**禁止**把任何情况都塌缩成"连接失败"，每个错误都要给出原因、影响、修复动作与回退路径。
- 绝不绕过锁屏、DRM / `FLAG_SECURE`、受保护系统页面、MDM 策略或用户同意。禁止无人值守的通用控制、隐藏会话、默认云端录制、默认遥测。
- 会话、录制、音频、剪贴板、文件传输、可选无障碍控制必须**可见且可撤销**。屏幕帧、剪贴板、输入的口令、访问令牌**一律不得写入日志**。
- scrcpy 集成需具备可审计的 来源 / 版本 / 许可证 / NOTICE / SBOM / 升级测试证据；运行时能力探测通过前，不得宣称通用支持。
- 产品边界：通用 Android App 无法承诺无授权、后台常驻、绕过 DRM 或无感控制。

## 团队与交接

- 角色边界见 `agents/TEAM_AGENTS.md`（交付负责人、桌面与媒体工程、Android 伴侣工程、UX/引导/客服、QA/设备实验室、安全与合规）。
- 任何成员都可因**权限、隐私、供应链或发布门禁**问题阻断交付。
- 每次交接必须报告：受影响平台与渠道、权限/数据、改动文件、验证证据、兼容性影响、许可证/政策影响、未关闭风险。

## 完成标准（Done）

1. 运行**最窄**的相关自动化检查（前端 `pnpm build`；Rust `cargo test --manifest-path src-tauri/Cargo.toml`；涉及 Gradle 时走 `gradle-run` 包装器）。
2. 走通一条**成功路径**和一条**真实的恢复路径**（例如拔线、撤销授权、锁屏、网络切换）。
3. 若某项验收依赖真机、签名凭据、商店账号或第三方审核，且这些条件不具备——**记录为显式未验证面**，不得伪造验收、不得标记 ✅。
4. 同一次改动内更新 `DEVELOPMENT_TASKS.md`；仅**有证据**的条目可标 `✅`，未完成或外部阻塞的条目保持未勾选并写明缺失证据。
5. 报告内容：执行命令、未验证面、兼容性、政策/许可证影响、回滚行为。

## 参考

- `references/mirrordock-context.md` — 仓库结构、构建命令、阶段验收表、当前阻塞项速查
- `references/verification-evidence.md` — 证据要求与不可替代验收项清单
