# MirrorDock 证据与验收规则

> 本文件回答一个问题：**什么才算"做完了"**。核心原则：证据不可替代，外部条件缺失时如实标记，不得伪造通过。

## 1. 三层证据，缺一层就不算完成

| 层级 | 内容 | 最低要求 |
| --- | --- | --- |
| 静态 | 代码改动本身 | 改动文件清单 + 为什么这样改（含被否决的方案与理由） |
| 自动 | 可重复执行的检查 | 实际执行的命令与真实输出（前端 `pnpm build`；Rust `cargo test`；Gradle 走 `gradle-run` 包装器） |
| 行为 | 人在真实环境中的观察 | 一条成功路径 + 一条**真实恢复路径**（拔线 / 撤销授权 / 锁屏 / 网络切换 / ADB 重启 / 拒绝授权） |

只跑单元测试、只截图 UI、只"看起来没问题"，都不构成完成。

## 2. 不可替代的验收项（缺条件时必须标为外部阻塞）

以下项目**无法**用模拟、桩、mock 或推理替代。条件不具备时，在 `DEVELOPMENT_TASKS.md` 保持未勾选并写明缺失证据：

- 真实 Android 设备上的首帧、鼠标键盘、音频
- USB 与 Wi-Fi **各自独立**的验证（不可用一条路径的结果代表另一条）
- Android 11+ 无线配对、重连、忘记设备、网络切换恢复
- 授权撤销、锁屏、受保护内容（`FLAG_SECURE`）行为
- 60 分钟会话稳定性与性能基线（P95 首帧 < 5 秒）
- 30 台设备矩阵、三桌面系统（Windows / macOS / Ubuntu）构建与运行
- 代码签名凭据、商店账号、第三方审核结论
- 发行物 SBOM、NOTICE、许可证清单、漏洞扫描结果、签名验证、更新与回滚

## 3. 状态语义（禁止塌缩）

会话与设备状态必须分别可表达且可恢复：

`未授权(unauthorized)` · `离线(offline)` · `已配对(paired)` · `连接中(connecting)` · `镜像中(streaming)` · `失败(failed)`

每个失败态必须携带：**原因** + **影响** + **修复动作** + **回退路径**。

特别注意：`Running` 只表示进程在跑，**不代表首帧已到达**。不要用"已启动"当作"已镜像"的证据。

## 4. 隐私与日志红线（验证过程本身也要遵守）

- 屏幕帧、剪贴板内容、输入的口令、访问令牌、配对码 —— **一律不得**写入日志、报告、截图或提交记录。
- 测试记录中不写 IMEI、电话号码、账号。
- 真机运行数据只落盘到被 Git 忽略的 `test-runs/`，不纳入版本库。
- 面向用户的诊断信息必须脱敏，且仅在显式同意后生成。

## 5. 交付报告模板

每次交接按以下七项报告，缺项要显式写"无"而不是省略：

1. 受影响平台与渠道（Windows / macOS / Ubuntu × 国内 / Play / 企业侧载）
2. 权限与数据（新增/变更的权限、触碰的数据类型）
3. 改动文件
4. 验证证据（命令 + 输出 + 人工观察）
5. 兼容性影响（Android 版本下限、OEM 差异、桌面系统）
6. 许可证 / 政策影响（依赖变更、scrcpy 版本与哈希、渠道政策）
7. 未关闭风险与回滚行为

## 6. 门禁不可被指标覆盖

安全性、政策合规或许可证问题一旦成立，**不得**以产品指标（完成率、性能、进度）为由放行。

## 7. 发版完成度：tag 是唯一开关（2026-10-02 血泪教训）

**事故**：v0.4.5-beta 的桌面模式按设备设置 + 应用名下拉（X10-71）代码早已落地、测试全绿、`DEVELOPMENT_TASKS.md` 也标了「已发版」，但 `build.yml` **只在 `tags: v*` 时才打包**——而这个 tag 从未推送。结果用户手上的版本不含该特性，而台账声称已交付。**特性从未到达用户，误报持续了数轮。**

### 规则

1. **代码落地 ≠ 发版完成。** 标 ✅ 发版前必须同时满足：
   - `git ls-remote --tags origin` 能查到该 tag；
   - GitHub 上存在对应 Release（`draft=false`）且**资产齐全**；
   - `latest.json` 的 `version` 等于该版本号。
2. **"发版证据待 CI 完成后补记"这类占位句等于没发版。** 若 CI 未跑完，行末必须保持未勾选，并显式写「tag 已推 / CI 进行中 / Release 未核验」三段状态。
3. **版本号一处都不能少改。** 桌面版本号共 8 处口径，必须全量同步：
   `src-tauri/tauri.conf.json`、`package.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`README.md`、`site/index.html`、`site/compatibility.html`、`docs/compatibility-matrix.md`。改完用 grep 反查旧版本号，确认只剩 `DEVELOPMENT_TASKS.md` 的历史记录。
4. **误标的发布说明要删。** 若 `docs/releases/vX.md` 描述了一个从未发版的版本，删除该文件，不要留着误导后续会话。

### 本地打包与签名（已实测）

| 目的 | 做法 | 注意 |
| --- | --- | --- |
| 只出 Mac `.app` | `pnpm tauri build --bundles app`（约 6 分钟） | **不要**带 `TAURI_SIGNING_PRIVATE_KEY`：macOS `codesign` 会去解锁登录钥匙串，后台无 TTY 时永久卡在 `Password:` 提示（实测挂起 8m44s 后手动终止） |
| updater 签名归档 | 交给 CI（GitHub secret `TAURI_SIGNING_PRIVATE_KEY`） | 私钥在 `MirrorDock-内部文档/mirrordock-updater.key`（minisign 私钥、**无密码**），公钥 `.pub` 与 `tauri.conf.json` 的 `pubkey` 配对一致 |
| 伴侣 APK | 推 tag 走 CI：`companion.yml` / `build.yml` 的 `companion-apk` job | gradle 在 runner 上跑，签名取 `COMPANION_KEYSTORE_BASE64` / `COMPANION_STORE_PASSWORD` secret。**本地没有 gradle 也出得了 APK，不要据"本地无 gradle"断言 APK 打不出来** |

### 发版后核验清单

- Release：唯一非草稿、`prerelease` 标志正确、**资产数量与文件名齐全**（四平台安装包 + updater 归档 + `.sig` + `SHA256SUMS` + SBOM + `latest.json` + 伴侣 APK）。
- updater 端点：解析 `latest.json`，确认 `version` 正确、`platforms` 覆盖四个平台且**每个平台都有非空 `signature`**。
- 沙箱网络：`curl` 直连 `github.com/.../releases/download/...` 常被拦；改用 GitHub API（`api.github.com/repos/:owner/:repo/releases`、assets API）核验。`gh` CLI 通常不在非交互 shell 的 PATH 中，用 `git ls-remote` 或 GitHub MCP 工具替代。
- 版本考古：报告"某特性没生效"之前，先查**远端 tag 与 Release 资产**，再查代码——顺序反了会误判成漏写代码。

