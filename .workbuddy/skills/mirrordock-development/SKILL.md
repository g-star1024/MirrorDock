---
name: mirrordock-development
description: MirrorDock（Android 桌面镜像工具）全部工程工作的入口技能——实现/测试/评审/打包/发布/诊断 Tauri+Rust 桌面端、ADB 与 scrcpy 集成、USB 与无线调试配对、首次连接引导与故障恢复、设备会话状态机、Android 伴侣 App、兼容性矩阵、三渠道（国内 / Google Play / 企业侧载）构建与合规时使用。不适用于其他仓库。
---

# MirrorDock Development

为**非技术用户**在自己电脑上连接**自己的** Android 手机，交付**最小且安全**的改进。

## 启动闸门（每次必做，不可跳过）

1. 读取仓库 `AGENTS.md`、产品规划文档（**已移出仓库**，位于 `<仓库上级目录>/MirrorDock-内部文档/Android桌面镜像工具产品规划.md`）的相关章节、`DEVELOPMENT_TASKS.md`、`agents/TEAM_AGENTS.md`，以及当前实现与测试代码。
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
| 设备会话期间保持亮屏、排查镜像黑屏/屏幕自动熄灭/无线调试掉线 | `android-screen-awake-forensics` |
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

## 发版与打包（最高频误判区，先读这里）

**签名密钥不是问题**：updater 私钥 `MirrorDock-内部文档/mirrordock-updater.key`（minisign，**无密码**）与公钥 `.pub` 均已配置，`tauri.conf.json` 的 `pubkey` 与之配对一致；伴侣 APK keystore 在 `mirrordock-companion.keystore`。**不要**再因"找不到密钥"而判定无法签名。

**APK 不是打不出来**：伴侣 APK 走 CI（`.github/workflows/companion.yml`、`build.yml` 的 `companion-apk` job），gradle 在 runner 上运行，签名取仓库 secret。**本地没装 gradle 不构成阻塞。**

**代码落地 ≠ 发版完成**：`build.yml` 仅在 `tags: v*` 触发。标「已发版」前必须核验三件事——远端 tag 存在、Release 非草稿且资产齐全、`latest.json` 的 `version` 对得上。v0.4.5-beta 曾因漏推 tag 而让特性从未到达用户，台账却误标已发版（详见 `references/verification-evidence.md` 第 7 节）。

**报告"某特性没生效"时的排查顺序**：先查远端 tag 与 Release 资产 → 再查版本号是否八处同步 → 最后才查代码。顺序反了会把"漏发版"误判成"漏写代码"。

**本地只出 Mac `.app` 时不要设 `TAURI_SIGNING_PRIVATE_KEY`**：会触发 macOS `codesign` 解锁登录钥匙串，后台无 TTY 时永久卡在 `Password:`。updater 签名交给 CI。

**沙箱网络**：`curl` 直连 `github.com/.../releases/download/...` 常被拦，改用 GitHub API 或 GitHub MCP 工具；`gh` CLI 通常不在非交互 shell 的 PATH。

**升级到新版本时的版本号口径（共 8 处）**：`src-tauri/tauri.conf.json`、`package.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`README.md`、`site/index.html`、`site/compatibility.html`、`docs/compatibility-matrix.md`。

## 参考

- `references/mirrordock-context.md` — 仓库结构、构建命令、阶段验收表、当前阻塞项速查
- `references/verification-evidence.md` — 证据要求、不可替代验收项清单、发版完成度核验（tag/Release/latest.json）

## ⚠️ 「app」有两个指代 —— 动手前必须确认是哪个端（2026-10-03 血泪）

MirrorDock 交付**两个端**，它们都叫"app"：

| 端 | 位置 | 发布方式 | 用户会怎么称呼 |
| --- | --- | --- | --- |
| **PC 桌面端** | `src/` + `src-tauri/`（Tauri+React） | `vX.Y-beta` tag → 四平台安装包 | "电脑端"、"桌面版" |
| **安卓伴侣 App** | `companion/`（Kotlin+XML） | 随桌面版重打包，APK 名为 `MirrorDock-companion-X.Y.Z.apk` | **"app"、"客户端"、"装到手机上"** |

**用户在 2026-10-03 的原话**：「手机发送到电脑的文件，客户端一直看不到」「调用你的设计 skill，给 app 整体做个设计升级」「修改完打包推送把新包安装到**手机**」—— 全程指**安卓伴侣 App**。我却改了一整轮 PC 桌面端（做了 impeccable 全局审计、设计令牌层、暗色主题、信息架构重组……），还把「PC 端已有的 `appVersion` 挪到侧栏」当成"看不到版本信息"的修复，而**安卓端一个版本号都没显示**。

**为什么容易错**：项目历史 6 次发版全是桌面端，我形成了惯性。

**铁律**：
1. 用户说"app / 客户端 / 安装到手机 / 打包推送"时，**先确认是哪个端**，别从最近上下文推。
2. 用户报"看不到 X"时，**先 grep 确认它到底存不存在**。PC 端存在（只是位置错）、安卓端不存在（真缺失）—— 不查就动手，方向从根上就错。
3. 「先调研再动手」的习惯是对的，但**调研对象错了，习惯对了等于白做**。这个错误本可以用一句话确认避免。

## 安卓端（companion）的本地验证能力

- **本地无 gradle**（沙箱拦大文件下载，gradle 发行版拉不下来），**真编译只能靠 CI**。
- 本地两道强制闸门（`AGENTS.md` 有登记）：
  - `python3 scripts/verify-companion.py` —— 8 类静态交叉检查
  - `python3 scripts/check-companion-kotlin.py` —— **真实 kotlinc 类型检查**（用 Gradle 缓存里的编译器 + `android.jar`，**必须用系统 `/usr/bin/java`（Temurin 17）**，Android Studio 的 JBR 是 JDK 25，kotlinc 2.0.20 解析不了）
- **本地 checker 的已知盲区**（2026-10-03 修掉两处）：R 存根原本只覆盖 6 种资源、**漏了 `dimen`**（导致所有 `dp(R.dimen.*)` 报假 unresolved reference，真错误被淹没）；且只读 `res/values/`、**漏掉 `res/values-night/`**。**新增资源类型或新的 values-* 限定符目录时，要同步更新 R 存根。**
- **CI 上唯一能验证、本地验证不了的部分**：`assembleDebug`（aapt 资源编译 + dex + 打包）。本地 kotlinc 只做类型检查，**不过资源链接**。
- **APK 分发的现实**：`companion.yml` 只上传 artifact，**而 GitHub artifact 下载端点需要认证**（沙箱无 token）。要装到手机上，需要：① 用户提供 PAT，或 ② 打 tag 走 `build.yml` 的 `companion-apk` job（产物会进 Release 的 assets，那里下载不需要认证）。

## 改完 UI 必须抓屏（2026-10-03 血泪）

X10-76 那一轮我改了 13 个文件（新建 `dimens.xml` + `values-night/` + 版本显示），
跑通了两道本地门禁（`verify-companion.py` + `check-companion-kotlin.py` 真实 kotlinc
类型检查），CI 9 步全绿（含 aapt + dex）—— **但没有抓一张真机截图**。

结果用户截图指出两处**视觉上明显错乱**的缺陷：

1. **「端到端加密」被挤成两行**（显示成「端到端加」/「密」）——
   `quality_row` 是横向 LinearLayout，三个子 TextView 全是 `wrap_content`，
   按内容抢宽度，状态文案一长第三个就被压到折行。
2. **版本行与隐私摘要不在卡片里** —— 我插入时锚在了日志卡的闭合标签处，
   于是它们成了 ScrollView 的直接子节点（卡的**兄弟**），直接躺在灰色页面上，
   与上方五张白卡完全不是一套视觉。

**两处都是"结构合法、类型正确、编译通过"** —— 静态检查、类型检查、aapt、
dex 全都抓不到。**只有看渲染结果才能发现。**

### 铁律

- **改完 `activity_main.xml` 或任何视觉样式后，抓一次真机截图确认**：
      adb -s <serial> exec-out screencap -p > /tmp/x.png
  然后**真的用 Read 工具看那张图**。截图存下来不等于看过。
- 改完记得恢复手机设置（我为了测暗色 `cmd uimode night yes`，验完必须改回 `no`）。
- **横向 LinearLayout 里有 3 个以上 `wrap_content` 子控件就要警惕** ——
  它们会互相抢宽度。把承载关键信息（尤其安全承诺、错误提示）的那一个
  拆到独占一行，或给它 `layout_weight=1` 吃掉剩余空间。
- **插入新元素时锚点要选"内容容器内部"**，不是"容器的闭合标签处" ——
  后者会让你插到卡片外面去。
