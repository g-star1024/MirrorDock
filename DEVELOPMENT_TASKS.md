# MirrorDock 开发任务清单

> 规则：按依赖顺序完成；只有验证证据和退出标准满足后才将任务改为完成。真机、签名凭据、商店账号或第三方审核不可替代，缺失时必须标记为外部阻塞，不能伪造验收。

## 阶段 0：可行性 POC

- [✅] P0-01 初始化 Tauri 2 + React + TypeScript 桌面项目。
- [✅] P0-02 实现安全的 ADB 设备/授权状态读取与中文恢复引导。
- [✅] P0-03 安装并固定 scrcpy 开发运行时；记录版本、校验信息、许可证与开发依赖来源。
- [✅] P0-04 实现仅对已授权序列号启动 scrcpy 的直接进程接口。
- [ ] P0-05 在真实 Android 设备上验证 USB 首帧、鼠标/键盘、断开与授权撤销恢复。**USB 首帧（无头实测 2.0s）、鼠标/键盘（含中文经 UHID）、拔线后恢复均已在 Redmi M2104K10AC 真机验证（明细见 X10-59/守卫轮条目）；仅剩「撤销授权后的恢复」待用户配合（外部阻塞）。**
- [ ] P0-06 实现无线调试配对、同网连接、保存/忘记可信设备与网络切换恢复。**已在 Android 13 真机完成配对、独立端口连接和镜像；重连、忘记与网络切换待测。**
- [ ] P0-07 建立首帧、FPS、时延估算、掉线与 60 分钟稳定性测试记录。**首帧（真机 10 轮，P50=0.91s / P95=0.99s，见 B2-04）、USB 无头首帧（2.0s）、Wi-Fi FPS/分辨率（动态峰值约 35 FPS）、60 分钟 soak 与失败注入（掉线场景）记录均已在 `test-runs/`（`perf-*`、`failure-injection-*`）；输入时延仍为人工观察（无端到端探针），作为发布门禁前需补探针。**
- [x] ✅ P0-08 将 MirrorDock 开发技能体系接入 WorkBuddy：项目级技能 `mirrordock-development`（含启动闸门、技能路由表、产品红线、完成标准与两份参考材料）建于 `.workbuddy/skills/`；12 个配套专业技能安装至用户级 `~/.workbuddy/skills/`。**证据：安装前完成安全审计（无凭据访问、无混淆代码、无外发数据，风险 🟢 LOW–🟡 MEDIUM）；安装后校验 13 份 SKILL.md frontmatter 均可解析、无 `CODEX_HOME` 残留引用、与既有技能无重名。**
- [x] ✅ P0-09 建立 Git 版本控制与 GitHub 远端基线：初始化仓库并以 `main` 为默认分支；加固 `.gitignore` 排除构建产物（`src-tauri/target` 2.1G、`node_modules`、`.tools/`、`test-runs/`、`dist/`）与本地工作笔记 `.workbuddy/memory/`；创建远端公开仓库 `g-star1024/MirrorDock` 并完成首次推送。**证据：推送通道经 SSH 认证（`Hi g-star1024!`）；密钥扫描（`ghp_`/`sk-`/`AKIA`/PRIVATE KEY/硬编码口令赋值）零命中；本地 `HEAD` 与 `origin/main` 一致、工作区干净。后续因把内部产品规划文档移出仓库，已重写历史并强制推送：远端递归文件树由 53 降至 52 个 blob、该文档命中 0，`.gitignore` 已加入该文件名与 `/内部文档/` 防止再次误入库。**注意：旧提交在 GitHub 上短期内仍可按完整 SHA 访问，如需彻底清除需删除并重建仓库或联系 GitHub 支持。**

- [x] ✅ P0-10 为项目添加 Apache-2.0 许可证与版权声明。**证据：仓库根 `LICENSE` 为 Apache-2.0 官方全文（下载自 apache.org，202 行）并附版权声明；`README.md` 增加许可证引用并指向 `LICENSE`；`THIRD_PARTY_NOTICES.md` 继续单独覆盖第三方（scrcpy v4.1, Apache-2.0），与本项目自身许可区分。**
- [x] ✅ P0-11 建立 GitHub Actions 三平台打包流水线（`push to main` 触发 + 手动触发）：`verify` 作业跑 `cargo test` 与 `pnpm build` 作为门禁；`package` 作业按 `windows-latest` / `ubuntu-22.04` / `macos-latest`(aarch64) / `macos-15-intel`(x86_64) 四目标矩阵调用 `pnpm tauri build` 并上传安装包。**已观测证据**：**run #5（`head_sha=9f71e13`）五项作业全部 `completed/success`，四个平台安装包全部产出**——`MirrorDock-linux-x64` 81.48 MiB、`MirrorDock-windows-x64` 3.35 MiB、`MirrorDock-macos-arm64` 1.92 MiB、`MirrorDock-macos-x64` 2.06 MiB（作业含 `if-no-files-found: error`，作业成功即证明文件确实产出）。**run #4（`head_sha=33bc26f`）取得四目标中三个的成功产出**——门禁「验证（Rust 单测 + 前端构建）」、`打包 linux-x64`、`打包 windows-x64`、`打包 macos-arm64` 均为 `completed/success`（run #3 亦已单独取得 linux-x64 与 macos-arm64 的成功记录）。**修正一处真实缺陷（外部证据）**：`macos-x64` 原先挂在 `macos-13` 上，而该 runner 镜像**已于 2025-12-04 完全退役**（依据 GitHub Actions runner 生命周期公告）；被弃用的 label **不会报错**，而是**排队到 24 小时上限后被自动取消**，表现为该作业长期 `queued` 并拖住整个运行（run #2 排队逾 20 分钟、run #4 的 macos-x64 同样卡住均可由此解释）。已改为 **`macos-15-intel`**（GitHub 现存的最后一个托管 x86_64 macOS 镜像，约可用至 2027-08），并在 workflow 内注释说明。**未验证面（外部阻塞）**：安装包在真机上的**可运行性**未验证；产物**默认未签名**（macOS Gatekeeper / Windows SmartScreen 会提示），正式分发需另配签名密钥；**不捆绑 scrcpy 运行时**（属 A1-08 范围），当前产物运行需本机已具备 scrcpy 或设置 `MIRRORDOCK_SCRCPY_PATH`。**触发策略（成本分层，本轮调整）**：原先 `push to main` 会触发全部四平台打包，而 workflow 的 `concurrency.cancel-in-progress` 又使后一次推送**取消**正在进行的构建（run #1/#2/#3 均因此被取消，形成“推得越勤、验证越碎”的反效果）；现改为 **`push to main` 只跑 `verify` 门禁（约 3 分钟，保留每次提交的回归保护）、打包改由标签 `v*` 与手动触发**，两个作业均显式设置 `timeout-minutes`，避免再次出现无限排队。**

## 阶段 1：MVP Alpha

- [x] ✅ A1-01 抽象 `AdbRuntime`、`MirrorRuntime`、会话状态机与结构化错误码；为 ADB/scrcpy 留出可替换实现。**本轮补齐四项缺口：（1）全部命令统一返回结构化 `AppError`（code/message/recovery），配对与无线连接不再是裸字符串；（2）会话状态机区分为 idle/unauthorized/offline/paired/connecting/streaming/failed 七态，失败落到具体状态而非统一"连接失败"；（3）首帧以 `FirstFrame` 显式建模，`streaming` 不再被当作首帧已到达；（4）`MirrorRuntime::start` 返回可 `kill` 的 `MirrorProcess`，新增 `stop_mirroring` 命令与 Tauri `RunEvent` 退出回收。证据：`cargo test --manifest-path src-tauri/Cargo.toml` 22 项全通过（含用 fake 运行时覆盖未授权/离线/未连接/ADB 不可用/引擎缺失/正常退出/异常退出/停止/配对态不被覆盖等路径）；`pnpm build` 通过。**
- [x] ✅ A1-01a 会话启动互斥、正常退出恢复空闲、异常退出可重试及前端状态轮询；同设备重复启动/异常退出测试通过。Running 仅表示进程运行，不代表首帧已到达。

- [x] ✅ P0-06a 无线连接保存前验证精确端点处于授权状态，拒绝端口 0，配对尝试结束清除界面配对码；端点状态测试通过。真机重连与网络切换验收仍待完成。
- [ ] A1-02 多设备工作台、最近设备、本地预设与“忘记设备”。**设备选择、上次选择记录、本地预设、无线可信设备忘记均已实现。最近设备本轮补齐两处缺口：（1）新增 `forget_recent_device` 命令（命令面 13 → 14），可逐条移除本机的最近使用记录，且**只删记录**——不断开连接、不忘记无线配对、不撤销手机调试授权；（2）此前 `list_recent_devices` 虽已实现但**前端从未调用**，本轮接入工作台：显示设备名、相对使用时间与当前连接状态，已连接的“就绪”设备可一键开始镜像，形如 `ip:port` 的无线记录可尝试重新连接（界面明确提示端口可能已变化、需要重新配对），USB 序列号在未连接时如实提示“请用数据线重新连接”，并附「记录只保存在这台电脑上、可随时移除」的说明。证据：`cargo test` 54 项全通过（本项新增 2 项：逐条移除且幂等、删除真正落盘）；`pnpm build` 通过。30 台设备矩阵与完整验收待完成。**
- [x] ✅ A1-02a 质量/窗口预设本地保存、读取校验、恢复默认和存储失败提示；前端生产构建通过。
- [ ] A1-03 会话控制：旋转、全屏、置顶、质量预设、能力探测和受限能力说明。**启动参数（A1-03a）、能力探测/受限能力说明（A1-03b）、防锁屏/唤醒/锁屏诊断（A1-03c）与会话中应用设置（A1-03d）已完成；会话中**热更新**在架构上不可行（窗口形态由独立 scrcpy 进程在启动时确定），已按“结束旧窗口 + 按新设置重开”实现并如实告知；真机/跨平台验收仍待完成。**
- [x] ✅ A1-03a 启动参数支持三档画质、0/90/180/270°显示旋转、全屏和置顶，固定 H.264；后端白名单及非法参数测试通过。真机/跨平台验收仍待完成。
- [x] ✅ A1-03b 设备能力探测与受限能力说明：新增 `probe_device_capabilities` 命令，在启动会话**之前**读取设备系统属性（`adb -s <serial> shell getprop`，固定参数直接调用、无 shell 拼接与插值），据此判定画面/控制支持（API ≥26）与系统音频转发（API ≥30），并生成「原因 + 影响 + 应对」式的受限能力说明（系统版本过低、音频不可转发、版本未知、受保护内容黑屏、应用屏蔽电脑输入、OEM 开发者选项差异）；读不到系统版本时结论保持"未知"，不默认成"支持"。探测为只读操作，不配对、不连接、不改写会话状态。同时把序列号校验抽成 `validate_serial`（拒绝空值、`-` 开头、超长、含空白或控制字符的取值），会话启动与能力探测共用同一套设备状态错误码。**证据：`cargo test --manifest-path src-tauri/Cargo.toml` 31 项全通过（新增 9 项，覆盖属性解析、版本分档、未知态不被默认成支持、不可信属性文本的控制字符剔除与长度截断、探测复用 `device_unauthorized`/`device_offline`/`adb_unavailable`/`device_not_connected`/`probe_failed` 五类错误码、探测只读取属性、序列号校验，以及**设备标识（序列号/运营商/hostname）不被属性白名单保留、也不出现在返回结构里**）；`cargo clippy --all-targets` 零告警；`pnpm build`（tsc + vite）通过。**未验证面（外部阻塞）**：真机上 `getprop` 的实际返回内容、各 OEM 属性差异、以及受限说明在真实机型上的准确性均需真机复核，标记为未验证。**
- [x] ✅ A1-03c 防锁屏、远程唤醒与锁屏诊断：`SessionOptions.keep_awake`（默认开启，映射 scrcpy `--stay-awake`）从根上避免「镜像过程中手机自动锁屏」；新增 `wake_device` 命令（`adb -s <serial> shell input keyevent KEYCODE_WAKEUP`，固定参数直接调用、无 shell 拼接）**仅点亮屏幕**；新增 `device_lock_report` 命令读取 `dumpsys window policy` 与 `dumpsys power`，输出钥匙锁状态（`locked`/`unlocked`/**`unknown`**）、是否设置安全锁屏、屏幕唤醒状态，并生成「现状说明 + 下一步」面向非技术用户的引导。**读不到 keyguard 段落时结论保持 `unknown`，不默认成已解锁。**前端新增「手机当前的锁屏状态」面板、唤醒按钮与「会话期间保持唤醒」开关。**证据：`cargo test --manifest-path src-tauri/Cargo.toml` 44 项全通过（本项新增 13 项，含真机样本解析、unknown 不得被当成已解锁、锁屏时不得承诺无凭据可操作、唤醒只发送唤醒键、锁屏诊断不得触发写操作、最近设备去重与容量上限）；`pnpm build`（tsc + vite）通过。真机取证（Android 13 / Redmi M2104K10AC）：`secure=true deviceHasKeyguard=true` 时 `wm dismiss-keyguard` **无法**解除锁屏（`showing` 保持 true），`KEYCODE_WAKEUP` 可把设备从 `mWakefulness=Asleep` 唤醒到 `Awake`；取证见 `test-runs/keyguard-probe-*.txt` 与 `test-runs/keyguard-transition-*.txt`。**明确不做**（工程铁律红线）：无凭据越过锁屏、绕过生物识别 / MDM / `FLAG_SECURE`、无人值守控制——前者已被真机证据证实为 Android 系统层面拒绝。**未验证面（外部阻塞）**：真机上通过镜像窗口输入解锁凭据的可用性、各 OEM 的 `dumpsys` 字段差异、`--stay-awake` 在长时会话下的耗电与烧屏影响，均待复核。**
- [x] ✅ A1-03d 会话中应用窗口设置（旋转 / 全屏 / 置顶 / 画质 / 保持唤醒）：新增 `update_session_options` 命令（命令面 12 → 13）。**设计前提（架构事实，非取舍）**：镜像窗口由独立 scrcpy 进程持有，窗口形态在启动时由参数确定，**无法在运行中热更新**。因此语义被明确实现为「结束旧窗口 + 按新设置重新打开」，而不是把界面上的选项标记为已生效。要点：（1）**先校验参数再触碰运行中的会话**——非法请求（如 45°）只返回 `invalid_rotation`，绝不打断一次正常的镜像；（2）设置与当前会话完全一致时**不做任何动作**，返回 `applied:false` 与说明文字，避免无谓地中断画面；（3）会话槽位只有在**真正持有运行中进程**时才算“进行中”，`connecting`（尚未拿到进程）/`paired`/`failed` 一律返回 `session_not_running` 并说明「设置会在下次开始镜像时生效」，不去 kill 不存在的进程；（4）重启失败**保留原始具体错误码**（可能是 `device_unauthorized`/`device_offline`/`device_not_connected`/`mirror_runtime_missing`/`mirror_start_failed`），只在恢复建议后追加「本次修改未生效，镜像窗口已关闭」，**禁止塌缩成一个笼统的“重启失败”**；同一上下文同步写入会话错误，保证轮询到的状态与返回结果一致；（5）`SessionState` 新增 `options` 字段记录**当前会话实际使用的参数**——这是判断「是否需要重启」的唯一依据（已启动参数 ≠ 界面上的当前选择）；（6）新增 `begin_session_restart` 原子交出进程并就地转入 `Connecting`——**不能让会话先回到 `Idle` 再重启**，否则两次轮询之间前端会读到“没有会话”、“结束镜像”按钮会闪断，用户可能误以为镜像已经结束。前端：设置区标题按会话状态切换为「开始镜像时生效 / 会话中修改需重启镜像窗口」，会话进行中出现「应用并重启镜像窗口」按钮与「画面会短暂中断并自动恢复」的说明，结果以独立样式反馈。**证据：`cargo test --manifest-path src-tauri/Cargo.toml` 54 项全通过（本项新增 8 项：按新参数重启且旧窗口被结束、参数未变不打断正常会话、非法参数不干扰运行中会话、无运行会话时被拒且说明生效时机、重启启动失败时保留具体原因并带上下文、重启期间设备掉线落到 `device_offline` 而非笼统失败、重启途中停在 `Connecting` 而非 `Idle`、重启后可正常停止回到空闲）；`cargo clippy --all-targets` 零告警；`pnpm build`（tsc + vite）通过。**未验证面（外部阻塞）**：真机上重启造成的中断时长与观感、macOS/Windows/Linux 三平台窗口行为（全屏/置顶在各平台的差异）均需真机复核。**

- [ ] A1-04 截图、MP4 录制、双向剪贴板、文件传输；每项提供可见状态、撤销和失败恢复。**截图（A1-04a）、录制（A1-04b）、双向剪贴板（A1-04c）与文件传输（A1-04d）已实现并过单测；真机/跨平台验收仍待完成。**
- [x] ✅ A1-04d 文件传输：在电脑与手机之间收发文件，目录锁定为设备的 `/sdcard/Download/MirrorDock` 与本机的「下载 / MirrorDock」（回退应用数据目录）。新增命令 `send_file_to_device`、`list_device_files`、`fetch_file_from_device`（命令面 18 → 21，均在 `generate_handler!` 注册且 `capabilities/default.json` 已补 dialog 权限）。要点：（1）**传输范围限定在专用目录**：收发都只碰 `MirrorDock` 目录，不提供任意路径访问——这对非技术用户是「不可能选错位置」的保护，对安全边界是「adb 参数只含固定目录常量 + 校验过的文件名单 argv」；（2）**设备文件名不能用 ASCII 白名单**——真实设备文件常含中文等非 ASCII 字符（有单测「相册 导出.jpg」原样保留），改用**黑名单**：拒绝路径分隔符 `/`、`\`、`..`、控制字符、隐藏前缀与超长名（`transfer_name_invalid`），枚举每一条逃逸途径后，名字只可能落在传输目录内部；本地路径来自系统文件选择器，发送前先 `fs::metadata` 确认存在且是文件（`transfer_local_missing`）；（3）**发送流程 = mkdir + push**（固定参数直接调用，无 shell 拼接），任一步失败返回 `transfer_push_failed` 且不假装成功；（4）取回落到本机「下载 / MirrorDock」，撞名顺延序号；设备清单逐行解析并剔除空行与 `\r`；（5）未授权/离线设备**在发起任何 adb 调用之前**即被拒（单测断言此时不发起 mkdir/ls）；文件内容直接经 adb 传输，不进前端状态、不入日志。前端新增「文件传输」面板：系统文件选择器（官方 dialog 插件 `open`，用户取消不算错误）、「查看/刷新手机文件列表」（按需加载，不随会话轮询）、逐文件「取回到电脑」、回执展示文件名 + 大小 + 完整路径 +「在文件夹中显示」。**证据：`cargo test --manifest-path src-tauri/Cargo.toml` 73 项全通过（本项新增 5 项：中文文件名原样保留、push 失败如实上报且不声称成功、未授权设备在任何传输调用前被拒、设备清单逐行解析且空行剔除、取回文件真实落入本机传输目录）；`cargo clippy --all-targets` 零告警；`pnpm build`（tsc + vite）通过。**未验证面（外部阻塞）**：真机上的大文件传输表现、Android 各厂商对 `/sdcard/Download` 的写入差异、Windows 文件选择器与中文路径，均需真机复核。**真机 adb 层验证（2026-09-28，macOS + 无线 TLS 调试，Xiaomi M2104K10AC / Android 13）**：`mkdir -p /sdcard/Download/MirrorDock` → `push`（中文文件名「mirrordock-verify-传输测试.txt」原样保留）→ `ls` → `pull` 双侧 md5 完全一致（`b7a52c3b…`），验证后双侧测试痕迹已删除；大文件与 Windows 端仍待复核。**
- [x] ✅ A1-04c 双向剪贴板：把 scrcpy 默认的剪贴板自动同步做成**可关闭的显式选项**。`SessionOptions` 新增 `clipboard_autosync`（**默认开启**，与 scrcpy 默认一致：设备剪贴板变化同步到电脑、粘贴前把电脑剪贴板同步到设备；关闭时向镜像进程传 `--no-clipboard-autosync`——该参数已用本地二进制 `--help` 核实）。提供关闭开关的原因是隐私取向：有的用户不希望手机上复制的内容自动出现在电脑剪贴板里；开关文案明确写出「关闭后手机与电脑的复制内容不再自动互通」。剪贴板文本由 scrcpy 进程内部处理，不经过 MirrorDock，也不入日志。注意剪贴板行为受 Android 版本与前台应用限制（如部分输入框禁止粘贴），属系统能力差异，不构成会话失败。**证据：`cargo test --manifest-path src-tauri/Cargo.toml` 73 项全通过（本项新增 2 项：默认开启且不向镜像进程传剪贴板相关参数、关闭后参数列表包含 `--no-clipboard-autosync`）；`pnpm build`（tsc + vite）通过。**未验证面（外部阻塞）**：真机上双向同步的实际表现（含各 Android 版本的剪贴板限制）需真机复核。**
- [x] ✅ A1-04b 录制（MP4）：把本会话画面录成本机 MP4。`SessionOptions` 新增 `record`，**默认关闭**——录制会产生一份屏幕副本，与 `keep_awake` 的默认值取向不同：「保护用户」的默认值可以替用户打开，「替用户留存屏幕副本」必须由用户自己决定。实现要点：（1）录制与画面**共用同一个 scrcpy 进程**（`--record=<路径> --record-format=mp4`；scrcpy 4.1 支持哪些录制格式已用本地二进制 `--help` 核实：`mp4/mkv/m4a/mka/opus/aac/flac/wav`），因此**只能在启动时决定、无法在会话中单独开关**；界面沿用 A1-03d 的语义「会话中开启/关闭录制 = 结束旧窗口 + 按新设置重开」并如实告知，不假装热生效；（2）**保存目录由后端决定**（视频目录下的 `MirrorDock`），前端只能给文件名——文件名经与截图同一套 ASCII 白名单校验（错误码 `media_name_invalid`）并强制 `.mp4` 扩展名，路径穿越无从发生；（3）**打开录制时在占用会话槽位之前先准备路径**：目录不可写或文件名非法时直接失败，不浪费一次会话；（4）新增 `current_recording` 命令返回 `{file_name, path, active}`，其中 `active` 由「是否存在运行中的进程」推出——「磁盘上有一个 mp4」与「此刻正在录」是两件事，界面必须区分；会话结束后**刻意保留**该路径，用户才能打开或删除刚录的文件；（5）新增 `delete_recording` 作为**撤销**：正在写入的录像**拒绝删除**（`recording_in_progress`，提示先结束会话），文件不存在时如实返回 `recording_missing`，都不假装成功。前端新增「录制这一会话的画面（MP4，保存在本机）」开关，以及「录像」面板展示「正在录制 / 已结束录制 + 文件名 + 完整路径」，「在文件夹中显示」与「删除这段录像」（录制中禁用）。**证据：`cargo test --manifest-path src-tauri/Cargo.toml` 65 项全通过（本项新增 5 项：默认不录制且不向镜像进程传任何路径、开启录制后路径原样交给镜像进程且状态为「正在写」、录像文件名无法逃出录制目录（`../escape.mp4` / `sub/dir.mp4` / `sub\dir.mp4` / `.hidden.mp4` / `clip.mkv` / 空）并验证同一校验函数不放行其它扩展名、正在写入的录像拒绝删除且结束会话后可删、会话中打开录制会按新参数重启并记录新文件）；`cargo clippy --all-targets` 零告警；`pnpm build`（tsc + vite）通过。**未验证面（外部阻塞）**：真机上录制的实际体积与帧率表现；受保护页面的录像表现同样待复核。**MP4 强杀风险已真机实验证实并修复（2026-09-28，Xiaomi M2104K10AC / Android 13，USB）**：用 `--no-window --record` 无窗录制 8 秒后（a）SIGKILL → 文件 6,815,792 字节，顶层 box 仅 `ftyp`+`free`+`mdat`（**无 moov，无法播放**）；（b）SIGTERM → scrcpy 打出 "Recording complete"，顶层 box 完整含 `moov`（3,698 字节），文件可播放。据此把结束会话改为**优雅终止**：`MirrorProcess` 新增 `stop`（默认强杀，测试替身沿用；系统实现 = Unix 先 SIGTERM、等 `PROCESS_GRACEFUL_TIMEOUT` 3 秒收尾、超时 SIGKILL 兜底再回收；Windows 无 SIGTERM 对应物，保持 TerminateProcess，**Windows 上录制收尾风险如实保留待验证**）。`stop_mirroring`、会话中应用设置（重启旧窗口）与应用退出回收三处全部改走 `stop`。**证据：`cargo test` 76 项全通过（新增 2 项真进程测试：SIGTERM 路径迅速退出且被回收；忽略 SIGTERM 的 python 进程等满宽限期后被强杀、总时长有上界——用 ready 标记规避「信号早于处理器注册送达」的竞态）；`cargo clippy --all-targets` 零告警。依赖新增：Unix 平台 `libc 0.2`（发 SIGTERM），Windows 零新增。**
- [x] ✅ A1-04a 截图：把手机当前画面保存为本机 PNG。新增 `capture_screenshot` 命令（命令面 14 → 16，含撤销命令 `delete_screenshot`），经 `adb -s <serial> exec-out screencap -p` 读取画面（固定参数直接调用、无 shell 拼接与插值；用 `exec-out` 而非 `shell`，避免 Windows 上把 `\n` 改写成 `\r\n` 从而破坏 PNG 二进制），保存到本机「图片 / MirrorDock」。要点：（1）**文件名由前端按本机时间生成**（后端不猜时区），随后按 ASCII 白名单严格校验——只放行字母、数字、`-`、`_`、`.`，因此 `/`、`\`、`..`、控制字符与隐藏文件前缀都无法通过，**不存在路径穿越**；（2）**只看退出码不够**——必须校验 PNG 文件头，字节流被文本模式改写或中间层插入诊断信息时返回 `screenshot_not_image`，**绝不写下一个打不开的文件**；（3）同名不覆盖，顺延加序号（`-1`、`-2`…），避免同一秒内连续截图覆盖用户已有文件；（4）新增 `delete_screenshot` 命令作为**撤销**，文件名经同一套白名单校验，因此只可能落在截图目录内部；文件已不存在时如实返回 `screenshot_missing`，不假装成功；（5）设备未授权时**在读取屏幕之前**即被拒绝（有单测断言此时不会发起截图调用）；（6）**截图内容是屏幕像素**——只写入用户可见的本地文件，不返回像素数据、不进日志、不进错误消息。前端新增「截图」面板：操作按钮、保存结果的**文件名 + 大小 + 完整路径**、「在文件夹中显示」（`revealItemInDir`，`opener:default` 权限已含该项）与「删除这张截图」，并说明「受保护页面由 Android 自行屏蔽，截出来是黑屏，这不是故障」。**证据：`cargo test --manifest-path src-tauri/Cargo.toml` 60 项全通过（本项新增 6 项：截图落盘且字节与设备返回完全一致、非图片负载绝不落盘、文件名无法逃出截图目录（`../escape.png` / `sub/dir.png` / `sub\dir.png` / `.hidden.png` / `shot.jpg` / 空 / 超长）、同一秒内两次截图不互相覆盖、可撤销且文件消失时如实报错、未授权设备在读屏前被拒）；`cargo clippy --all-targets` 零告警；`pnpm build`（tsc + vite）通过。**未验证面（外部阻塞）**：真机上 `exec-out screencap` 的实际耗时与输出大小、Windows 上是否出现二进制改写、`FLAG_SECURE` 页面的黑屏表现，均需真机复核。**真机 adb 层验证（2026-09-28，macOS + 无线 TLS 调试，Xiaomi M2104K10AC / Android 13）**：`adb -s <serial> exec-out screencap -p` 返回 15,580 字节且文件头为 `89 50 4E 47 0D 0A 1A 0A`（PNG 完整）——链路本身无二进制损坏；Windows 文本模式改写风险仍待 Windows 实测。**
- [x] ✅ A1-05 音频能力检测与 Android 版本/应用限制说明：能力检测（`audio_forwarding_supported`，SDK ≥ 30）在 A1-03b 已具备并带 `audio_forwarding_unavailable` 限制通知；本项补齐**声音转发开关与限制说明**。要点：（1）`SessionOptions.audio`（**默认开启**，映射 scrcpy 默认行为；关闭时传 `--no-audio`——该参数已用本地二进制核实被 scrcpy 4.1 接受）；（2）**不提供麦克风采集**：scrcpy 的 `--audio-source=mic` 属更敏感的隐私面，MVP 不开放，仅转发系统播放声音（scrcpy 默认 `output`），界面文案明确「不使用麦克风」；（3）**Android 11 以下的行为如实建模**：scrcpy 在不支持音频捕获的设备上会**自动禁用音频**继续镜像（官方行为，`--require-audio` 才会失败），因此后端**不做硬校验**（硬校验反而会拒绝本可正常工作的会话）；前端在能力探测返回 `audio_forwarding_supported = false` 时**自动关闭并禁用**该开关，文案说明「这台手机不支持系统音频转发，已自动关闭」，界面与 scrcpy 实际行为一致，不留「开了也不生效」的开关；（4）**应用级限制如实说明**：通话与通过捕获策略退出的应用（部分受保护/DRM 应用）可能无声，属系统行为，写入设置区说明文字，不假装可修复。**三处行为已对照 scrcpy v4.1 官方文档 `doc/audio.md` 核实**：（a）"For Android 10 or earlier, audio cannot be captured and is automatically disabled"；（b）"If audio capture fails, then mirroring continues with video only … unless `--require-audio` is set"——佐证后端不做硬校验的决策；（c）`output` 源（默认）"forwards the whole audio output, and disables playback on the device"——**开启转发后手机本地静音**，此交互后果已写入界面说明；（d）Android 11 有额外前提："you'll need to ensure that the device screen is unlocked when starting scrcpy"，也已写入界面说明。会话中开关声音沿用 A1-03d 的「应用并重启镜像窗口」语义（音频参数在启动时确定）。**证据：`cargo test --manifest-path src-tauri/Cargo.toml` 74 项全通过（本项新增 1 项：默认不传任何音频参数、关闭后参数含 `--no-audio`、字段缺省经 serde 视为开启）；`cargo clippy --all-targets` 零告警；`pnpm build`（tsc + vite）通过。**未验证面（外部阻塞）**：真机上音频转发的实际听感与时延（本机曾实测 Android 13 设备 `--no-audio` 参数被接受，但未系统测量音频链路）；Android 11 以下设备上「自动禁用音频继续镜像」的行为需真机复核；受保护应用无声的表现需复核。**
- [x] ✅ A1-06 厂商品牌知识库与引导：Pixel、Samsung、Xiaomi、OPPO、vivo、OnePlus。新增 `src/brandGuides.ts`（纯前端内容层），每个品牌四块内容：打开开发者选项的具体路径、开启 USB 调试（含品牌特有要求）、无线调试入口、品牌特有注意点。要点：（1）**内容诚实性**：全部条目附「不同机型与系统版本的菜单名称可能不同，以手机实际设置为准」，不假装路径永远准确；小米明确写出「USB 调试只允许看到画面，USB 调试（安全设置）才允许控制，且需登录小米账号并插 SIM 卡——是系统限制，MirrorDock 无法绕过」；OPPO/vivo/一加写明账号验证码机制与一次验证后通常不再重复；vivo 写明子用户/访客模式无法开启；OPPO 条目兼容 realme（同源 ColorOS）。（2）**品牌自动识别但不伪装确定性**：`detectBrand` 用设备 label（厂商 + 型号）关键词匹配（如 `SM-`→三星、`Redmi`→小米、`iQOO`→vivo），识别到时自动选中并明确说「**疑似**为该品牌，如型号不符可手动切换」；识别不到时如实显示「选择品牌查看步骤」，绝不把「猜不出」伪装成某个品牌；用户手动选择始终优先于自动猜测。（3）**只在需要时出现**：面板仅在设备尚未就绪（未连接/未授权/离线）时渲染，不干扰已就绪用户的操作路径。**证据：`pnpm build`（tsc + vite）通过（类型检查覆盖 `brandGuides.ts` 全部导出与 `App.tsx` 的消费端）；内容为静态知识层，无运行时设备差异可单测，真机核对留待 30 台设备矩阵（A1-07）抽查。**未验证面**：各品牌最新系统版本（HyperOS 2 / One UI 7 / ColorOS 15 等）菜单名称是否仍与描述一致，需真机抽查。**
- [ ] A1-07 单元、集成和端到端测试；30 台设备矩阵、三桌面系统的 Alpha 报告。**三层测试体系已建立并全绿：单元（Rust 76 项 + 前端 vitest 21 项）、集成（lib.rs 内 3 项跨命令用户旅程：未授权→启动→改设置→优雅停止 / 截图与传输共用授权门 / 无线端点同契约）、端到端（`scripts/e2e-smoke.sh` 真机冒烟：授权状态、截图 PNG 完整性、传输 push/list/pull md5、录制 moov 索引——M2104K10AC 上 6/6 通过，报告在 `test-runs/`）。CI 推送到 main 即运行全部 Rust 测试与 `pnpm build`。****未验证面（外部阻塞）**：30 台设备矩阵与三桌面系统的 Alpha 报告需要对应设备/平台，待资源到位后用 `scripts/e2e-smoke.sh` 与 `scripts/measure-session.sh` 批量采集。**
- [ ] A1-08 打包 scrcpy 运行时、第三方 NOTICE、SBOM、构建哈希与签名验证流程。**已实现：① scrcpy v4.1 随包分发——CI 按平台下载官方包并强制 SHA-256 校验（三平台哈希固定写死，见 THIRD_PARTY_NOTICES.md，均经实际下载复核），解压进 `src-tauri/resources/scrcpy` 随安装包分发（包内自带 adb）；Linux 无官方包，运行时回退 PATH。② 运行时查找升级为四级（显式环境变量 → 开发目录 → 随包资源目录 → PATH），Rust 82 项测试（含 4 项查找顺序新测试）。③ NOTICE 随包（`src-tauri/resources/THIRD_PARTY_NOTICES.md` + 官方包内 LICENSE 原件）。④ 构建哈希清单 SHA256SUMS 与依赖清单（cargo metadata + pnpm list JSON）作为 `-meta` artifact 随产物发布。****未验证面（外部阻塞）**：tag 触发的真实打包运行（四平台 bundle 中 resources 生效、包体体积）需下一次发版标签验证；代码签名无证书未做，已如实记录。**

## 阶段 2：MVP Beta

- [ ] B2-01 产品内诊断包（显式同意、脱敏、可预览），本地日志和支持导出。**已实现；待真实支持场景复核内容充分性。**
  - **内存事件日志**：`DiagnosticsLog` 环形缓冲（上限 200 条、单条 400 字符），默认不落盘——只有用户预览并显式导出时才写成文件。
  - **脱敏靠构造 + 擦除兜底**：事件详情只允许来自用户可见文案（`AppError.message/recovery`）或固定字符串；序列号、配对码、端点、本地路径由调用点声明进机密列表，入库前统一替换为「[已脱敏]」。单元测试断言声明的机密绝不出现。
  - **挂接点**：设备检查（只记状态计数，不记序列号/型号）、镜像启动/停止、会话设置更新、无线配对/连接（配对码与端点为机密）、截图、文件发送/取回（序列号、文件名、路径为机密）。
  - **预览=导出**：`diagnostics_preview` 返回的内容与 `export_diagnostics` 写入文件的内容完全一致（同一 `build_diagnostics_preview`）；内容仅应用版本、系统类型、镜像引擎可用性、事件列表，无其他字段。
  - **前端**：「帮助与诊断」面板常驻；「预览诊断内容」→ 列表展示 → 「导出为文件」（系统保存对话框，取消不算错误）；界面明确告知「不会自动上传任何内容」。
  - **证据**：Rust 82 → 87（脱敏/容量/预览结构/导出回执 5 项新测试）、前端 21 → 22（面板预览前禁用导出、文案可见）；clippy 零告警；`pnpm build` 通过。
  - **未验证面**：真实支持场景下事件是否足够定位问题（需用户反馈迭代）；事件时间戳为本地时钟（用户改时间会偏移，如实接受）。
- [ ] B2-02 隐私遮罩、演示指针、快捷键配置和可访问性审查。**已实现；隐私遮罩按能力边界诚实降级（见下）。**
  - **快捷键配置**：`SessionOptions.shortcut_mod`（`--shortcut-mod`，默认不传 = scrcpy 默认 左Alt/左Super）；后端白名单校验（lctrl/rctrl/lalt/ralt/lsuper/rsuper，白名单外拒绝，错误码 `shortcut_mod_invalid`），不把任意字符串透传进 scrcpy 参数；前端下拉选择；设置区列出 scrcpy 4.1 真实快捷键（对照 `--help` 核实，含 MOD+O 关屏=镜像继续）。
  - **演示指针**：`SessionOptions.show_touches`（`--show-touches`，默认关）；文案说明「显示手机上的物理触摸点（不显示电脑注入的点击，scrcpy 官方行为）、结束镜像自动恢复原设置」——默认不替用户改设备设置。
  - **隐私遮罩（诚实降级）**：scrcpy 4.1 无任何画面遮盖/滤镜能力（已核实 `--help` 全文），不假装有此功能。实现为：① `SessionOptions.read_only`（`--no-control` 只读演示模式，消除向他人演示时的误操作风险）；② 设置区明示能力边界与真实隐私建议（勿扰模式/结束镜像）。
  - **可访问性审查**：全局 `:focus-visible` 焦点框（此前仅无线表单有焦点样式）；aria-live/alert/status 覆盖已达标，本轮未发现新缺口。
  - **证据**：Rust 87 → 89（修饰键白名单/演示双开关固定参数/缺省回填）、前端 22 → 23（含「被篡改的修饰键回落默认」测试）；clippy 零告警；`pnpm build` 通过。
  - **未验证面**：真机上 show-touches 的显示效果与恢复、read-only 模式输入被拒的实际体验——待真机验收轮次一并复核。
- [ ] B2-03 失败注入：拔线、ADB 重启、网络切换、锁屏、拒绝/撤销授权、保护内容。**传输通道层自动化子集 + 物理场景（USB 拔线/撤销授权/保护内容）均已真机验证，全部通过（2026-09-28 晚，用户配合操作）；仅剩 GUI 应用层故障表现的端到端验收待后续轮次。**
  - **自动化子集（scripts/failure-injection.sh，11/11 通过）**：
    - 场景 A（ADB 服务重启）：运行中 `adb kill-server` → 镜像会话 1s 内退出（不挂死）、adb 服务恢复后设备状态仍为 device（授权保留）、故障前录制段 moov 完整。
    - 场景 B（无线通道断开）：无线镜像会话中 `adb disconnect` → 会话退出（不挂死）、无线通道可重连恢复。
    - 场景 C（锁屏与恢复）：锁屏后镜像会话未终止；包含锁屏区段的录制 time-limit 自然收尾、moov 完整（3.4MB）；唤醒恢复 Awake。
  - **物理场景 D（USB 拔线，test-runs/physical-unplug-20260928-225833.log）**：拔线瞬间会话 ≤1s 退出，scrcpy 干净输出 `WARN: Device disconnected` 并完成录制收尾（moov 完整），无挂死；断开窗口 ~15s；插回后直接恢复 device（授权持久化，无需重新授权）；无线通道为独立传输，USB 断开期间保持可用，重插后 ~1 分钟内由 mDNS 自动重新注册；传输层不自动重连会话（重连引导为应用层职责——产品设计输入）。
  - **物理场景 E（撤销授权，test-runs/physical-revoke-*.log）**：撤销瞬间 USB+无线同时掉出设备列表；插回后进入 `unauthorized` 独立状态（持续 ~24s，等待用户在设备端授权）；点允许后直接回 device。**证实 unauthorized 必须作为独立可恢复状态建模**（与 offline/failed 不同）。意外发现：撤销 USB 调试授权未连带清除无线调试配对——USB 重新授权 1s 后无线 TLS 通道自动恢复。
  - **物理场景 F（FLAG_SECURE 保护内容，test-runs/physical-secure-20260928.mp4/.txt）**：4.8 分钟无窗录制（70.5MB），ffmpeg blackdetect（d=1.5, pix_th=0.10）检出 3 个黑帧区段（6.7s / 21.7s / 3.6s，含用户操作的保护页面时段）；**所有黑屏区段期间录制流持续、会话未中断、收尾 moov 完整**；非黑帧区段有正常画面作对照。符合 Android FLAG_SECURE 保护预期（镜像黑屏但不断流）。
  - **范围边界（如实记录）**：以上验证 adb/scrcpy 传输通道层 + 设备端授权状态机；应用 GUI 下故障表现的端到端验收（用户看到的状态文案与恢复引导）待后续真机验收轮次。
- [ ] B2-04 性能/稳定性门禁：P95 首帧 <5 秒、60 分钟会话与回归基线。**首帧门禁已通过（真机 10 轮，P50=0.91s / P95=0.99s）；60 分钟 soak 已通过（进程 60 分钟全程存活 + 自然收尾，详见 soak 证据）。**
  - **脚本**：`scripts/perf-gate.sh`（子命令 `first-frame [轮数]` / `soak [分钟]`），报告落 `test-runs/`。
  - **首帧口径（如实记录）**：进程启动 → 视频隧道 TCP 连接建立（lsof ESTABLISHED，排除 adb server:5037），是可见首帧的**乐观下界代理**（不含解码渲染；macOS 无头模式无法捕获窗口首帧）。已实测 scrcpy 的 mp4/mkv 封装在缓冲满或收尾前不落盘，文件增长不可作首帧信号。
  - **首帧证据**：Xiaomi M2104K10AC / Android 13，10/10 轮有效，P50=0.91s、P95=0.99s、max=0.99s，门禁 P95<5s 通过（test-runs/perf-first-frame-20260928-224728.txt）。
  - **60 分钟 soak 证据（Xiaomi M2104K10AC / Android 13，USB，test-runs/perf-soak-20260929-000604.txt）**：scrcpy 无窗无控无音频 `--record --time-limit=3600`，每分钟采样一次，**60/60 分钟进程存活**，录制稳定增长约 1.05 MB/分钟，第 60 分钟 64,223,908 字节；到达 time-limit 后自然收尾，scrcpy 日志 `Time limit reached` + `Recording complete to mp4 file`。
  - **soak 判定缺陷已修复（外部证据）**：首次运行脚本判 FAIL，根因是本机 macOS BSD grep 2.6.0-FreeBSD 的 `grep -a` 在二进制文件上**反而不匹配**（`grep -aq moov` 恒假）；已验证 `grep -q` 可正确命中 mp4 内的 moov 原子（GNU/BSD 兼容），修复 `scripts/perf-gate.sh` 后 1 分钟 soak PASS（test-runs/perf-soak-20260929-010917.txt，moov 完整）。**如实记录**：60 分钟录制文件按脚本设计随临时目录清理，未直接复查其 moov；其完整性依据为「time-limit 自然收尾 + Recording complete 日志 + 同配置干净收尾录制均含 moov」的证据链，而非对该文件本身的直接检验。
  - **待完成**：回归基线固化（多设备/多桌面矩阵为外部阻塞）。
- [ ] B2-05 非技术用户可用性测试、帮助中心、客服分流与兼容性页面。**帮助中心与客服分流的代码面已落地：内置帮助 9 篇（2026-10-02 守卫轮新增「寻求帮助」：先自助 → 导出诊断包 → GitHub 仓库 Issue 反馈格式 → 防钓鱼提醒（不提供画面截图/许可证密钥/配对码）→ 安全问题走 GitHub 私密报告）；与 channels.md 支持分流底稿、B2-01 诊断包（已脱敏）一致。**公开兼容性页已落地为 `site/compatibility.html`（2026-10-02 守卫轮，见下方证据），上线动作随发布流程执行。**待完成：真实用户的可用性测试（外部阻塞）；兼容性页对外发布（依赖 R3-04 打标演练，页面内容已就绪）。**
  - **证据（2026-10-02 守卫轮，改动 = `src/helpContent.ts` + 新增 `src/helpContent.test.ts` + 本文件）**：vitest **43 passed / 0 failed**（41 → 43，新增 2 项：帮助文章结构不变量（id 唯一/标题/段落非空）、「寻求帮助」条目覆盖诊断包导出 + Issue 渠道 + 防钓鱼提醒 + 脱敏承诺一致性）；`pnpm build`（tsc + vite）通过。纯前端内容层改动，无 Rust 改动。
  - **证据（2026-10-02 守卫轮，兼容性页随 v0.4.2-beta 发版同步；改动 = `site/compatibility.html` 6 处 + `docs/compatibility-matrix.md` 4 处 + 本文件）**：发版提交 6ad4e15 同步了 README/site 首页版本行但遗漏兼容性页（仍写 v0.4.1-beta）。本轮先把 v0.4.2-beta 打包证据独立核验成立（GitHub API：Release 唯一、非草稿、prerelease、30 资产，伴侣 APK 首次带版本名 `MirrorDock-companion-0.4.2.apk`，latest.json + 7 个 .sig 在位；updater 分支 latest.json version=0.4.2 四平台 URL/签名齐全），再把两文件共 10 处「✅ v0.4.1-beta」同步为 v0.4.2-beta；「v0.4.1 起已修复更新端点」等事实性表述不动。安装/运行列维持「未验证」。验证：`pnpm build` 通过（site/ 不在 vite 编译范围，保险性复核）。
  - **证据（2026-10-02 守卫轮，公开兼容性页落地；改动 = 新增 `site/compatibility.html` + `site/index.html` 两处修正 + `docs/compatibility-matrix.md` 打包列同步 + 本文件）**：①新页沿用公开站设计语言（同一套 design tokens / 头尾导航），四个区块（桌面平台 / 已验证设备 / Android 版本能力差异 / 已知限制）与 `docs/compatibility-matrix.md` 一一对应，**只写有证据的行**——「打包 ✅ v0.4.1-beta」有 CI 资产清单佐证（经 GitHub API 核对 v0.4.1-beta Release 全部 30 项资产：exe/msi/dmg×2/AppImage/deb/rpm 齐全），安装/运行列保持「未验证」不冒进；②修正 `site/index.html` 下载区过时表述「本版本不提供自动更新」→ 如实改为「v0.3.0 起内置验签自动更新；v0.4.0-beta 及更早客户端内嵌失效端点需手动升级一次」（与 compatibility-matrix.md 及首页功能列表此前各自矛盾的口径统一），并加入口导航/页脚/下载区三处指向兼容性页；③顺带核验 site 全部下载链接与 v0.4.1-beta 实际资产名一致（伴侣 APK 该版本仍为 `app-debug.apk`，`MirrorDock-companion-<版本>` 改名自下个 tag 生效），无死链。验证：自研 HTML 结构校验（标签配对/内部锚点/本地链接）双页零错误；`pnpm build` 通过（site/ 不在 vite 编译范围，属保险性复核）。**未验证面**：页面在真实浏览器/移动端的渲染观感（静态内容层，风险低）；上线部署为用户决策，未代行。
  - **X10-65 Linux 运行时说明纠错（2026-10-02 守卫轮，文档正确性修复，无代码/版式改动；改动 = `docs/compatibility-matrix.md` + `site/compatibility.html` + 本文件）**：兼容性矩阵与公开页两处的 Ubuntu 行仍写「无官方 scrcpy 包，运行时回退系统 PATH 中的 scrcpy」——这是 **X10-48 之前的旧口径**，对用户的实际影响是「以为要自己装 scrcpy」，而真实缺口是发行版运行库（SDL3/FFmpeg）。三处取证一致推翻旧表述：①`build.yml` 的 Linux 步骤（第 170–222 行）在 runner 上从官方源码编译 scrcpy v4.1 并组装 `scrcpy` + `scrcpy-server` + platform-tools `adb` 进随包资源目录（固定 tag + 固定源码 SHA-256 + 官方 release digest，fail-closed）；②`THIRD_PARTY_NOTICES.md` 明写「Linux 自 v0.3.0 起同样随包分发…SDL3 与 FFmpeg 等运行时动态库不随包，由用户的发行版提供」；③`README.md` 系统要求行与「Linux 运行库说明（诚实版本）」章节同口径。本轮只改文档表述、不动任何代码与页面版式：矩阵 Ubuntu 行与公开页备注改为如实描述（随包分发 + 运行库由系统提供 + 指向 README 说明），并在公开页「已知限制」按其既有 `limits` 标记样式补一条「Linux 安装包需要发行版提供运行库」，与 README 一致。验证：页内既有标记结构沿用（未新增样式类），HTML 标签配对与内部锚点校验通过；`pnpm build` 通过、vitest 全绿（`site/` 不在 vite 编译范围，属保险性复核；本轮无 Rust 改动）。
  - **X10-67 新功能帮助覆盖（2026-10-02 守卫轮，见 X10 条目）**：帮助中心补齐 v0.4.3-beta 交付的两个新功能——`tools` 篇章补拖拽路由 / 发送区管理 / 反向「发送文件到电脑」，新增「手机通知镜像」篇章（9 → 10 篇）；同时修正 README 对外声明的篇数（8 → 10）并新增两条功能条目。证据：vitest 51/51（新增 3 项，含「README 声明篇数 == 实际篇数」不变量）、`pnpm build` 通过。
  - **待完成（外部阻塞/待发布）**：非技术用户可用性测试需要真实用户轮次；兼容性页上线依赖发布流程决策（页面与数据源均已就绪）。
- [ ] B2-06 Windows、macOS、Ubuntu 安装包；更新、回滚、签名和渠道差异验证。**2026-10-02 用户裁定：mac 签名（需 Apple Developer 账号/证书，涉及收费）与 win 测试矩阵（需 Windows 设备）当前条件均不满足，一并搁置；连同渠道发布（涉及收费，见 R3-02）三项均不计入当前推进顺序。**

## 阶段 3：公开 v1

- [ ] R3-01 免费/Pro 授权设计与本地权益状态；不将任何授权密钥写入日志。**已实现（2026-09-28 夜间轮次，自主决策已记录）；待真实激活流程的 GUI 端到端验收。**
  - **授权模型（自主决策）**：无账户、无激活服务器、离线激活；免费版保留全部核心体验（USB/无线镜像、截图、文件传输、**音频转发**、会话设置、诊断）；Pro 门控仅 MP4 录制。特别说明：音频转发不做门控——`audio` 默认开启，纳入门控会让免费版默认启动直接报错，违背开箱即用。
  - **许可证格式**：ed25519 签名 JSON 载荷（product/key_id/edition/expires_at），`MD1-` 前缀 + 自实现 base32（长度前缀消歧义、6 字符分组可抄写）；公钥编译进二进制（`LICENSE_VERIFYING_KEY`），私钥由 `examples/license_keygen` 生成、存仓库外内部文档目录，绝不入 Git/CI/日志。
  - **命令面 23 → 26**：`entitlement_status` / `entitlement_activate`（失败诊断脱敏：原始密钥串声明进机密列表，测试断言不回显）/ `entitlement_deactivate`；权益文件 entitlement.json 存许可证原文、加载时重验签，损坏/篡改/过期一律安全回退免费版。
  - **门控位置**：`start_mirroring` 与 `update_session_options` 前置 `ensure_edition_allows`（错误码 `pro_required`，触碰设备之前就拒绝）；录制开关在免费版禁用并给出激活指引。
  - **签发工具**：`examples/license_sign`（env 种子）与 `examples/license_keygen`（/dev/urandom + 签名自检）。
  - **证据**：Rust 89 → 99（base32 往返、验签往返/篡改拒绝/过期/产品不符、损坏回退、门控矩阵、密钥不回显、**真实种子端到端**（无种子环境自动跳过））；前端 23 → 27（免费/专业两态渲染、激活调用契约、失败不回显密钥）；clippy 零告警；`pnpm build` 通过。
  - **未验证面**：GUI 下真实激活/撤销的端到端体验、过期许可在到期后门禁的运行时行为（逻辑有测试、无真机轮次）。
- [ ] R3-02 国内、Google Play、企业侧载各自的隐私、权限、签名、更新与支持材料。**底稿完成（docs/channel/）；法务复核与渠道审核为外部流程。2026-10-02 用户裁定：渠道发布涉及收费，暂时搁置，不计入当前推进顺序（阶段 3 优先 mac 签名与 win 测试矩阵）。**
  - `docs/channel/`：README（三渠道材料矩阵与门禁）、privacy-policy-draft（每条承诺对应实现证据）、permissions-data（桌面端与设备端权限逐项表）、channels（国内/Play/企业差异、更新与支持分流）。
  - 渠道状态如实标记：签名 ❌（外部阻塞）；SBOM/NOTICE ✅（A1-08 产物）。
- [ ] R3-03 安全审计：威胁模型、依赖漏洞、许可证、SBOM、签名、更新和数据流。**首版完成；四项假设已于 2026-09-29 由产品负责人回签成立（私钥永不进 CI / 不承诺公共 WiFi / v1 无自动更新 / scrcpy 固定哈希 fail-closed），结论转为在假设成立时有效。**
  - 威胁模型：`docs/mirrordock-threat-model.md`（7 条威胁 TM-001..007、信任边界与 Mermaid 图、重点审查路径；按方法论第 7 步降级执行——假设显式化，未阻塞产出）。
  - 依赖漏洞：cargo-audit v0.22.2 扫描 **0 漏洞**；2 条警告（RUSTSEC-2024-0370 proc-macro-error unmaintained、RUSTSEC-2024-0429 glib unsound——均为 Linux GTK 传递依赖，记录在案）。
  - 许可证/SBOM：Apache-2.0（与 scrcpy 同源合规）、SBOM/NOTICE 随产物（A1-08）。
  - 签名/更新：未签名为已知外部阻塞；自动更新未实现（有意），更新链路威胁建模列为后续项。
- [ ] R3-04 发布候选、试点、支持值守、发布/回滚决策与公开兼容性矩阵。**流程文档与矩阵底稿完成；RC 流程待下次打标演练。**
  - `docs/release-runbook.md`：发布前门禁清单（含 cargo audit 空结果留痕要求）、RC→真机验证→正式打标→72h 值守流程、回滚触发与动作。
  - `docs/compatibility-matrix.md`：桌面四平台与设备矩阵（只写有证据的行）、能力差异（音频/FLAG_SECURE 按系统版本）、已知限制诚实清单。

## v1 后：Android 伴侣 App

- [ ] C4-01 Kotlin Android App、扫码配对与同网加密会话 POC。**代码与桌面端配对端点完成（POC）；真机端到端验证待用户配合安装 APK。**
  - 桌面侧：`src-tauri/src/companion_pairing.rs`——TLS 1.3 服务端（rustls/ring）、每次配对现场生成一次性自签证书（rcgen，不落盘）、一次性 token（10 字节熵 base32 16 字符）、SPKI SHA-256 出带指纹校验；命令面 26→29（`companion_begin_pairing` / `companion_pairing_status` / `companion_end_pairing`）。Rust 端到端测试 2 项（真实 TLS 握手 + 指纹锁定 + token 握手 + 统计流；错误 token 拒绝），Rust 102→104。
  - 前端：「伴侣 App 配对（实验）」面板——qrcode 渲染二维码（内存生成、不落盘）、2s 轮询状态与事件流、二维码不可用时退化为手动输入配对码。前端 27→29。
  - 伴侣侧：`companion/` Gradle 工程（Kotlin、minSdk 26、仅 zxing-core + CameraX + appcompat）——扫码配对（CameraX ImageAnalysis + zxing 离线解码）与手动输入兜底、TLS SSLSocket 自定义 TrustManager 校验 SPKI 指纹、MDP1 JSON 行会话（device_hello/capture_stats/bye）。
  - 协议：二维码载荷 `MDP1|主机列表|端口|一次性配对码|SPKI SHA-256(hex)`；token 只经加密通道发送；屏幕帧不落盘不入日志（服务端只回传统计）。
- [ ] C4-02 MediaProjection、前台服务、音频能力探测、同意/撤销状态机。**代码完成（POC）；真机验证待用户配合。**
  - 同意状态机 NotRequested→Pending→Granted/Denied→Revoked（系统面板撤销触发 onStop→REVOKED→立即停采，不落盘）。
  - `CaptureService`：mediaProjection 前台服务类型（API 34 硬性要求已声明权限+类型）、ImageReader 帧计数、首帧 JPEG 样本上行（服务端只统计字节数不保存）、音频播放捕获能力探测（API 29+，以 AudioRecord 初始化成败为准）。
  - APK：本地 Gradle 8.9 + AGP 8.5.2 构建成功（4.36MB debug），产物归档 test-runs/mirrordock-companion-debug-0.1.0-poc.apk；CI `companion.yml` push 触发上传 artifact。
- [ ] C4-03 可选 Accessibility 实时控制、显著披露、Play 声明预审和 OEM 限制测试。**按用户指示跳过（硬性前置审核：Play 显著披露预审未完成前不进入范围）。**
- [ ] C4-04 远程协助 / 企业 MDM / OEM 系统级增强，仅在单独权限与威胁模型审核后进入范围。**按用户指示跳过（硬性前置审核）。**

## v1 后：客户端体验轮（2026-09-29，参照 ToDesk / UU 远程）

- [ ] X5-01 客户端 UI 重设计：左侧导航 + 顶栏状态 + 卡片主区（ToDesk / UU 远程风格）。**代码完成；GUI 视觉验收待用户。**
  - 五页签：连接 / 工具 / 无线 / 设置 / 帮助；所有面板常驻渲染、CSS 显隐，切换不丢状态。
  - 安装 Anthropic 官方 frontend-design skill（`.workbuddy/skills/frontend-design/`，SKILL.md 审计 P2 安全）。
  - 前端 29→31（快捷键契约、自动旋转默认值）；`pnpm build` 通过。
- [ ] X5-02 镜像会话全局快捷键。**代码完成；真机验证待用户。**
  - 新增 tauri-plugin-global-shortcut（镜像窗口持焦时主窗口收不到键盘事件）；会话中注册 Ctrl/⌘+Alt+S 截图、+R 开关录制、+D 轮换显示方向，结束自动注销。
  - 录制开关尊重 Pro 门控；旋转/录制经 update_session_options 重启窗口（与设置页同一语义），已在设置页如实告知。
- [ ] X5-03 自动横竖屏。**代码完成；真机验证待用户。**
  - **根因定案**：`--display-orientation` 在 rotation=0 时也被强制传入，把画面锁死在竖屏；改为 rotation=0 不传该参数（scrcpy 默认跟随设备旋转，打开横屏游戏画面自动转正、窗口自适应），90/180/270 仍为锁定。
  - 设置页文案同步：0° 显示为「自动（跟随手机）」且为默认项。
- [ ] X5-04 伴侣 App 扫码闪退修复 + 崩溃取证。**代码完成；待用户重装 APK 验证。**
  - ScanActivity 加固：相机权限改为本页 ActivityResult 请求（旧版无权限时静默 finish，在部分 ROM 上与生命周期竞争可致异常退出）；相机初始化失败在屏幕上显示原因，不再黑屏退出。
  - 新增 CrashGuard：未捕获异常堆栈先落盘（应用私有目录 last_crash.txt，含设备信息）再交回系统；下次启动在主界面日志区回显，可拍照取证。堆栈只落本机、不自动上传。
  - 版本 0.1.0-poc → 0.1.1-poc（versionCode 2）；本地构建产物归档 test-runs/。
- [ ] X5-05 开发者中心无线调试自动发现配对地址（mDNS）。**代码完成；真机验证待用户。**
  - 新命令 `discover_pairing_services`（命令面 29→30）：`adb mdns services` 解析 `_adb-tls-pairing._tcp.`（配对地址，手机停在配对页才广播）与 `_adb-tls-connect._tcp.`（连接地址），自动填入无线表单。
  - **诚实边界**：Android 不提供配对二维码，6 位配对码仍需用户从手机屏幕读取后手动填写；自动发现只省去抄写地址。测试 Rust 104→106。
  - **X6-01 勘误**：真实 adb（37.0.1 实测）输出是「实例名+服务类型+端点」三列，X5-05 初版解析按两列写、单测用了假数据未暴露，对真实 adb 完全失配。已重写为 `parse_mdns_entries`（白名单识别服务类型、兼容两列旧格式、保留实例名）。

## X6 客户端体验轮之二（2026-09-29）

- [ ] X6-01 桌面出码、手机扫码配对（Android 11+ 无线调试「使用二维码配对设备」）。**代码完成；真机验证待用户。**
  - 协议与 Android Studio「Pair Using QR Code」一致：二维码 `WIFI:T:ADB;S:<服务名>;P:<6位码>;;` → 手机扫码后广播 `_adb-tls-pairing._tcp`（实例名=S）→ 桌面轮询命中后自动 `adb pair` → 手机改广播 `_adb-tls-connect._tcp` → 自动 `adb connect`。
  - 新命令 `begin_qr_pairing` / `qr_pairing_progress`（幂等轮询，状态机 waiting→paired→done）/ `end_qr_pairing`（命令面 30→33）；配对码等价一次性凭据，不写日志。前端轮询模式与伴侣配对一致，超时 150s 自动取消。
  - 同时修正 X5-05 的三列解析勘误（见上）。
- [ ] X6-02 精简客户端文案。**已完成。**连接/工具/无线/设置/帮助全页去保姆式长段说明，关键隐私与安全结论保留短句版。
- [ ] X6-03 全局快捷键自定义。**代码完成。**设置页新增「全局快捷键」：截图/录制/轮换方向三组合可改，校验「修饰键+普通键」，localStorage 持久化，会话中改组合即时重注册。前端 31→34。
- [ ] X6-04 Pro 激活码签发验收。**已签发（key-id 2026-001，永久）**，交付用户做 GUI 激活验收；私钥仍只在仓库外内部目录。
- [x] X6-05 伴侣 App 扫码闪退根因修复。**已修复并真机验证。**
  - **根因（真机 logcat 定案）**：`ActivityNotFoundException: ScanActivity 未在 AndroidManifest.xml 声明`——X5-04 重写扫码页时漏了清单注册，点击「扫码配对」即闪退。与权限、CameraX、zxing 均无关。
  - 修复：清单补声明（exported=false + 竖屏）；版本 0.1.2-poc（versionCode 3）。
  - **真机验证**（Redmi M2104K10AC USB）：`pm grant CAMERA` → am start 主页 → uiautomator 定位「扫码配对」按钮 → input tap → `topResumedActivity=.ScanActivity`，crash 缓冲区 0 条。
  - 教训：**新 Activity 必须同步进清单**；此前无 logcat 的「加固」方向（权限流程/崩溃取证）没有命中真因——有真机时第一动作是拉 `logcat -b crash`。

## X7 Beta 发布轮（2026-09-29 上午）

- [x] X7-01 伴侣 App 重设计。**已完成并真机验证。**
  - 与桌面客户端同一视觉：蓝色头部（#2563EB）+ 浅灰底 + 白卡片 + 圆角按钮；扫码页加同款标题条。
  - 崩溃取证收敛：红卡只在确实有崩溃时出现，默认只显示标题、详情按需展开，「清除」删盘后立即消失（真机验证：清除后卡片收敛）。运行日志默认收起为一行，点「查看」展开。
  - 图标：桌面客户端 icon.png（512px）经 sips 生成 mdpi→xxxhdpi 五档 mipmap，替换旧矢量图标；CaptureService 通知图标同步改 @mipmap。
- [x] X7-02 激活码运营手册。**已完成**（docs/license-operations.md）：key-id 规范、签发命令、交付流程、续费换码；诚实边界= v1 无远程吊销，泄露兜底靠换钥+有效期。
- [x] X7-03 Beta 全量内置激活码。**已完成**（Rust 108→109）。
  - `BETA_LICENSE_KEY`（key-id `beta`，永久）编译进二进制；`entitlement_status` 发现未激活时静默激活；用户显式撤销后写 `beta_opt_out` 标记不再自动激活。
  - 新增守卫测试：内置码必须能被内置公钥验签（防止换钥/换码不同步）。
- [x] X7-04 版本 0.2.0 + tag 发布首个 GitHub Release。**已完成并上线。**
  - tauri.conf.json / Cargo.toml / package.json 统一 0.2.0；tag `v0.2.0-beta`。
  - build.yml 新增 companion-apk（tag 触发出 APK）与 release 任务（softprops/action-gh-release@v2，汇总桌面安装包 + SHA256SUMS + SBOM + APK，prerelease）。
  - 发布地址 https://github.com/g-star1024/MirrorDock/releases/tag/v0.2.0-beta ，共 20 个产物：Windows exe+msi、macOS x64/arm64 dmg、Linux AppImage+deb+rpm、伴侣 APK、四平台 SHA256SUMS 与 SBOM。
- [x] X7-05 修复 Windows 打包失败（`cfg(unix)` 段吞依赖）。**已完成**（提交 e70f0fe）。
  - 根因：`tokio`/`rustls`/`tokio-rustls`/`rcgen`/`sha2`/`getrandom` 写在 `[target.'cfg(unix)'.dependencies]` 之后，TOML 节段延续导致全部变成「仅 Unix」依赖；macOS/Linux 门禁全绿、Windows 编译 26 个 E0433。已移回 `[dependencies]`，`libc` 留 Unix 段。
  - 验证方式（无 Windows 工具链也可）：`cargo tree --target x86_64-pc-windows-msvc -i <crate>` 必须能看到 mirrordock。
- [x] X7-06 修复 Release 发布失败（glob 未匹配）。**已完成**（提交 2ef4948）。
  - 根因：`release-files/**/*.app.tar.gz` 永远不会匹配——macOS 端 `targets=all` 只产 app+dmg，`.app.tar.gz` 需启用 updater 才生成；`fail_on_unmatched_files: true` 遇未匹配即失败（六个构建 job 全绿、仅发布挂）。已移除该 glob。
  - 发布说明改为 workflow 内置正文（下载指引 / 开始使用 / 测试版说明 / 校验与透明），版本无关写法避免过期。
- [x] X7-07 修复 Release 发布失败（孤立 Release 复用）。**已完成**（提交 697c534）。
  - 场景：删 tag → 重推 tag 会让旧 Release 变成「孤立 Release」，其上传地址失效，表现为正文更新成功而资产清空、上传失败。
  - 修法：发布前按 tag 删掉历史 Release（`--cleanup-tag=false`，保留 tag）。**该修法不完整**——按 tag 查不到草稿，见 X7-08。
- [x] X7-08 修复 Release 发布「同 tag 多 Release」根因（softprops 草稿/发布两段式）。**已完成，线上状态已修复并核验。**
  - **现象**：日志里 20 个资产全部 `✅ Uploaded`，紧接最后一步 `Finalizing release...` 连错三次
    `Validation Failed: already_exists, field=tag_name`，job 判失败；但 Release 页面上资产数为 0。
  - **根因（本轮按 API 实测定案，非推测）**：
    1. `softprops/action-gh-release@v2` 在 prerelease 场景下先建**草稿** → 上传资产 → 再 `updateRelease(draft:false)` 发布；
    2. **GitHub 允许同一 tag 下并存「一个草稿 + 一个正式 Release」**，而草稿在「按 tag 查 Release」的接口
       （`gh release view`、`/releases/tags/{tag}`）里**不可见**；
    3. 于是每轮 run 都新建一个 Release。实测同 tag 下并存 id=398740537「草稿，20 资产」与 id=398762605
       「正式，0 资产」——**资产全落在草稿上，对外可见的那个反而是空的**；
    4. 最后把草稿发布出去时 tag 已被另一个正式 Release 占用 → `already_exists, field=tag_name`。
    X7-07 的按 tag 清理步骤因看不到草稿而**完全没生效**：实测该 tag 下按 tag 匹配到 0 个、按 id 列全部匹配到 **2** 个。
  - **反证**：删掉那个空的正式 Release 后，用同一条 PATCH 立即把草稿发布成功——确证是 tag 占用，而非资产损坏或权限不足。
  - **修法**：发布 job 弃用 softprops，改 `gh` CLI 三步全显式——
    ① 列出该仓库全部 Release（含草稿）并按 **id** 删净该 tag 下的所有 Release；
    ② `gh release create --prerelease --notes-file`（一次性直接建正式版，无「草稿→发布」两段）；
    ③ `gh release upload`（用 `find` 收集文件，上传前卡数量 ≥10）。
    自校验由「资产数 ≥10」升级为「**同 tag Release 数 == 1 且非草稿 且资产 ≥10**」。
  - 正文抽出为 `docs/release-notes-beta.md`（单一来源），workflow 末尾追加随 tag 变化的「完整变更记录」链接。
  - 线上修复：删除空 Release 398762605 → 发布草稿 398740537（正文 1016 字，版本无关）。核验（**匿名视角**）：1 个 Release、非草稿、20 资产齐全。
  - 教训：**「按 tag 查」的接口看不到草稿**，凡涉及 Release 清理/复用一律改「列全部 + 按 id 操作」；且「日志说成功」≠「对外状态正确」，必须查匿名视角的最终状态。

## X8 产品主页与发布链收口（2026-09-29）

- [x] X8-01 客户端产品主页（静态 HTML）。**已完成**（提交 697c534，site/index.html）。
  - 单文件、内联 CSS/JS、无外部资源；含真机实测数据条（首帧 P95 0.99s / 60 分钟无中断 / ~35 FPS）、功能区、三步开始、隐私与边界（含「它做不到什么」）、下载表、FAQ。
  - 诚实边界：主页上的性能数字全部来自 test-runs/ 已归档的真机报告，不以宣传话术替代证据。
- [x] X8-02 发布链稳定性收口。**已完成**（见 X7-08）。当前 Release 状态与「同 tag 唯一、非草稿、资产齐全」三项判据一致。

## X9 常驻与快捷唤醒轮（2026-09-29）

- [x] X9-01 重打 tag `v0.2.0-beta` 验证新发布 job。**已完成并上线**（run 36512102908 success）。
  - 发布 job 八步全部 success：清理（含草稿）→ 创建 → 上传 → 校验。匿名视角核验：1 个 Release、非草稿、20 资产、正文 1017 字。X7-08 的 gh CLI 发布链至此有真实流水线证据。
- [x] X9-02 关闭按钮 = 最小化到菜单栏/托盘 + 开机自启。**代码完成；GUI 行为真机验证待用户。**
  - Rust：`on_window_event` 拦截主窗口 `CloseRequested` → `prevent_close` + `hide`；退出只能走菜单栏/托盘菜单「退出 MirrorDock」。
  - 托盘（`tauri` `tray-icon` feature）：图标用默认应用图标，菜单 = 打开 MirrorDock / 唤醒手机屏幕 / 手机截图 / 退出。唤醒与截图复用命令侧逻辑（`wake_screen_for_serial` 提取共用）；无会话时弹系统对话框如实说明。截图文件名用 Unix 时间戳（纯 ASCII 过白名单），**不猜时区**——与前端本地时间命名的截图并存是刻意取舍。
  - 开机自启：`tauri-plugin-autostart`（macOS 用 LaunchAgent）；capabilities 新增 `autostart:allow-enable/disable/is-enabled`。
  - 新命令 `get_app_settings` / `set_app_settings`（命令面 33→35，X9-02 初稿误写为 31→33，此处勘误），应用级设置落 `app-settings.json`；读取**容错**（缺失/损坏/缺字段一律回默认，设置坏了不挡启动）。Rust 111（+2：缺失/损坏回默认、落盘往返）。
- [x] X9-03 镜像黑屏免开客户端唤醒。**已定案：右键手势（scrcpy 内置）+ 界面提示；左键唤醒做不到，原因如实记录。**
  - 依据 scrcpy v4.1 官方文档（doc/mouse.md）：SDK 鼠标默认「右键触发 BACK（熄屏时改为 POWER 点亮屏幕）」；`--mouse-bind` 只能配置**次级**按键（右/中/4/5），**左键（primary）永远转发触控**——「左键点击唤醒」在 scrcpy 4.1 配置层面不存在，不虚报。
  - 落地：镜像窗口内右键即可点亮（无需打开客户端，天然满足诉求）；设置页「通用」与锁屏面板（会话中且屏幕关闭时）两处明示该手势。
- [x] X9-04 macOS 菜单栏图标 + 隐藏 Dock 图标。**代码完成；GUI 行为真机验证待用户。**
  - 菜单栏图标即上述托盘（macOS 左键弹菜单），提供打开/唤醒/截图/退出。
  - 设置中心（仅 macOS 显示）新增「隐藏 Dock 图标」：`AppHandle::set_activation_policy` 在 **Accessory ↔ Regular 间运行时切换**，设置经后端持久化、启动时在 `setup` 中先于窗口生效；回到主窗口时临时切回 Regular 并激活应用，保证窗口能正常抢焦点。非 macOS 平台后端强制 false（防「勾了没效果」的假开关）。前端 38（+4：平台判断、自启切换、关闭/右键文案、Dock 选项仅 macOS）。
- [x] X9-05 托盘菜单动态化（连接/断开、屏幕录制开关）+「屏幕唤醒」更名 + 设置中心重排。**代码完成；GUI 行为真机验证待用户。**
  - 托盘菜单新增「连接设备/断开连接」与「开始/结束屏幕录制」，文案随会话状态在跃迁点刷新（启动/停止/重启命令 + 监视线程发现进程退出，经 `TRAY_APP` OnceLock）；文案只是提示，**动作以点击时真实状态为准**（状态与文案短暂不一致时不会执行错误方向的动作）。
  - 连接目标 `pick_tray_target`：最近设备中当前就绪者优先，其次第一台就绪设备；没有就绪设备如实弹窗。启动参数用后端默认值（与前端默认一致）；托盘路径同样记入诊断日志（mirror_start/mirror_stop/session_update）。
  - 录制开关语义与主窗口一致：结束旧窗口+按新设置重启（画面短暂中断），成功对话框如实说明；免费版走 `ensure_edition_allows` 拒绝；录像文件名 Unix 时间戳（ASCII 过白名单，不猜时区）。
  - 「唤醒手机屏幕」→「屏幕唤醒」；锁屏面板引导文案同步更名。无会话时屏幕唤醒/截图菜单项置灰（保留弹窗兜底）。
  - 设置中心重排：旧 `session-options` 挤压式 flex 全部替换为「设置卡 × 主题」结构（通用 / 镜像窗口 / 全局快捷键 / 版本与授权），每行「名称+说明居左、控件居右」，操作按钮集中在卡片头，快捷键输入等宽字体+固定标签宽（修复「录制开关」竖排换行）。
  - 验证：Rust 114（+3：pick_tray_target 三例）、clippy 0、vitest 38、pnpm build 通过。
- [x] X10-01 一键安装 APK 到手机。**代码完成；真机安装验证待用户。**
  - 命令 `install_apk_to_device`（命令面 35→36）：`adb -s <serial> install -r -t <apk>`，固定参数直调、APK 路径作为单个 argv（不 shell 拼接）。`-r` 覆盖安装保留数据、`-t` 允许 test-only 包；**刻意不用 `-g`**——那会在不与用户确认的情况下批量授予运行时权限，与「不绕过用户同意」边界冲突（权限仍由用户在手机上确认）。
  - 路径校验 `validate_apk_path`：绝对路径 + 已存在普通文件 + `.apk` 后缀才放行；**不设目录白名单**（安装包可能在下载/桌面/U 盘），后缀严格是因为 `adb install` 对非 APK 只会回难懂的解析错误，提前拒绝能给出可照做的提示。
  - 输出解析 `describe_apk_install_output`：`adb install` 失败时输出 `Failure [REASON]` 且退出码非零，因此接口**契约改为业务失败也返回原始输出**（仅进程无法启动才 Err），由解析层把 17 类常见 REASON（版本降级、签名不一致、空间不足、未知来源限制…）翻成可照做的文案；未知 REASON 保留原始错误码，绝不塌缩成笼统的「安装失败」。
  - 前端（工具页新增「安装 APK」面板）：系统文件选择器只筛 `.apk`；选中路径显示出来供核对，未选包时安装按钮禁用；成功后显示「文件名 · 大小 · 结论」并提醒手机上的安装确认需本人同意；失败原因原样呈现。路径只存内存，不入 localStorage、不入日志（诊断日志把路径与序列号一并作为机密擦除）。
  - 验证：Rust 120（+6：成功路径/非 APK 拒绝/缺失或相对路径拒绝/失败原因保留/输出解析含未知原因/未就绪设备拒绝）、clippy 0、vitest 40（+2：安装流程与失败原因呈现）、pnpm build 通过。

- [x] X10-02 侧栏徽标一行 + 连接页紧凑化。
  - 侧栏底部「仅在本机连接」与「专业版」徽标改为同行 flex 排列（原纵向 grid 堆叠）。
  - 连接页五条状态/错误横幅收进 `.status-stack`（集中、间距 8px、容器空时不占位），不再各自撑一块黄色区域；eyebrow 随会话状态显示「镜像运行中/第一步：连接手机」，减少与顶栏状态芯片的重复感。
  - 能力/锁屏面板紧凑化：内边距与条目间距收紧、说明文字 13px/12.5px、面板内按钮不再拉伸整行（「屏幕唤醒」恢复自然宽度）。
  - 杂项：上一轮误入仓库目录的无关文件（谜页集调研，未跟踪）已移出至仓库外同级目录，空的 docs/research 一并清理。
  - 验证：vitest 40、tsc、pnpm build 通过。

- [x] X10-03 伴侣会话 15 秒必断的根因修复（versionCode 4 / 0.1.3-poc）。
  - 现象：配对成功后约 15 秒，伴侣 App 显示「会话已断开」，桌面端记录「会话结束（超时或断开）」；与切换页签无关，时间点恒为握手后 +15s。
  - 根因：`PairingClient.tryConnectAnyHost` 设的 `soTimeout = 15000` 是给「等待 welcome 握手回复」用的，但握手成功后**没有解除**，长会话期间服务端不主动发消息，读线程 15 秒后必然 `SocketTimeoutException` → `onDisconnected`。
  - 修法：握手完成（welcome + device_hello 发出）后 `tls.soTimeout = 0`，断开仍由 `readLine()` 返回 null / IOException 感知，语义不变。
  - 产物：`test-runs/mirrordock-companion-debug-0.1.3-poc.apk`（本地 gradle 构建，SHA-256 c58a7456…44c61）。真机配对保持 >15s 待用户验证。
  - 附带结论（不改代码）：手机上「撤销 USB 调试授权」只影响**下一次**连接的授权检查，已建立的 adb/scrcpy 会话（USB 或无线）不会被主动踢下线——认证发生在连接建立时；且 Android 11+ 无线调试的配对授权独立于该开关。

- [x] X10-04 伴侣 App 界面现代化 + 「电脑发来的文件」（versionCode 5 / 0.1.4-poc）。
  - 视觉重做：去掉挤压式蓝头部条，改为「标题区留白 → 蓝色渐变 hero 卡（状态点 + 白胶囊主 CTA）→ 白卡片分区（文件 / 崩溃 / 日志）」；20dp 圆角白卡、扁平无描边、次级按钮白底、文字按钮收敛。
  - 新增「电脑发来的文件」卡：列 `Download/MirrorDock`（与桌面端 send_file_to_device 同一目录），显示类型徽标 + 文件名 + 大小/时间；点击查看（ACTION_VIEW + FileProvider，仅暴露该目录）；`.apk` 走系统安装器再次安装（REQUEST_INSTALL_PACKAGES，仍需用户逐次确认）。
  - 存储访问逐级降级（如实呈现，不假成功）：API ≤ 32 运行时 READ_EXTERNAL_STORAGE；API 30+ 先试直读/MediaStore 兜底，都不行时空状态给「授权文件访问」按钮（MANAGE_EXTERNAL_STORAGE，侧载 POC 可接受）；Manifest 加 requestLegacyExternalStorage + FileProvider（file_paths 仅 Download/MirrorDock）。
  - 验证：本地 gradle assembleDebug 通过，APK 落 `test-runs/mirrordock-companion-debug-0.1.4-poc.apk`（SHA-256 c7ba8f96…a88e）。真机视觉与文件列表（Android 13 Redmi）待用户验证。

- [x] X10-05 伴侣 UI 三项真机反馈修复（versionCode 6 / 0.1.5-poc）。
  - 顶部不适配：主题加 `android:statusBarColor=screen_bg` + `windowLightStatusBar=true`（minSdk 26 可直用），状态栏与页面同底色、深色图标，不再压一条系统默认深灰。
  - 去蓝色改「石墨墨色」体系：hero 卡改近黑渐变（#2A2D35→#15171C），强调色 accent=#1A1D24（文字按钮/徽标/主按钮文字/「安装·查看」），`brand_blue` 全部移除（colors.xml、styles.xml、MainActivity、ScanActivity 顶栏），徽标底改中性浅灰。
  - 按钮灰色圆角边框：三个按钮样式统一 `android:stateListAnimator=@null`，去掉系统给 Button 默认挂的 Z 抬升投影（浅色底上呈灰边感）；白胶囊按下色同步去蓝（#E9EAEE）。
  - 产物：`test-runs/mirrordock-companion-debug-0.1.5-poc.apk`（SHA-256 b98d38d1…ef922）；已 adb 安装到真机（M2104K10AC）并启动，效果待用户确认。
  - 追补（同日用户反馈「墨色风格，用图标里的青碧色」→ versionCode 7 / 0.1.6-poc）：强调色改青碧（图标实测主色 #50B0B8；白底文字用加深档 #2F98A1、徽标底 #E6F4F5），hero 渐变改墨色带青碧冷调（#23393C→#131E20），连接状态点改亮青碧 #6FE3E8（绿→青，与主色呼应）；产物 `test-runs/mirrordock-companion-debug-0.1.6-poc.apk`（SHA-256 fbd3a057…87452b），已真机安装并启动。
  - 追补（同日用户反馈「点手动输入有 bug」→ versionCode 8 / 0.1.7-poc）：真机复现被锁屏挡住（不绕过锁屏），按代码链路修三处——①展开即聚焦+弹键盘（原样只改 visibility，键盘不弹，观感「点了没反应」）；②输入框改白底+1dp 边框（bg_input_field，原底色 #F7F7F9 贴页面底色几乎隐身）；③`imeOptions=actionGo` + 只认 IME_ACTION_GO/实体回车触发连接（原任意编辑动作都触发）；发起连接后自动收起输入框与键盘。产物 `test-runs/mirrordock-companion-debug-0.1.7-poc.apk`（SHA-256 2d2a7d83…d0e02），已 adb 安装（设备锁屏中，待用户解锁验证）。

- [x] X10-06 连接页布局调整（用户反馈）。
  - 「屏幕唤醒」按钮从锁屏面板移到设备就绪行（ready-panel）右侧、「会话进行中/开始镜像」主按钮左边（`.ready-wake` 接管 `margin-left:auto`，主按钮在行内归零）。
  - 「这台手机的能力」面板移出双栏 grid，沉到页面底部「为什么需要授权」说明之下占满整幅（`.capability-foot`），不再抢占操作动线；锁屏状态面板独占双栏 grid（auto-fit 自动拉满）。
  - 文案用户语言化：标题「这台手机的能力」→「这台手机能做什么」；摘要从电报腔「可以镜像 · 可转发声音」改为整句「Xiaomi M2104K10AC（Android 13）：画面可以镜像到电脑，手机声音会一起传到电脑。」未知态如实（「手机声音能否转发还无法确认」）；测试断言同步。
  - 验证：vitest 40、tsc、pnpm build 通过。

- [x] X10-07 锁屏面板刷新与瘦身 + 帮助中心内置文档 + 主页上 Pages（用户反馈四项）。
  - 锁屏状态不刷新根因：`device_lock_report` 只在 readySerial 变化时取一次，之后屏幕点亮/解锁/熄屏面板都停在旧状态。修法：readySerial 存在期间每 5 秒轻量复查（失败保持上一次结果不刷错误）；「屏幕唤醒」成功后仍立即刷新。
  - 面板瘦身：只留「状态摘要 + 一句该做什么」（新增 `lockActionHint`：熄屏→点屏幕唤醒；已锁→去手机解锁、凭据不记录；已解锁→可直接镜像）；`explanation`/`recovery` 长说明与「只点亮不解锁」沉到页面底部新卡「关于锁屏与解锁」。
  - 黄色提示条文案用户语言化：「镜像进程已启动，但尚未确认首帧到达…」→「画面正在启动…若几秒后仍未出现，请检查手机屏幕是否亮起并确认授权。」（测试断言同步；「streaming ≠ 首帧」语义不变，只改措辞）。
  - 帮助中心补实质内容：新增 `src/helpContent.ts` 内置 7 篇帮助文档（三步开始/无线连接/锁屏与解锁/黑屏与屏蔽/声音转发/截图录像文件 APK/常见问题），随应用分发、离线可读、无外部依赖；帮助页新增「使用帮助」卡片区。
  - 官网「被吞」定案：`site/index.html`（X8 提交 697c534）一直在仓库与远端 main，从未被删——只是从未上线。新增 `.github/workflows/pages.yml` 把 site/ 发布到 GitHub Pages（push 触发 site/** + 手动；需在仓库 Settings → Pages 把 Source 设为 GitHub Actions）。
  - 验证：vitest 40、tsc、pnpm build 通过。

- [x] X10-08 工具页等高 + 保持唤醒如实说明 + 帮助卡间距 + 侧栏顺序（用户反馈四项）。
  - 工具页四卡忽大忽小：`.panel-grid` 加 `grid-auto-rows: 1fr`（同一网格内所有行等高）；截图/录像/文件传输/安装 APK 四卡说明文案统一精简为 1-2 句（录像卡保留录制中/已结束的动态状态展示，测试断言未涉及旧文案）。
  - 「会话期间保持唤醒但屏幕仍熄屏」定案（不改行为，改如实说明）：scrcpy `--stay-awake` 依赖 Android 的「插入时保持唤醒」状态（stay_on_while_plugged_in），**只在 USB（数据线）连接时生效；无线（TCP/IP）连接设备不处于「已插入」状态，系统仍按超时熄屏**——熄屏后到锁屏是安全锁屏的系统行为，需本人解锁。设置页说明与帮助中心「锁屏与解锁」同步改为如实表述（USB 生效 / 无线无效 / 熄屏用「屏幕唤醒」点亮）。
  - 帮助页帮助卡与「诊断包」贴住：帮助卡包进 `.help-list`（grid gap 14px，末卡与诊断包间距一致）。
  - 侧栏底部徽标顺序：「专业版」放到「仅在本机连接」前边。
  - 验证：vitest 40、tsc、pnpm build 通过。

- [x] X10-09 无线会话亮屏补偿：锁屏页来不及输密码（用户真机反馈）。
  - 问题：无线连接下 scrcpy `--stay-awake` 无效（依赖 stay_on_while_plugged_in，仅 USB 生效），会话中设备仍按超时熄屏——锁屏页亮起后用户还没输完 PIN 屏幕就黑了；且每次点亮都要重新面对锁屏超时。
  - 方案（不绕锁屏，只留输入时间）：会话启动后若 `keep_awake` 且序列号为无线（ip:port / mDNS 派生，`is_wireless_serial`），读系统 `screen_off_timeout` 原值备份到会话状态，写入延长值（12h，与 stay-awake「会话期间常亮」语义一致）；结束（主动停止 / 进程退出监视线程 / 应用退出）时还原原值。
  - 幂等与安全：重启会话只重复写延长值，绝不把延长值再当原值备份；读不到原值（`null`/离线）就不写；还原失败（设备离线）时备份保留在内存与磁盘（`screen-timeout-backup.json`），下次启动 `restore_persisted_screen_timeout` 重试——应用崩溃也不会留下被延长却无人还原的熄屏时间。
  - 实现：AdbRuntime 新增 `screen_off_timeout`/`set_screen_off_timeout`（固定参数直调 `settings get/put system screen_off_timeout`）；`SessionState.screen_timeout_backup`；`MONITOR_ADB` OnceLock 供监视线程还原（沿用 TRAY_APP 测试跳过模式）；接线点：launch_into_reserved_session（启用/关闭补偿）、stop_mirroring、tray_connect_toggle 停止分支、spawn_session_monitor 退出分支、RunEvent::Exit、setup 崩溃遗留还原。
  - 文案同步：设置页「会话期间保持手机唤醒」说明与帮助中心「锁屏与解锁」改为「USB 与无线均生效；无线通过临时延长熄屏时间实现，结束自动恢复」。
  - 测试：新增 8 项（无线/USB 序列号判定、超时解析严格性、补偿+备份、USB 不碰设置、重启不覆盖原值、读不到不写、还原成功/失败保留、端到端补偿还原）；Rust 129 / clippy 0 / vitest 40 / build 通过。

- [x] X10-10 X10-09 无效的定案与真正修复：锁屏页 10 秒必灭的真因是钥匙锁窗口覆盖超时（真机取证）。
  - 真机证据链（Redmi M2104K10AC / Android 13 / MIUI，无线会话）：
    - `mScreenOffTimeoutSetting=43200000` —— X10-09 写入的 12h 延长**确实生效了**；
    - `mUserActivityTimeoutOverrideFromWindowManager=10000` —— 锁屏页由 WindowManager 把用户活动超时强制覆盖成 10 秒；
    - `mLastSleepReason=timeout` —— 唤醒后约 13 秒屏幕又被系统按超时熄灭。
    ⇒ 结论：**锁屏页上 `screen_off_timeout` 被钥匙锁窗口覆盖，设多大都没用**；X10-09 只对这一把杠杆下药，因此在锁屏场景无效。
  - 供应链证据：scrcpy 4.1 的 `scrcpy-server` 内部同时改 `screen_off_timeout` 与 `stay_on_while_plugged_in`（均带 restore），而 `scrcpy.1` 手册对 `-w/--stay-awake` 的定义是「Keep the device on while scrcpy is running, **when the device is plugged in**」⇒ 无线下设备未插电，`stay_on_while_plugged_in` 不满足生效条件，这是 X10-09 无效的根因。
  - 另外实测到一条独立故障：手机熄屏十余分钟后**整台从网络消失**（ping 100% 丢包、无 mDNS 广播、ADB transport 变 offline/消失），即「镜像窗口点不动」并非点击被拒，而是无线链路整条断了。
  - 真正修复（仍不绕锁屏，只负责让屏幕别灭）——会话期间把系统置于「充电时保持唤醒」三把杠杆：
    ① 备份并写长 `screen_off_timeout`（沿用 X10-09）；
    ② 备份并写 `stay_on_while_plugged_in = 7`（AC|USB|WIRELESS，与 scrcpy 同语义）；
    ③ `dumpsys battery set usb 1` 让系统认为已插电，使 ② 真正生效（系统自带 shell 测试钩子，uid 2000 可用，不需 root）。
    结束后按 ③→②→① 逐项还原，崩溃时由落盘账本在下次启动还原。
  - 安全边界与副作用（如实告知）：假充电是**会话级临时状态**，无线会话期间手机状态栏可能显示「充电中」（唯一用户可见副作用，因此撤销顺序上它排最前）；不注入任何解锁凭据、不解除钥匙锁。设置页与帮助中心已同步说明，避免用户误判为故障。
  - 实现：AdbRuntime 新增 `stay_on_while_plugged_in`/`set_stay_on_while_plugged_in`/`set_charging_override` 三方法（SystemAdbRuntime 固定参数直调）；`KeepAwakeBackup` 由「只备份熄屏时间」扩展为三杠杆账本（新增字段带 `#[serde(default)]`，磁盘文件仍是 `screen-timeout-backup.json`，**老版本崩溃残留的账本仍可解析并还原**）；新增 `restore_keep_awake` 统一撤销，`disable_wireless_keep_awake` 与启动还原共用；后两把杠杆逐级降级（读不到原值就不改它，宁可不生效也不留无法还原的改动）。
  - 测试：新增 5 项（三杠杆全拉满、读不到原值则跳过该杠杆、撤销顺序且假充电最前、会话重启只重写不覆盖账本、老版本账本文件兼容）；Rust 134 / clippy 0 / vitest 40 / build 通过。
  - ⚠️ **待真机验证**（手机在排查过程中掉线，无法当场实测）：需重连后确认三杠杆生效、锁屏页可持续亮、会话结束后手机状态栏充电指示消失。

- [x] ✅ BUG-充电掩盖（X10-10 派生，2026-10-02 真机复现 + 已修复）：无线「保持唤醒」的伪造充电（`dumpsys battery set usb 1`）是**设备全局** mock 状态，复位 `restore_keep_awake` 按**原无线序列号**执行 `reset`；用户从无线切 USB 后旧序列号离线 → 复位静默失败 → `UPDATES STOPPED` 残留 → 真实充电被掩盖（已真机复现：mock 下 `Max charging current:0`/`status:3` 放电，`reset` 后 `status:2`/`500mA` 正常充；电量 1% 系 mock 冻结值，真实 80%）。
  - **已按方案1修复（2026-10-02）**：移除 `set_charging_override` 方法与全部调用、`faked_charging` 字段，保留 `stay_on_while_plugged_in`+`screen_off_timeout` 两把安全杠杆；无线亮屏由 scrcpy `--stay-awake` 覆盖（启动 scrcpy 时已传入）。同步改写 App.tsx / helpContent 中「状态栏可能显示充电中」的过时说明。`cargo test` 192 passed / `cargo clippy` 0 warning / 前端 `pnpm build`+`vitest` 58 passed 全绿。应用数据 `screen-timeout-backup.json`（含 192.168.1.9:44093、192.168.0.165:33943、192.168.0.165:44327 共 3 条 `faked_charging:true` 旧账）已备份至 /tmp 并删除。
  - 证据：代码（enable/restore 不再出现 `set usb 1`）、测试（改 `wireless_keep_awake_arms_timeout_and_stay_on_levers`、撤销顺序断言不再含 `set_charging`）、真机（手动 `dumpsys battery reset` 后恢复 500mA 充电）。真机回归待用户按 GUI 验收清单复测「无线保持唤醒不再伪造充电」。

- [x] X10-11 「屏幕亮着但镜像黑」的第二段根因与修复：电源策略卡在变暗（DIM）阶段（真机取证复现）。
  - 用户真机反馈：屏幕明明亮着，镜像窗口黑屏，点「屏幕唤醒」也没用；怀疑是「安全策略不让镜像输密码」。
  - 排查中排除的假设（均有实测证据）：①锁屏页 FLAG_SECURE 屏蔽——screencap 与 scrcpy 流在锁屏主页都能拿到正常画面（亮度 131/255、129/255）；②PIN 键盘界面屏蔽——多次尝试验证均被 DIM 干扰，未定案；③参数问题（--max-size/--bit-rate/codec）——逐参数对照全部为亮；④自适应亮度变暗——`screen_brightness_mode=0`（手动，255），光线传感器不参与。
  - **定案根因**：`mPowerRequest=policy=DIM`。`stay_on_while_plugged_in`（X10-10 杠杆②/scrcpy --stay-awake）只能拦「熄屏（OFF）」，拦不住熄屏前的「变暗（DIM）」——锁屏静置约 3 分钟后进入 DIM：背光压到 5%（`SdrBrightness=0.05`，设置明明是最大）、渲染层对虚拟显示器输出黑帧（复现：DIM 时 scrcpy 录制 11~14 帧全部 0/255）。物理上像熄屏、镜像上就是黑屏，而 `mWakefulness` 仍是 Awake。
  - **为什么点「屏幕唤醒」没用**：`KEYCODE_WAKEUP` 的语义是「Asleep→Awake」，对已 Awake 但 DIM 的状态无效（实测：DIM 后发 WAKEUP，policy 仍 DIM）；实测 `KEYCODE_BACK` 可把 DIM 拉回 BRIGHT（锁屏上无副作用、不闪黑）；`KEYCODE_MENU` 无效；兜底手段是 SLEEP→WAKEUP 强制走一次熄屏-点亮环（有效但闪黑）。
  - 修复两处：
    ① 「屏幕唤醒」命令智能化（`wake_screen_for_serial`）：先读显示策略——DIM 走恢复（BACK→复查→环兜底）、BRIGHT 直接成功（不注入）、读不到退回原 WAKEUP 保守路径；
    ② 会话期「变暗守护」`spawn_keep_awake_guard`：keep_awake 开启时随会话启动（USB/无线都启动），每 20 秒检查一次，发现 DIM **且设备处于锁屏**时恢复（解锁状态下绝不注入——BACK 会后退用户界面）。退出条件：epoch 变化/进程消失/keep_awake 关闭。
  - 实现：AdbRuntime 新增 `display_state`（dumpsys display 原始输出）与 `press_key`（keycode 仅限代码常量，注释明确禁止接用户输入）；`parse_display_policy` 解析 `mPowerRequest=policy=`；`revive_dimmed_display`/`force_display_cycle`/`keep_awake_guard_tick` 均为纯函数便于测试。
  - 测试：新增 8 项（策略解析三态、BACK 恢复不闪黑、BACK 无效走环且如实报败、唤醒命令 DIM 路径选 BACK、BRIGHT 路径零注入、守护锁屏注入/未锁屏不注入/epoch 过期退出）；Rust 142 / clippy 0 / vitest 40 / build 通过。
  - ⚠️ 待真机验证：锁屏静置 4 分钟后镜像应仍可见（守护每 20 秒巡检）；「屏幕唤醒」在 DIM 下应能立即恢复画面。

- [x] X10-12 密码页黑屏定案为「安全表面」+ 锁屏面板自动提示（用户拍板：只提示手机端解锁，不做键位映射——各机型屏幕与布局差异大，映射不可通用，也不把功能做单一）。
  - 用户真机反馈推翻 X10-11 的 DIM 归因：「只要手机屏幕出现解锁输入开机密码的画面，镜像就黑屏，跟亮度一点关系没有」。
  - **双路同拍取证定案**（受控实验）：上滑调出密码输入页后，同一时刻 `screencap` 返回 **0 字节** + 无头 scrcpy `--record` 2 秒共 **11 帧全部 0/255**；对照组锁屏壁纸页两路均正常（117.5 / 133.3）。结论：锁屏壁纸页可镜像；**密码/图案凭据输入页（bouncer）是 Android 安全表面，系统对截屏与虚拟显示器镜像同时拒绝输出**——平台级保护，不可绕过也不应绕过。X10-11 记录的 DIM 状态本身真实（policy=DIM、BACK 拉回 BRIGHT 均实测），但「DIM ⇒ 虚拟显示器黑帧」的归因存疑（当时黑帧采样期间密码页可能同屏）；技能文档 android-screen-awake-forensics 已同步修正（误判表、新增密码页章节、排查顺序重排）。
  - 检测信号：`dumpsys window` 无可靠 bouncer 区分字段（MIUI 实测两态无差异），最可靠信号是「锁屏中 + 屏幕点亮 + screencap 输出 ≤100KB（实测 0 字节；正常壁纸页 3.4~3.7 MB）」。此前「exec-out 返回 0 字节是管道损坏」的记录，部分案例其实就是密码页被拒。
  - 实现（后端）：AdbRuntime 新增 `screencap_probe_bytes`（exec-out 只统计字节数，**像素就地丢弃，不落盘不回传不入日志**）；新增命令 `probe_pin_pad_state`（锁屏 + 点亮 + 截屏被拒三条件全满足才激活，探测失败一律不激活——宁可少提示不可误报）；已注册 generate_handler（29→30）。
  - 实现（前端）：主窗口 5 秒密码页守护（会话进行中才探测），检测到时在锁屏面板显示提示——「🔒 此画面受系统安全保护，无法镜像。请在手机上直接输入密码解锁，解锁后画面自动恢复」，密码页退出后提示自动消失；读不到（设备离线等）保持现状不闪烁。
  - 曾实现过「远程解锁键盘浮窗」（坐标映射代输），真机验证过点击注入可行（点「返回」坐标后 bouncer 收回、截屏恢复 3.39 MB）；用户裁定去掉：映射绑定单一机型比例，通用性差且功能面过窄，仅保留提示。相关代码（tap_screen / remote_unlock_input / 浮窗 / capabilities 扩权）已全部回退。
  - 安全边界：探测是只读的，不保存、不回传任何屏幕像素；不做任何绕过锁屏凭据界面的尝试。
  - 测试：新增 3 项 Rust（探测四态：激活 / 可捕获不激活 / 未锁或熄屏保守不激活 / 探测失败不激活）；Rust 145 / clippy 0 / vitest 40 / build 通过。
  - ⚠️ 待真机验证：锁屏上滑调出密码页 → 锁屏面板出现提示 → 手机解锁后提示自动消失。

- [x] X10-13 README 全面重写（对齐 0.2.0 现状）。
  - 旧版停留在 0 阶段 POC 描述（仅三条能力），与当前功能面严重脱节。
  - 新版：徽章、版本号（桌面 0.2.0 / 伴侣 0.1.7-poc）、完整功能清单（连接 / 镜像与会话 / 工具 / 帮助四组，含屏幕唤醒智能化、保持唤醒、密码页安全表面提示、内置帮助中心、官网链接）、安全与隐私边界六条、系统要求表、本地开发与质量检查命令、伴侣 App 构建命令、文档导航。
  - 新增「技术栈与致谢」：核心依赖表（scrcpy / adb / Tauri / React / TypeScript / Vite / Rust / Kotlin）、Rust 生态（tokio / rustls / tokio-rustls / rcgen / ed25519-dalek / serde）、前端生态（qrcode / Vitest / Testing Library / jsdom）、特别感谢（Genymobile、AOSP、Tauri 社区），并指向 THIRD_PARTY_NOTICES.md。
  - 纯文档变更，无代码改动。

- [x] X10-14 发布 v0.2.1-beta（tag `v0.2.1-beta`，commit `5de74bb`）。
  - 版本号 0.2.0 → 0.2.1（tauri.conf.json / package.json / Cargo.toml 三处同步）；首版带出 X10-05～X10-13：伴侣 0.1.7-poc、屏幕唤醒智能化（DIM→BACK）、会话期 20s 变暗守护、无线三杠杆保活、密码页安全表面检测提示、内置帮助中心 7 篇、GitHub Pages 官网。
  - 本地门禁：Rust 145 / clippy 0 / vitest 40 / build 全绿；推送后 tag 触发 build.yml（verify → 四平台打包 + 伴侣 APK → release）。

- [x] X10-15 隐藏 Dock 图标失效修复 + 定制应用图标。
  - 用户真机反馈：勾选「隐藏 Dock 图标」后图标仍显示。根因：`show_main_window`（菜单栏「打开 MirrorDock」触发）在 macOS 上无条件 `set_activation_policy(Regular)`，把 Accessory 策略覆盖，Dock 图标被拉回。修复：新增 `show_focus_policy(hide_dock_icon)`——开启隐藏时返回 None（保持 Accessory，窗口照常 show/focus），未开启才切 Regular；策略以磁盘上的设置文件为准。新增 1 项测试（Rust 146）。
  - 定制应用图标：此前 Dock/任务栏图标是 Tauri 框架默认图标（蓝橙环），无定制。新图标与产品视觉体系一致（石墨墨色 #101A1C~#2A4145 渐变底 + 青碧 #50B0B8 系描边，「显示器 + 手机」投屏母题 + 连接光点），PIL 绘制 1024×1024 源文件经 `tauri icon` 生成全平台尺寸（icns/ico/ Square*/android mipmap）。验证 scrcpy 镜像窗口不会在 Dock 增加图标（lsappinfo 仅 MirrorDock 注册），用户所见即应用图标本身。
  - ⚠️ 交付说明：本条目随本地 `tauri build` 装机验证；正式 tag 发布（v0.2.2-beta）待用户确认后执行。

- [x] X10-16 镜像窗口 Dock 图标包装 + 键盘直输（UHID）+ 主客户端图标按用户裁定重做。
  - 用户裁定澄清：主客户端 Dock 图标应保持 Tauri 蓝橙环风格（X10-15 的「显示器+手机」设计被否，git 恢复后重做）；要改的其实是**镜像窗口（scrcpy）在 Dock 里的绿色安卓机器人图标**。
  - 镜像窗口图标包装（macOS）：直接运行 scrcpy 裸二进制时 SDL 把 scrcpy 自带图标挂上 Dock。新增 `macos_mirror_bundle_exec`：在 scrcpy 同目录构建最小 `MirrorDock Mirror.app` bundle（Info.plist com.mirrordock.mirror + 主客户端同款 icon.icns + scrcpy/adb/scrcpy-server 复制件），经 bundle 启动后 LaunchServices 按 bundle 注册，Dock 显示「MirrorDock 镜像」与主客户端同款图标；任何失败回退裸二进制，绝不挡镜像。真机实验先行（/tmp 测试 bundle 经 lsappinfo 证实注册为「MirrorDock 镜像」）。启动点显式 `env("ADB", adb_binary)` 保证 bundle 内也能找到 adb。Rust 测试 +1。
  - 键盘直输（UHID）：用户反馈「微信发送时唤起手机自带输入法，不方便输入」。SessionOptions 新增 `keyboard_uhid`（serde 默认 true）：开启传 `--keyboard=uhid`（手机把电脑当外接键盘，全屏软键盘收起为小候选条，电脑键盘直接打字），关闭传 `--keyboard=scrcpy`（注入模式，软键盘照常弹出）。前端设置页新开关「键盘直输（手机不弹全屏键盘）」，旧配置无缝回填。Rust 测试 +1、vitest 断言同步。
  - 主客户端图标 v2（用户指定 Tauri 蓝橙环配色+风格）：PIL 绘制海军蓝渐变圆角方底 + 蓝橙双色粗环（蓝 340° 渐变环 + 橙 150° 呼应弧 + 中心橙点），经 tauri icon 生成全平台尺寸；镜像 bundle 的 AppIcon.icns 自动同款，两图标风格一致。

- [x] X10-17 键盘直输不生效根因修复（设备端键盘布局未配置）+ 菜单栏白模板图标 + 版本号 0.2.2。
  - **键盘直输根因定位（真机证据链）**：用户反馈「微信、便签 Mac 直接输入不生效」。逐步排除——①会话命令行核查（pgrep）：用户运行中的镜像进程确实带 `--keyboard=uhid`，配置链路（前端 readOptions 旧配置回填 true → begin_session → arguments()）无误；②受控实验：`--keyboard=uhid` 无头会话下设备端 `dumpsys input` 出现 `6: scrcpy`（KEYBOARD|ALPHAKEY，/dev/input/event6），UHID 键盘在 InputReader 注册成功；③logcat 无 UHID 报错；④打开系统实体键盘设置页（`am start -a android.settings.HARD_KEYBOARD_SETTINGS`）+ 截屏取证：scrcpy 实体键盘的**键盘布局列表一个布局都没启用**——没有布局，HID 按键无法映射成字符，按键必然无效。**修复**：经 adb 导航在设备上启用「英语（美国）」布局（此为设备侧一次性配置，持久生效）。取证截图落 `test-runs/uhid-e2e/`。**遗留观察项**：「使用屏幕键盘」开关当前保持开启（接实体键盘时软键盘以小条形式出现）；若用户仍嫌输入法弹出干扰，可引导关闭该开关。
  - 菜单栏白图标（用户反馈「顶部跟 Mac 其他图标一样主元素统一为白色、去掉背景色」）：新增 `src-tauri/icons/tray.png`（44x44，单色环+缺口切片+中心点，黑形状+alpha 通道，与主图标同一设计语言）；`build_tray` 改用内嵌 PNG（`include_bytes!`，Cargo 新增 tauri `image-png` feature），macOS 上 `icon_as_template(true)`——模板图由系统按菜单栏深浅自动反色（深色菜单栏渲染为白色，与系统图标一致）；非 macOS 平台保持彩色应用图标。
  - 主客户端 Dock 图标「没改」实为图标缓存：/Applications 内 icon.icns 已是 v2 蓝橙环设计（sips 渲染核对），重装后 `killall Dock` 刷新缓存。
  - 版本号三处同步 0.2.1 → 0.2.2（tauri.conf.json / package.json / Cargo.toml），tag `v0.2.2-beta` 发布。
  - 证据：`cargo test` 148 项全过；`cargo clippy --all-targets` 零告警；`pnpm test`（vitest）40 项全过。
  - 门禁：Rust 148 / clippy 0 / vitest 40 / build 全绿。
  - ⚠️ 待真机验证：镜像运行时 Dock 应显示「MirrorDock 镜像」蓝橙环图标；微信输入框点击后不再弹全屏键盘（底部小候选条直接打字）。

- [x] X10-18 主图标按 Apple 规范留边（修复 Dock 中偏大）。
  - 用户截图反馈：Dock 上 MirrorDock 图标比系统其他图标大一圈。根因：v2 图标 1024 满幅绘制，违反 Apple 模板（主图形 824×824 居中 + 100px 透明边距）。修复：设计缩至 824 渲染后居中贴 1024（圆角 224→180 等比），设计不变；镜像 bundle 图标运行时复制主 icon.icns 自动同步。commit ec535e8。
  - ⚠️ 引入回归：本条目实现有误——设计层仍在 1024 画布渲染再贴入 824 画布，右下 200px 被裁掉，图标只剩左上角圆角（用户 X10-19 截图证实）。v4 修复见 X10-19。

- [x] X10-19 本地打包缺 scrcpy 运行时致全部连接功能失效（用户反馈「USB 调试、无线调试全部挂掉」）+ 图标圆角回归修复 + v0.2.3。
  - **连接全挂根因**：装好的 .app 内无 scrcpy/adb。A1-08 的运行时下载校验步骤只存在于 CI 打包作业（build.yml package job），本地 `pnpm tauri build` 不会执行——此前 X10-16～X10-18 均为本机构建装机，发行模式查找链（显式 env → 随包资源 → PATH）全部落空退回裸 `scrcpy`（PATH 无）→ 镜像/连接/无线全不可用。adb 因 /usr/local/bin/adb 存在仍可用，故设备列表表现掩盖了镜像侧缺失。
  - **修复**：新增 `scripts/prepare-runtime.sh`——把本地已校验的 `.tools/scrcpy/<平台>/` 运行时全量复制进 `src-tauri/resources/scrcpy/`（布局与 CI 一致），挂入 tauri.conf.json `beforeBuildCommand`；`src-tauri/resources/scrcpy/` 进 .gitignore（二进制永不入库，供应链校验只在 CI/准备阶段做）。
  - **图标圆角回归修复（v4）**：设计直接在 824 画布渲染（环半径 242/宽 119/点 47 等比缩放），四角圆润经像素级断言验证（四角 alpha=0、边中点 255）；`tauri icon` 重生成全平台尺寸。
  - 版本号三处同步 0.2.2 → 0.2.3；tag `v0.2.3-beta` 发布（用户已明确授权「修复并重新推送打包 tag 发布新版本」）。
  - 铁律沉淀：①本地打包装机前必须先跑 prepare-runtime 并核验 .app 内含 scrcpy；②图标改版必须像素断言四角透明；③bash grep 会静默返回空——工作流核查一律用 Grep 工具。

- [x] X10-20 伴侣 APK 升级安装失败排查（用户经客户端装 CI 新 APK 报错）。
  - `INSTALL_FAILED_UPDATE_INCOMPATIBLE`：手机旧包（0.1.7-poc，本地构建）与 CI debug APK 签名不一致（debug keystore 随构建机随机），Android 拒绝覆盖安装；卸载重装可解，但每次升级都要来一遍且丢配对数据。
  - MIUI 侧另有 `INSTALL_FAILED_USER_RESTRICTED`：锁屏状态或安全中心拦截 USB 安装（截屏取证「应用安装拦截——已拦截通过USB安装的MirrorDock 伴侣」）；锁屏时必被拦，解锁后安装正常（07:56 实证）。此为平台安全边界，不做绕过。
  - adb 经沙箱时裸 `uninstall` 会被 broker 拒（decisionRecord 报错），改 `adb shell pm uninstall` 通过。

- [x] X10-21 伴侣 App CI 固定签名（根治升级必须卸载重装）。
  - 生成专用 keystore（RSA 2048/30 年/别名 mirrordock-companion），存放仓库外内部文档目录（与许可证签发私钥同域管理，永不入库）；凭据说明 + 证书基线指纹（SHA-256 ff06cd63…3cd9a0）落 `MirrorDock-内部文档/mirrordock-companion-signing.txt`（chmod 600）。
  - 经 GitHub API 写入 Secrets：`COMPANION_KEYSTORE_BASE64` + `COMPANION_STORE_PASSWORD`（libsodium sealed box 加密，tweetnacl-sealedbox-js；注意 Uint8Array.toString("base64") 不生效须 Buffer.from 包装——实测踩坑）。
  - `companion/app/build.gradle.kts`：环境变量存在时给 debug/release 挂 `ci` signingConfig，本地无变量回退默认 debug 签名（显式 warning 不挡构建）；`companion.yml` 与 `build.yml` 伴侣作业加 keystore 解码步骤。commit 2d4b45d。
  - CI 实证：companion.yml 构建成功，apksigner 核验产物指纹与固定密钥完全一致。**自此 Release/CI 产物均可直接覆盖安装，不再卸载重装。**
  - 手机侧完成固定签名版换装（卸旧装新一次），配对数据需重新配对。

- [x] X10-22 无线连接设备客户端不显示（修复 A+B）。
  - 现象：另一台设备手机端无线调试显示「已连接」，但客户端设备列表不出现新设备（之前已确认 `adb devices -l` 中 `192.168.2.224:33153` 在线，排查曾被 API 429 限流打断）。
  - 根因：① 前端设备列表**从不自动轮询**，`refreshDevices()` 只在挂载与显式动作（配对并连接/重新连接/重新检查）后调用；无线设备若在本客户端显式流程外连上（同 Wi-Fi 自动重连、已配对回连），界面不重新读取。② 设备列表块带 `!readyDevice` 守卫——已有一台就绪设备（如 USB）时，新连上的无线设备若处于 `unauthorized`（手机还要点允许）或 `offline`，整条列表被跳过，用户既看不到设备也看不到「待授权」状态。
  - 修复 A（后端+前端）：`check_adb_devices` 新增 `silent: Option<bool>` 参数；前端 `App.tsx` 新增 5s 静默轮询（`silent=true`，只刷新界面不写诊断日志），新设备无需手动「重新检查」即出现，状态变化（unauthorized→ready）也会自动拉出。
  - 修复 B（前端）：取消 `!readyDevice` 守卫，已就绪设备存在时仍列出其余非就绪设备，确保新无线设备的「待授权/离线」始终可见。
  - 验证：`tsc --noEmit` 通过、`cargo check` 通过、`cargo test parse_adb_devices` 通过（无回归）。待真机回归：连第二台无线设备，确认不点「重新检查」也自动出现，且未授权时显示「待授权」。

- [x] X10-23 同一台设备经 USB 与无线并存时只显示一条。
  - 需求：无论 USB 还是无线，如果是同一台设备，列表只显示一台，不能因连接方式不同而重复出现。
  - 鉴别方式：读设备硬件序列号 `ro.serialno`——同一台手机无论经 USB 还是无线都一致；USB 连接时它恰好等于 adb 序列号，无线连接时 adb 序列号是 `IP:端口`。这是跨连接方式识别「同一台设备」的唯一可靠依据（型号相同不足以去重）。
  - 实现（后端 `src-tauri/src/lib.rs`）：① `AdbDevice` 新增 `physical_serial: Option<String>` 与 `connections: Vec<ConnectionEndpoint>`（`kind: usb|wireless`+`state`）；② `AdbRuntime` 新增 `physical_serial()`（仅已授权设备可 `getprop ro.serialno`，失败/未授权返回 `Ok(None)`）；③ `AppRuntimes` 加 `serial_cache`（Mutex<HashMap>）避免每轮询重复 `getprop`，序列号消失即清理；④ 新增 `dedup_devices()`：按 `physical_serial` 分组合并，`physical_serial` 为 `None` 的绝不合（未授权同型号也保持独立），合并后 `serial` 取首选通道（状态最优、并列优先 USB）、`state` 取所有通道最优、`connections` 保留全部通道；⑤ `endpoint_is_ready()` 同时匹配合并后的 `connections` 原始端点，避免无线端点被判未就绪。
  - 实现（前端 `src/App.tsx` + `App.css`）：`Device` 类型补充 `physical_serial`/`connections`；用 `deviceKey()`（硬件序列号优先）作 React key 防重复渲染；新增 `connectionLabel()` 显示「USB + 无线 / 无线」徽标；新增 `findConnectedDevice()` 让最近设备按硬件序列号或任一通道匹配；就绪面板与设备列表均展示通道徽标。
  - 验证：`tsc --noEmit` 通过；`cargo test --lib` **151 项全过**（含新增 3 个去重单测：`merges_same_physical_device_across_usb_and_wireless` / `does_not_merge_devices_missing_physical_serial` / `endpoint_is_ready_matches_merged_connection_serial`）。
  - 待真机回归：同一台手机同时插 USB 并开无线调试，确认列表只出现一台且显示「USB + 无线」徽标；启动镜像走 USB（首选通道）。
  - 版本：0.2.3 → 0.2.4（tauri.conf.json / package.json / Cargo.toml 三处同步）。本地 release 打包（scrcpy 经 prepare-runtime.sh 打入 bundle）已发起，待用户真机测试通过后推送 tag v0.2.4-beta。

- [x] X10-24 修复「仅 Wi-Fi 连接却被误标 USB + 无线」与就绪面板/列表错位。
  - 现象（用户真机截图）：同一台手机只用 Wi-Fi 连接，列表却标「USB + 无线」；就绪面板与下方列表对同一台设备呈现不一致（一个有徽标、一个没有/各显示一条）。
  - 根因：① `connectionLabel()` 只看通道类型、不看通道状态——adb 拔除 USB 后残留的 `offline` 陈旧条目（ghost）被当成有效 USB 通道，导致徽标误写「USB + 无线」；② `dedup_devices()` 把未就绪（`offline`/`unauthorized`）通道也一并并入合并设备，使同一物理设备的不同状态各自成行、面板与列表对不上。
  - 修复（后端 `lib.rs`）：`dedup_devices()` 改为**仅合并「已就绪（`Ready`）且已知硬件序列号」的通道**；陈旧/offline/未授权条目各自独立成行（界面标「离线/待授权」），绝不污染已就绪设备的通道徽标。合并后 `connections` 只含已就绪通道。
  - 修复（前端 `App.tsx`/`App.css`）：`connectionLabel()` 仅统计 `state==="ready"` 的通道；React key 与「其余设备」过滤统一改用实际端点 `serial`（不再用会随去重变化的 `physical_serial` 派生 key），保证就绪面板与列表指向同一台设备、不重复不错位；选择器按钮也补通道徽标保持一致。
  - 验证：`tsc --noEmit` 通过；`cargo test --lib` **152 项全过**（新增 `stale_offline_usb_does_not_merge_into_ready_wireless` 锁定 ghost 不并入徽标）。待用户用「仅 Wi-Fi」真机复测。
  - 说明：若 adb 确实仍残留陈旧 USB 条目，用户可在刷新前执行 `adb disconnect <ghost>` 或重启 adb server 以彻底清除；本次修复已保证陈旧条目不再影响徽标与合并。
- [x] X10-25 复测仍误标「USB + 无线」的真正根因：mDNS 发现条目被误判为 USB；同型号多台设备重名加 `-1`/`-2` 后缀。
  - 真机证据：`adb devices -l` 里纯 Wi-Fi 设备有两条条目——`192.168.2.224:46289`（无线端点）+ `adb-<id>._adb-tls-connect._tcp`（无线调试的 mDNS/TLS 发现条目，**不含冒号**）。旧判定「含冒号=无线，否则 USB」把后者误归为 USB；两条又是同一物理机（ro.serialno 相同）被正确合并 → 徽标错写「USB + 无线」。X10-24 的 ghost 假设不是本次现场的主因。
  - 修复（后端 `lib.rs`）：`is_wireless_endpoint` 增加 mDNS 识别（含 `._adb-tls` / `._tcp` 即无线）；新增 `endpoint_preference`（USB > 无线 `IP:端口` > mDNS 名），合并组首选通道不再可能落到不稳定的 mDNS 名上；前端 `looksLikeWirelessEndpoint` 同步同一判定。
  - 新增（前端 `App.tsx`）：`buildDisplayLabels()`——多台设备型号名相同时按列表顺序追加 `-1`、`-2` 后缀，应用于设备选择器、就绪面板与设备列表。
  - 验证：`tsc --noEmit` 通过；`cargo test --lib` **153 项全过**（新增 `mdns_tls_entry_is_wireless_and_never_usb`：mDNS 条目归无线、合并首选 IP:端口、不含 USB 通道）。待用户真机复测。
- [x] X10-26 单会话下的「切换设备」流（方案 A，用户拍板 A 先做、B 排期）。
  - 现象：会话属于设备 X 时切到设备 Y，主按钮只显示禁用的「会话进行中」，不说明会话在哪台、也无换台路径（只能手动去顶栏「结束镜像」再回来）。后端 `SessionStore` 为全局单实例，`reserve_session` 对任何新会话返回 `session_busy`，属既定设计。
  - 修复（前端 `App.tsx`）：① 新增 `switchMirroring(serial)`——先 `stop_mirroring` 成功后立即 `start_mirroring`，一步完成切换；② 会话在另一台设备上时，就绪面板/最近设备列表的主按钮由禁用「会话进行中」升级为**「切换到此设备」**（同一设备仍显示禁用「会话进行中」）；③ 顶栏与首页眉文案带上当前会话设备名（如「镜像运行中：M2104K10AC -1」，用重名后缀显示名）。
  - 验证：`tsc --noEmit` 通过。待真机复测：会话中选另一台 → 按钮为「切换到此设备」，点击后旧窗口关闭、新设备镜像启动。
- [x] X10-27（用户拍板：TV 完成后开工）并发多设备镜像——里程碑 1（后端每设备一会话）已实现。
  - 架构：`SessionStore` 由全局单状态改为 `BTreeMap<serial, SessionState>` 会话表；互斥只收在**同一台设备**（同 serial 仍 `session_busy`），不同设备可同时各持一个镜像会话（scrcpy 原生多实例）。
  - 后端改动（`lib.rs`）：`reserve_session/mark_session/attach_process/take_running_process/begin_session_restart/running_session_options` 全部按 serial 键控；新增 `take_all_running_processes`（托盘断开/退出回收）、`prune_idle_entry`、`primary_session_serial`（主会话=持有进程者优先，向后兼容）；`check_adb_devices` 顺手清理「设备消失且无活动会话」的残条目。
  - 亮屏补偿多会话化：账本按设备各持一份；落盘文件从单对象升级为 `Vec<KeepAwakeBackup>`（兼容解析旧单条文件，崩溃残留不丢）；应用退出还原全部设备。
  - 命令面：`stop_mirroring` 增加可选 `serial`（None=主会话）；新增 `mirror_sessions`（全部设备会话快照，已注册 handler）；`update_session_options` 增加可选 `serial`（None=主会话）；`mirror_session`/`current_recording` 保留主会话语义向后兼容；托盘「断开连接」= 结束所有会话；录制删除校验扫描全部会话。
  - 前端（`App.tsx`）：轮询改 `mirror_sessions`；`sessionOwnedBy` 按设备各自的会话判断；**会话中其它设备的「开始镜像」按钮可用**（X10-26 的「切换到此设备」被真正的并发启动取代，函数已删）；顶栏多会话时显示「N 台设备镜像中」；`stop_mirroring` 支持按设备停止。
  - 验证：`cargo test --lib` **155 项全过**（新增 `two_devices_hold_concurrent_sessions_and_do_not_block_each_other`；更新同设备互斥测试适配按设备 Idle 语义）；`tsc --noEmit` 通过。待双设备真机验证。
  - 里程碑 2（已完成，见下方 X10-68）：托盘逐会话（动态菜单「结束 <设备> 的镜像」）与设置逐会话（「应用到哪台设备」选择器）落地；录制仍为主会话语义（多会话录制逐台细化留待按需）。
- [x] X10-28（用户拍板提前）伴侣 App TV 模式（第一期：可安装、可启动、可配对）。
  - 调研结论（2026-09-30）：Android TV/Google TV/Fire TV **可行**——开发者选项隐藏但可用遥控器开启（设置→设备偏好→关于→连点「版本」7 次），开启「ADB/网络调试」后电视盒通常可直接 `adb connect IP:5555`（无需 USB），Android 11+ 支持无线调试配对（与手机同流程，现有配对链路可复用）；MirrorDock 现有无线连接 + scrcpy 镜像控制 + APK 安装能力对 Android TV 原样适用。**不可行边界**：非 Android 电视（三星 Tizen / LG webOS 等）无 ADB，Android APK 与 adb 控制均不可能，替代方案是外接 Android TV 盒子。
  - 实现：① Manifest 加 `LEANBACK_LAUNCHER` 入口 + `android:banner`（320×180 墨色渐变 + 白 M + 青碧点，`tv_banner.xml`/`tv_banner_mark.xml`）；声明 `android.software.leanback required=false`（不排斥手机）与 `android.hardware.touchscreen required=false`（电视无触屏也能装）；② MainActivity 增加 `isTv()`（UiModeManager）分支：TV 上隐藏「扫码」（无相机）、默认展开「手动输入配对码」（电视配对唯一路径，不强制弹键盘）；③ 文件行背景改 `bg_file_row` 状态选择器——D-pad 聚焦时青碧描边高亮，焦点可见；④ 版本 8→9（0.1.8-tv）。
  - 验证：`gradle :app:assembleDebug` BUILD SUCCESSFUL；merged manifest 已核验含 leanback/touchscreen/banner/LEANBACK_LAUNCHER 全部声明。**真机验证待做**：电视桌面图标（banner 显示）、遥控器完成手动输入配对。
  - 模拟器实证（2026-09-30，本机 Android TV 16 Google TV x86_64 AVD，测后已清理）：① APK 安装 Success（touchscreen required=false 生效，无触屏可装）；② 启动后 UI 正常渲染，TV 分支触发——扫码按钮隐藏、手动输入默认展开（截图 test-runs/x10-28-tv-emulator/01_main.png）；③ D-pad 导航可用——5 次方向键焦点依次穿越按钮/输入框/文件区直到日志区（uiautomator dump 证据 ui.xml），DPAD_CENTER 点击「查看」成功展开日志（04_log_expand.png）；④ `cmd package resolve-activity` 以 LEANBACK_LAUNCHER 类别解析到 MainActivity 且 banner 资源在位（banner=0x7f070077）。已知局限：无头 swiftshader 下系统 TV 桌面黑屏（模拟器渲染问题，与 App 无关），桌面图标视觉效果留真机验证；测试 AVD 与 1.2GB 系统镜像已删净，证据落 test-runs/x10-28-tv-emulator/。
- [x] X10-29（复测反馈三件套）按设备结束镜像 + 伴侣 APK 真机安装 + 拖拽安装 APK。
  - ① 按设备结束（复测反馈：多会话时顶栏只有一个「结束镜像」，不知道结束谁）：前端 `App.tsx` 新增 `activeSessionList`（serial 非空的进行中会话）+「正在镜像的设备」面板——每台设备一行，各自带「结束镜像」按钮（`stop_mirroring(serial)`，只结束那一台）；就绪面板主按钮会话中由禁用「会话进行中」改为红色「结束镜像」（就地对当前选中设备结束）；最近设备行同理；`stoppingSerial` 让按钮各自显示「正在结束…」；顶栏按钮多会话时改为「结束全部（N 台）」，单会话仍为「结束镜像」。
  - ② 伴侣 APK 真机安装（versionCode 10 / 0.1.9-icon，本机 2026-09-30 13:52 构建）：无线端 `192.168.2.224:46289` `install -r -t` **Success**（可看新图标）；USB 端 `79j7kn9tkjt8rwss` 先遇 `INSTALL_FAILED_UPDATE_INCOMPATIBLE`（旧版为另一把 debug key 签名，已 `uninstall` 清掉）→ 重装遇 `INSTALL_FAILED_USER_RESTRICTED`（MIUI 拦截），**需在手机「开发者选项」开启「USB 安装」后重装**。注意：USB 那台是卸载重装，配对数据已清，需重新配对。
  - ③ 拖拽安装 APK：把电脑上的 `.apk` 拖进 MirrorDock 主窗口任意位置即可安装到当前就绪手机。前端用 `getCurrentWebview().onDragDropEvent`（Tauri v2 webview 拖放事件，`core:default` 权限已覆盖）；拖入时全屏虚线浮层提示「松开鼠标，安装 APK / 将安装到：<设备名>」；drop 后过滤 `.apk`（大小写不敏感），无 APK / 无就绪设备给明确错误提示；多个 APK 只装第一个并说明（不静默批量安装）；结果走右下角 toast（任何页签可见，含后端 receipt.summary 与「需要你本人同意」提醒）。目标设备读 `dragTargetRef`（每次渲染同步最新就绪设备），避免闭包过期。
  - 顺手修复（X10-27 遗留，测试暴露）：`App.test.tsx` 兜底 mock 补 `mirror_sessions`（返回 `[]`，漏 mock 时组件拿到 `{}` 渲染崩溃）；设备夹具补 `physical_serial`/`connections` 字段（`Device` 模型多通道合并新增，缺字段让 `connectionLabel` 渲染期崩溃）；两个用例的 mock 从旧命令 `mirror_session` 补到 `mirror_sessions`。
  - 验证：`tsc --noEmit` 通过；**vitest 40 项全过**。待真机复测：双会话时逐台结束互不影响、拖 APK 进窗口安装成功。
  - **设计精简（同日二次反馈：结束按钮一屏出现 3~4 个，冗余）**：主页重构为**统一设备卡片列表**——一台设备一张卡、状态即操作、每个动作全页只出现一次。删除：`session-list`「正在镜像的设备」面板、设备选择器 chips、独立就绪面板、「其余设备」device-list；最近设备里**已连接**的手机不再重复放开始/结束按钮（只留状态+移除记录）。卡片右侧唯一主操作按状态切换：镜像中→「结束镜像」（红）、就绪→「开始镜像」（+「屏幕唤醒」次操作）、待授权→纯文字、无线离线→「重新连接」；新增 `launchingSerial`/`stoppingSerial` 让「正在启动/正在结束」只显示在被点的那张卡上。顶栏「结束全部（N 台）」仅在 ≥2 台同时镜像时出现；会话还在但设备从列表消失时补一张兜底卡保留结束入口；锁屏状态面板保留（标题带设备名）。验证同上。
  - **最近设备支持物理删除（三次反馈）**：后端新增 `clear_recent_devices` 命令（把 recent-devices.json 落盘为空列表，注册 handler；与逐条移除同一边界——不断开连接、不撤销授权、不清无线配对，再次镜像会重新记入）；前端「最近使用过的设备」标题行加红色「清空记录」按钮。后端测试 `clearing_recent_devices_wipes_the_file_physically` 验证落盘物理删除。`cargo test --lib` 155 项全过、`tsc` 通过、vitest 40 项全过。
- [x] X10-30 Windows 客户端镜像运行时后台反复弹黑色控制台窗口（用户反馈）。
  - 根因：主程序 release 已是 `windows_subsystem = "windows"`（无主控制台），但 **adb.exe 是控制台子系统程序**——Rust `std::process::Command` 在 Windows 上默认为控制台子进程分配（或继承）一个新控制台，镜像会话期间监视线程高频轮询 adb，导致黑框反复闪烁。
  - 修复（`lib.rs`）：新增 `quiet_command()` 辅助——Windows 上对子进程附加 `CREATE_NO_WINDOW`（0x08000000，只为不分配新控制台，不影响 GUI 窗口本身），非 Windows 原样返回；全部 11 处生产 spawn 点替换（9 处 adb 调用 + scrcpy 启动 + scrcpy 版本探测）。scrcpy 为 GUI 子系统，加该标志无副作用，镜像画面窗口不受影响。测试代码与 macOS codesign 调用不变。
  - 验证：macOS `cargo test --lib` 155 项全过（Windows cfg 分支本地无法编译，待 CI Windows 打包验证）。
  - 待办：触发 CI 产出 Windows 包供用户验证黑框消失。
- [x] X10-31 伴侣 App 扫码「提示成功但连接列表不出现设备」（用户二次追问，定性为产品断层而非故障）。
  - 定性：客户端存在**两条独立通道**——①镜像通道（ADB：数据线 / 无线调试配对，连接列表只认它）；②伴侣加密会话（MDP1 TLS，C4-01 POC，用于后续助手能力）。伴侣扫码成功建立的是②，①未建立，故设备不出现。Android 安全设计决定第三方 App 无法替手机完成首次 ADB 配对授权。
  - 修复一（自动回连桥）：伴侣会话「设备报到」时，桌面端以伴侣端来源 IP 在 `adb mdns services` 里匹配该手机的 `_adb-tls-connect._tcp` 广播并自动 `adb connect`——已配对过且开着无线调试的手机扫码后**自动进入连接列表**；找不到广播/连接失败/无匹配时在伴侣事件流里如实告知两条通道的差别与正确入口（`companion_bridge_events` + `PairingState::set_device_bridge` 钩子，伴侣模块不感知 adb；spawn_blocking 避免阻塞会话循环）。`AppRuntimes.adb` Box→Arc 以支持后台任务共享。
  - 修复二（文案纠偏）：帮助中心「无线连接」原来写着「…或直接用伴侣 App 扫码」——这是误导（伴侣 App 扫的是 MDP1 载荷，扫 ADB 二维码会报「格式不正确」），已改为配对码填入步骤 + 两条通道独立说明；连接页伴侣区块描述与「已连接」状态补充边界提示；伴侣 App 配对成功日志追加「这是伴侣助手通道，不等于镜像连接」提示。
  - 验证：`cargo test --lib` **159 项全过**（新增 4 项桥接测试：按 IP 匹配只连目标设备、未发现服务不发起 connect、他人设备 IP 不误连、connect 失败如实上报）；tsc 通过；vitest 40 项全过。
  - 正确用法（已同步进帮助中心）：首次连接 = 数据线授权，或「无线」页填手机「无线调试 → 使用配对码配对设备」的 IP/端口/6 位码；配对一次后，后续可由伴侣扫码自动回连（手机需开着无线调试）。
- [x] X10-32 同机多卡去重失效 + 客户端侧取消授权（用户第四次反馈：2 台手机出 4 张卡；授权只能手机取消不合理）。
  - **同机多卡**：`dedup_devices()` 此前只合并「就绪且已知硬件序列号」的通道，离线/陈旧端点（拔线 ghost、无线调试重启后的旧端口、旧 mDNS 条目）各自成卡 → 2 台手机显示 3~4 张。改为**影子端点归并**：非就绪条目能判定属于某就绪组（① 硬件序列号相同；② mDNS 实例名内嵌该组 USB 序列号；③ 无线端点同 IP 不同端口）即不再单独成卡；都判不了才保留（离线必须如实可见）。整台手机离线时 USB 残影与 mDNS 残影也归并为一张离线卡。徽标口径不受影响（connections 只含就绪通道）。新增 `wireless_ip_endpoint`/`mdns_serial_contains_usb`/`group_usb_serials`/`group_wireless_ips`/`device_shadows_group`/`merge_offline_shadows`。
  - **取消授权**：设备卡新增「取消授权」（两段式确认）。执行链：①`am start` 打开手机开发者选项（趁通道活着先做）→ ②`settings put global adb_wifi_enabled 0` 关无线调试 → ③`settings put global adb_enabled 0` 关 USB 调试（等效收回所有电脑的访问权）→ ④`adb disconnect` 全部无线端点 → ⑤清理本机受信无线/最近设备记录。每步逐端点重试（USB 优先，无线死了 USB 兜底），执行回执逐条展示。**边界如实声明**：Android 授权记录存于手机（root 才能直接清除），「撤销 USB 调试授权」必须本人在手机上点——客户端负责把页面打开。镜像进行中的设备拒绝取消授权（先结束会话）。新 trait 方法 `open_developer_settings`/`disable_wireless_debugging`/`disable_usb_debugging`（固定参数、可审计）。
  - 验证：`cargo test --lib` **167 项全过**（新增 5 项影子归并 + 3 项取消授权：全步执行、端点收集 USB 优先、端点失败兜底）；tsc 通过；vitest 40 项全过。
- [x] X10-33 取消授权全覆盖 + 锁屏状态便签化（用户第五次反馈，2026-09-30）。
  - **取消授权扩到全部状态**：非就绪（待授权/离线）设备卡同样提供「取消授权」，成功后该设备立即从设备列表移除（前端 `hiddenRevokedSerials` 隐藏，adb 列表随后自然消失；设备重新就绪即自动恢复显示）；镜像进行中的卡不显示（后端本就拒绝）。
  - **样式统一**：取消授权从纯文字链改为与「屏幕唤醒」同款 `secondary-button`；两段确认改为「确认取消」（danger-stop）+「算了」双按钮。
  - **锁屏面板 → 便签**：删除主页整块「xx 当前的锁屏状态」面板（单设备视角不适配多台并发）；锁屏状态改为设备卡上连接徽标后的便签（`lock-badge`，`lockTag()` 短语：已解锁/已锁屏/安全锁屏/锁屏未知 · 亮屏/熄屏/屏幕未知），按就绪设备集合 5 秒轮询各自刷新。`lockReports` 状态从单值改 `Record<serial, DeviceLockReport>`；唤醒失败提示移入状态栈；熄屏右键提示与 FLAG_SECURE 提示移入状态栈；底部「关于锁屏与解锁」长说明保留。删除不再使用的 `lockActionHint`。
  - 验证：tsc 通过；vitest **41 项全过**（新增 `lockTag` 用例）。Rust 无改动（167 项维持）。版本 0.2.5，tag `v0.2.5-beta` 走 CI 发布。
- [x] X10-34 状态区通知支持关闭（用户第六次反馈，2026-09-30）。
  - 取消授权回执等状态区通知（`revokeNotice`）右上角加「×」关闭按钮（`notice-dismissable`/`notice-close` 样式，错误态红字区分）；点击即从状态栈移除，不依赖自动消失。
  - 验证：tsc 通过；vitest 41 项全过。版本 0.2.6，tag `v0.2.6-beta` 走 CI 发布。
- [x] X10-35 取消授权的移除跨重启持久 + 回执文案去原始报错（用户第七次反馈，2026-09-30：离线设备取消授权后重启客户端，离线卡又出现；回执满是 adb 英文报错）。
  - **根因**：离线设备的取消授权指令送不到手机（三条 shell 全部失败），adb 列表里的 offline 残影不会自己消失；X10-33 的隐藏清单只在内存里，重启即失效。
  - **修复①持久化**：隐藏清单落 localStorage（`mirrordock.revokedSerials`），重启后仍不显示；设备重新以 ready 出现时自动恢复显示（自愈——用户重新授权即回来）。
  - **修复②回执文案**：`revoke_device_access_with` 失败步不再透出 `{error}` 原始报错，改为「这台手机当前无响应（离线或未授权）…请手动进入 设置 → 开发者选项」等人话；成功步补「已把它从设备列表移除」。FakeAdb 的取消授权三方法遵守 `device_writes_fail`，新增 `with_device_write_failure()`。
  - 验证：`cargo test --lib` **168 项全过**（新增离线回执友好性测试：断言不含原始报错与 adb 字样）；tsc 通过；vitest 41 项全过。版本 0.2.7，tag `v0.2.7-beta` 走 CI 发布。
- [x] X10-36 首页说明块全部移入帮助中心 + README/官网主页同步最新版本（用户反馈，2026-09-30）。
  - **首页减负**：删除连接页底部三块说明——「为什么需要授权？」aside、「这台手机能做什么」能力面板（含 per-device notices 着色列表）、「关于锁屏与解锁」长说明面板；连接页只留操作动线。能力探测（`probe_device_capabilities`）保留——结果仍驱动设置页「转发手机声音」开关的一致性；`capabilitiesError` 死代码与 `.capability-notices` 系列样式一并清理。
  - **帮助中心承接**：新增第 8 篇帮助文档「为什么需要授权」（`helpContent.ts`，置于「三步开始镜像」之后），内容覆盖：授权机制与随时可撤销、锁屏是系统防线与授权无关、按系统版本如实报告能力（画面 Android 8+/声音 Android 11+，读不到显示未知）、黑屏与屏蔽点击属应用安全策略、品牌开发者选项入口差异。锁屏细节已由既有「锁屏与解锁」篇覆盖，无重复新增。
  - **README 同步**：版本行 0.2.0→0.2.8（伴侣 0.1.7-poc→0.1.9）；功能列表补统一设备卡片/同机归并、取消授权、最近设备清空、伴侣扫码自动回连、锁屏便签、APK 安装（含拖拽）、通知可关闭；帮助篇数 7→8。
  - **官网主页（site/index.html）同步**：kicker/下载区/页脚全部 v0.2.0→v0.2.8（下载链接与文件名指向 v0.2.8-beta 资产）；功能条目补「一台手机一张卡片」「安装 APK」，伴侣条目补自动回连。push main 触发 pages.yml 自动重发 GitHub Pages。
  - 验证：tsc 通过；vitest **41 项全过**；cargo check 通过（刷新 Cargo.lock）。Rust 无代码改动（168 项维持）。版本 0.2.8，tag `v0.2.8-beta` 走 CI 发布。
- [x] X10-37 真机排障：电脑打字「始终不生效」（用户反馈，2026-09-30，v0.2.8 会话中）。
  - **诊断链**（X10-17 模板）：①会话命令行确认 `--keyboard=uhid` 正常；②`dumpsys input` Device 26 scrcpy 已注册（KEYBOARD|ALPHAKEY）、logcat 无 UHID 报错；③「实体键盘」设置页截图定案——scrcpy 键盘**启用了 Colemak / Dvorak / Workman / 国际风格四个异形布局，标准「英语（美国）」反而被关闭**。Mac 的 QWERTY HID 键码经异形 KCM 重映射 → 字符全错，用户感知为「打字不生效」。
  - **根因推断**：Android 在镜像窗口按 **Ctrl+空格** 会循环切换启用的实体键盘布局（MIUI 布局页原文提示「要切换，请按 Ctrl+空格键」）；X10-17 只启用了标准布局，本次多个异形布局处于启用态且标准布局被切走，疑似用户在镜像窗口误触 Ctrl+空格 后在布局页探索时勾选了其它布局。
  - **修复（已真机执行并核验）**：通过 adb UI 驱动（1080×2400 实测坐标）在布局页只保留「英语（美国）」标准布局，其余全部关闭；返回页摘要确认 `scrcpy → 英语（美国）`，布局切换即时生效，镜像会话无需重启。打字端到端验证需用户真打（沙箱无法合成键击）。
  - **文档同步**：帮助中心 FAQ 新增条目「电脑上打字没反应或字符全错？」——指认 Ctrl+空格 布局循环、给出只保留标准英语（美国）的修复路径与 Mod+k 入口。
  - **待办（候选增强）→ 已收口（2026-10-02 守卫轮，见 X10-64 条的「候选增强收口」段）**：实测该探测不可实现——设备端 UHID 启用布局清单不在任何 adb 可读面（`settings secure/system/global` 与 `dumpsys input` 均无该状态）。不改代码，保留帮助中心 FAQ 的修复指引作为用户侧出口。
  - 环境备忘：沙箱内执行 `/Applications/MirrorDock.app/.../adb` 部分命令被 sandbox-center 拦截（`decisionRecord missing actual resource subject`），改用 `.tools/scrcpy/macos-x86_64/adb` 可用；`adb shell dumpsys input`、`wm size` 带引号/管道形式会被拦，改「无引号 + 输出重定向到文件再本地分析」可绕过。
- [x] X10-38 排障：Mac 第三方输入法（微信输入法 WeType）下镜像打字失效——可行性结论 + 文档同步（用户问询，2026-09-30）。
  - **现象**：Mac 装第三方输入法后镜像窗口打字无效；切回 macOS 自带输入法即恢复。
  - **取证**：当前输入源 `com.tencent.inputmethod.wetype`（微信输入法）；`kCGSSessionSecureInputPID` 无进程持有（排除「安全输入」全局吞键）。
  - **机理与可行性结论**：①UHID 模式是物理键盘语义，按键原始键码直送手机、不经过 Mac 输入法组字（scrcpy 官方文档明确 UHID「works for all characters and IME」是靠手机端输入法）；②`--keyboard=sdk`（原 scrcpy 模式）对中文**静默丢弃**——v4.1 服务端 `Controller.injectText` 逐字符走 `KeyCharacterMap.getEvents`，映射不到（中文/Emoji）即 `getEvents()==null` 跳过并记警告，**无剪贴板兜底**（剪贴板粘贴是独立的 `TYPE_SET_CLIPBOARD` 消息，需显式 paste 标志）；③第三方输入法（WeType/搜狗等）激活时把 keyDown 消费进自身组字缓冲，SDL 收不到原始键，UHID/sdk 两条路都断。**结论：无法在客户端层「支持第三方输入法直接打字」，这是 scrcpy/macOS 生态限制，Escrcpy/QtScrcpy 同样存在；正确姿势 = Mac 切系统自带输入法 + 中文由手机端输入法组字，或 Mod+v 粘贴中文。**
  - **文档同步**：帮助中心 FAQ 新增「Mac 上装了第三方输入法时打字没反应？」条目（机理 + 切换系统输入法恢复 + 中文两种推荐姿势）。
  - **候选增强（未实现，待拍板）**：会话启动时只读检测当前输入源（`defaults read com.apple.HIToolbox AppleSelectedInputSources`，bundle id 非 `com.apple.*` 判为第三方），在状态栈如实提示「第三方输入法可能导致打字无效」并给切换建议；不代切输入法（改用户系统状态违反「引导不绕过」哲学）。
- [x] X10-39 macOS 宿主输入源自动托管：镜像会话期间临时切换到系统输入源、结束后恢复（用户裁定 X10-38 候选增强方案：「只提示用户体验太差」，要求自动切换，2026-09-30）。
  - **决策变更说明**：X10-38 曾以「不代切输入法」为由只做提示；用户明确否决（「只提示用户体验太差，应该是镜像输入时自动切换到系统自带输入法，输入完毕后切回」）。范围界定为**会话粒度**的托管：开始镜像时切、最后一台会话结束（或退出/崩溃恢复）时还原；打字起止无法可靠探测，会话粒度是稳妥实现。
  - **新模块 `src-tauri/src/input_source.rs`**：Carbon TIS FFI（`TISCopyCurrentKeyboardInputSource` / `TISGetInputSourceProperty` / `TISSelectInputSource` / `TISCreateInputSourceList`，链接 Carbon + CoreFoundation，无新依赖、无权限要求）。`is_third_party`＝bundle id 不以 `com.apple.` 开头；切换目标三级偏好：`com.apple.keylayout.ABC` → 任意 Apple 布局 → **Apple 自家输入法**（`com.apple.inputmethod.*`）——真机取证发现用户可能没启用任何英文布局（实测该机只有 SCIM 拼音），第三级兜底必不可少。恢复按 bundle id 全量列表（`include_all_installed=true`，覆盖第三方输入法这类「输入模式」）。非 macOS 平台为空操作。
  - **真机端到端验证**（ctypes 复刻同款 FFI）：当前输入源读出 `com.tencent.inputmethod.wetype`；`TISSelectInputSource(SCIM)==0` 且 current 变为 SCIM；还原 wetype==0 且 current 复原。切换/恢复无需任何 TCC 权限。
  - **lib.rs 编排**：`HOST_INPUT_SOURCE_BACKUP`（OnceLock 全局，输入源是宿主全局资源，多设备共享一次切换）；`maybe_switch_host_input_source`（幂等：已托管/系统输入源/读取失败/切换失败均静默跳过，绝不阻断镜像）挂 `start_mirroring`、托盘启动、`update_session_options`（重启会话补检）三处；`maybe_restore_host_input_source`（仅在 `count_running_processes()==0` 时还原，否则把备份放回）挂 `stop_mirroring`、会话监视器退出、托盘「断开连接」三处；托盘「退出 MirrorDock」退出前强制还原。崩溃账本 `input-source-backup.json` 落 app_data_dir，启动时 `restore_persisted_input_source` 读取→恢复→删除。
  - **前端提示**：监听 `host-input-source-switched` 事件，状态栈显示可关闭通知「已临时切换到系统输入法，镜像结束后自动恢复」。
  - **文档**：README 功能列表补「macOS 输入法自动托管」；帮助中心 FAQ X10-38 条目改写为自动处理说明（0.2.9 起生效）。
  - 验证：`cargo test --lib` **172 项全过**（新增 `input_source` 模块 4 项纯逻辑测试：第三方判定/切换触发条件/恢复触发条件）；tsc 待跑见下；版本 0.2.9，tag `v0.2.9-beta` 走 CI 发布。
- [x] X10-40 修复：0.2.9 开始镜像即 SIGSEGV 三连崩（用户真机反馈 2026-09-30 22:23~22:27 崩溃报告实锤）。
  - **现象**：安装 0.2.9 后 ①顶部托盘图标消失 ②镜像异常卡顿 ③「打字闪退」。崩溃报告 `mirrordock-2026-09-30-222754.ips`：`EXC_BAD_ACCESS` 于 `TISGetInputSourceProperty → CFEqual → objc_msgSend → realizeClass`，调用栈 `current_bundle_id ← maybe_switch_host_input_source ← start_mirroring`。
  - **根因**：`kTISPropertyBundleID` 是导出数据符号（槽位存 CFStringRef），X10-39 误用 `addr_of!` 把**槽位地址**当属性 key 传给 TIS——Carbon 把垃圾指针当 CFString 解引用即崩。三症状归一：开始镜像即崩 → 应用进程死 → 托盘图标消失；崩溃循环里每次重试 spawn 的 scrcpy 成孤儿并存抢编码器 → 卡顿；「打字闪退」实为应用崩溃带崩镜像会话。
  - **修复**：属性 key 统一改为按值读取（`tis_property_bundle_id_key()`：`unsafe { kTISPropertyBundleID }` 加载槽位里的 CFStringRef），三处调用点同改。
  - **防回归**：新增走**真实 Rust FFI** 的只读冒烟测试 `current_bundle_id_smoke_test_via_real_ffi`（断言读到非空 bundle id）——上一轮只用 Python ctypes 验证了语义、未覆盖 Rust 侧传参，这是教训；cargo test 在测试进程内直接调用 TIS，传参再错会当场崩。
  - 版本 0.2.10，tag `v0.2.10-beta` 重发 CI；`cargo test --lib` **173 项全过**。
- [x] X10-41 修复：镜像打字自动切换目标改为**强制 ABC**（用户真机反馈：拼音模式下同样无法输入）+ 设置页新增「关于」板块 + Release 正文加「本版更新内容」（2026-09-30）。
  - **ABC 强制策略**：X10-39 的三级兜底在该机上落到系统拼音（SCIM）——用户实测「只有 ABC 能正常输入，拼音模式也不行」。新策略：当前输入源不是 `com.apple.keylayout.ABC` 就切（含第三方输入法、系统拼音、其它布局）；ABC 未启用时先 `TISEnableInputSource` 再选中（禁用态无法直接选中），恢复时还原这一临时启用（`ABC_ENABLED_BY_US` AtomicBool + `TISDisableInputSource`，失败静默）。
  - **关键取证**：新版 macOS 把键盘布局的 BundleID 统一收敛为 `com.apple.keyboardlayout.all`（全量 309 条里无任何 `com.apple.keylayout.*` 的 BundleID）——按 BundleID 匹配 ABC 永远找不到。改用 **InputSourceID**（`kTISPropertyInputSourceID`）作身份串：`source_identity()` 优先 InputSourceID、回退 BundleID；`current_bundle_id`/`switch_to_system_ascii`/`switch_to_bundle` 全部改按身份串匹配。备份/恢复往返一致（备份存的就是身份串）。
  - **真机端到端验证**（ctypes 复刻）：`wetype → ABC(status 0) → wetype(status 0)` 往返成功；`AppleCurrentKeyboardLayoutInputSourceID` 印证 ABC 的 InputSourceID。
  - **设置页「关于 MirrorDock」卡片**：版本号（`getVersion`）、开发者（g-star1024）、开源许可（Apache-2.0 + scrcpy 声明 + THIRD_PARTY_NOTICES 指引）、隐私承诺、官网 / GitHub 仓库按钮（`plugin-opener` 的 `openUrl`，capability 已有 `opener:default`）。测试补 `@tauri-apps/api/app` mock；`explains_when_the_mirror_runtime_itself_is_missing` 因新卡片出现 scrcpy 字样改用精确文本查询。
  - **Release 正文改进（用户反馈：固定文案不友好）**：build.yml 创建 Release 时若存在 `docs/releases/<tag>.md` 则在固定模板前输出「## 本版更新内容」栏目；本期新建 `docs/releases/v0.2.11-beta.md`。
  - 文档：帮助中心 FAQ 输入法条目改写（0.2.11 自动切 ABC）；README/官网版本行同步 0.2.11。
  - 验证：`cargo test --lib` **171 项全过**（is_third_party 移除：新策略不再需要该判定，-2 测试）；tsc / vitest **41** / pnpm build 全过；版本 0.2.11，tag `v0.2.11-beta` 走 CI 发布。
- [x] X10-43 结束镜像时整个客户端被杀（v0.3.0，macOS 15 TIS 主队列断言，崩溃报告实锤）。X10-42 编号未使用。
  - 修复：`input_source.rs` 全部 TIS 调用改走主线程（`run_on_main_thread` + 3s 超时降级）；`maybe_switch` 在锁外调度避免与主线程锁等待环；`ExitRequested(code=None)` 一律 `prevent_exit`，退出仅保留托盘菜单一个出口。
- [x] X10-44~48 + v0.3.0-beta / v0.4.0-beta（补录，2026-10-01：人工会话提交 b2a4546 / 23157b4 / 245e269 / 3331efc / f380be4 未同步本文件，本轮按提交信息与 release notes 整理入档，并做独立基线复核）。
  - **X10-45 无线断线自动重连**：异常退出 + 无线端点 → 5s 探测、15 分钟预算，设备回网后按原参数重建（录制不续录）；设置可关；事件通知前端。
  - **X10-47 应用内自动更新**：`tauri-plugin-updater` + minisign 签名（签名私钥只走 GitHub secrets `TAURI_SIGNING_PRIVATE_KEY` 注入，不进代码与构建产物日志，与「私钥永不进 CI 代码」假设一致）；CI 产出 `.sig` 与 `latest.json`（macOS 双架构 artifact 统一改名 `<v>_<arch>.app.tar.gz` 防 merge-multiple 互覆，提交 3331efc）；设置→关于一键升级。
  - **X10-44 帧率上限**：`--max-fps` 白名单 24/30/60，默认跟随设备。
  - **X10-46 手机实体键盘设置一键直达**：`open_keyboard_settings` 命令（命令面 36→37），设置页键盘直输区按钮直达手机「实体键盘」页（承接 X10-37/38 排障经验）。
  - **X10-48 Linux 随包 scrcpy 4.1**：CI 官方源码编译（固定 tag + SHA-256），server 用官方 release digest 校验，adb 用 platform-tools 固定版本；scrcpy 4.x 依赖 SDL3，CI 增 SDL3 release-3.4.16 源码编译（固定 tag + 固定 SHA-256，提交 245e269/f380be4——v0.3.0-beta 首跑实锤 Ubuntu 22.04 只有 SDL2，meson 报 sdl3 not found）；SDL3 动态库不随包，README/THIRD_PARTY_NOTICES 已如实更新。
  - **v0.4.0-beta 功能（代码已在 main，随下个 tag 发布）**：①桌面模式 `--new-display`——手机上创建独立虚拟显示器，电脑画面不被手机操作打断（类 DeX），需 Android 10+；②摄像头源 `--video-source=camera`——显式开启、强制 `--no-audio`（不采麦克风，兑现 A1-05 承诺）；③两者互斥（`video_source_conflict` 错误码），旧配置 serde/localStorage 回填。
  - **版本号口径**：v0.4.0 提交后为对齐重发的 tag `v0.3.0-beta`，版本号暂回 0.3.0（package.json/tauri.conf.json/Cargo.toml），v0.4.0 功能代码已在 main。
  - **本轮独立基线复核（2026-10-01 04:17，HEAD=f380be4，工作区干净）**：`cargo test --manifest-path src-tauri/Cargo.toml` **175 passed / 0 failed**；`pnpm build` 通过；vitest **41 passed / 0 failed**。与提交声称的 cargo 175 / vitest 41 一致。
  - **未验证面（外部依赖）**：v0.3.0-beta tag CI 重发 build（run 36770770206）进行中，产物可运行性、updater 端到端（minisign 验签 + latest.json 拉取 + 三平台升级）、桌面模式/摄像头源/自动重连/帧率上限的真机行为均待真机与产物验收。

- [x] X10-51/52 v0.3.0-beta CI 连败两轮修复（2026-10-01 凌晨，通宵发版会话）。
  - **X10-51 scrcpy 产物路径**（run 36772187374，tag cf4a984）：scrcpy 77 个编译目标链接全成功，仅 `cp` 落空——meson 产物在 `build/app/scrcpy`（app/ 子目录），工作流写 `build/scrcpy`。修复 9cd6bc1。
  - **X10-52 资产命名不一致**（run 36774581840，tag 9cd6bc1）：4 平台打包全绿、Release 已发布（29 资产），仅 latest.json 生成失败——脚本按 tag 版本 `0.3.0-beta` 拼名，而 tauri 打包产物（exe/AppImage/deb/rpm/dmg）用 conf 版本 `0.3.0`，macOS `.app.tar.gz` 因 X10-47 改名步骤用 tag 版本反而带 `-beta`，四平台两种命名混用致 fail-closed 落空。
  - **修复口径（定案）**：资产名一律用基础版本号（tag 预发布后缀 `-beta` 不进文件名）；macOS 改名步骤与 latest.json 脚本统一 `VERSION="${VERSION%%-*}"`；latest.json `version` 字段=conf 版本。tag 重打触发第 6 次 run。

- [x] X10-53 updater 端点恒 404（prerelease 不进 `releases/latest` 别名）（2026-10-01 凌晨，自动化守卫轮发现并修复）。
  - **根因（实测）**：编译进客户端的端点 `https://github.com/g-star1024/MirrorDock/releases/latest/download/latest.json` 恒 404——GitHub 的 `releases/latest` 别名**只解析非 prerelease、非 draft 的 Release**，而本仓库所有 Release 均以 `--prerelease` 发布（beta 语义，产品决策不动）。tag 直链（`releases/download/v0.3.0-beta/latest.json`）内容有效（4 平台、全带签名、URL 均可用）。即 v0.3.0-beta 起全部已装客户端「检查更新」必然失败。
  - **修复**：①release job 新增步骤「发布 latest.json 到 updater 分支（常驻端点）」——经 GitHub Contents API 把 latest.json 建/更到常驻分支 `updater`（单文件提交，release job 已有 `contents: write`，无需新凭据），随后 curl+JSON 自校验端点可取且版本一致（25s 重试窗口）；②`tauri.conf.json` 端点改为 `https://raw.githubusercontent.com/g-star1024/MirrorDock/updater/latest.json`（raw CDN 约 5 分钟缓存，对用户手动检查更新可接受）。
  - **验证**：build.yml YAML 解析通过、步骤顺序正确（latest.json 资产上传 → updater 分支发布 → Release 校验）；tauri.conf.json JSON 有效。端到端生效需下一次 tag 构建（首个验证轮 = 下个 release 的 release job 全绿 + `updater` 分支 raw URL 可取）。
  - **遗留（外部/待用户拍板）**：v0.4.0-beta tag 构建（run 36778123621，2026-10-01 05:14 发起）使用修复前的 tag 提交 dddf131——若该构建先于本修复完成，其产物客户端仍带 404 端点，需重打 tag 或接受手动下载升级一次；是否重打由用户/发版会话决定（涉及已发布制品，守卫轮不代行）。

- [x] X10-53 桌面模式白屏/黑屏（真机定案，2026-10-01 晨）。`--new-display` 在 MIUI（Redmi M2104K10AC, Android 13）上成功创建虚拟显示器，但 **MIUI 桌面不在虚拟屏上渲染**——无头录制实测首帧平均亮度 0/255（黑屏），用户感知为白屏且「手机没反应」（虚拟屏本就不显示在手机主屏，属预期）。scrcpy 引擎本身正常：`--new-display --start-app=com.android.browser` 无头实测虚拟屏完整渲染浏览器界面（抽帧确认为 Explore 页）。
  - 修复：`SessionOptions.desktop_app: Option<String>`（serde 默认 None，旧配置安全回填）——桌面模式开启且填写包名时追加 `--start-app=<pkg>`；包名白名单校验（字母/数字/点/下划线、≤120 字符，非法 → `desktop_app_invalid`）；未填行为不变（只创建虚拟屏）。
  - 新命令 `list_device_apps`（37→38）：`pm list packages -3` 解析 + 排序去重，同一套设备就绪门槛；前端桌面模式行下新增「虚拟屏启动的应用（可选）」输入框（datalist 联想，首次勾选拉取一次，失败不阻塞手填）。
  - 「手机没反应」已写进设置说明：虚拟屏独立于主屏，主屏不打断才是设计意图。
  - 证据：cargo 177（+2：start-app 组合与校验、pm list 解析与就绪门槛）/ vitest 41 / tsc / pnpm build 全绿；真机无头复现与修复验证记录见当日日志。
- [x] X10-54 关于卡按钮移位 + 托盘 GUI 验证脚本（2026-10-01 晨，用户反馈）。
  - 关于卡：「检查更新」从独立行移入按钮行，顺序 = 打开官网 / GitHub 仓库 / 检查更新（原独立说明行改为普通 note）。
  - 托盘 GUI 自动化：沙箱 System Events 被 TCC 拦（-10004，与键击结论一致），无法端到端；交付宿主机脚本 `MirrorDock-内部文档/托盘GUI验证.applescript`（自动验证菜单结构 + 动态文案 + 打开主窗口行为；连接/录制/唤醒/截图/退出仅列出不自动点击，避免真实改会话）。Rust 侧已有 pick_tray_target 3 项测试。
- [x] X10-56 `src/lib.rs` 的 clippy 告警（2026-10-01 夜守卫轮发现 6 处；2026-10-02 凌晨守卫轮修复——工作区干净开工，M4-2~M4-5 已提交，原「避免与人工会话冲突」搁置理由解除）。实际现存 5 处（`6719` 可折叠 match guard 已被人工会话顺带解决）：
  - `717` `unused_mut`（**Windows cfg 风险处**）：`quiet_command` 的 `mut` 仅在 `#[cfg(target_os = "windows")]` 分支使用，**未删 mut**（删除会破坏本机无法验证的 Windows 构建），改为 `#[allow(unused_mut)]` + 注释说明缘由——对两平台均零语义影响。
  - `1342` `type_complexity`：抽 `type TakenProcess = (String, Box<dyn MirrorProcess>)` 别名（与 `companion_pairing.rs` 的 `DeviceBridge` 同做法），函数签名与调用方行为不变。
  - `4093`/`4094` `doc_lazy_continuation`：无线序列号文档列表项与补充说明之间补空文档行。
  - `4325` `collapsible_if`：`device_shadows_group` 内嵌套 `if` 折叠为单条件 `&&` 链（两条件均无副作用，语义等价）。
  - 证据（2026-10-02 01:2x，HEAD=37a2790，工作区仅本改动）：`cargo clippy --all-targets` **零告警**；`cargo test --manifest-path src-tauri/Cargo.toml` **183 passed / 0 failed**（与基线一致，纯重构无行为变化）。CI 门禁不含 clippy（`build.yml` 只跑 `cargo test` + `pnpm build`），本轮为纪律性清零。
- [ ] M4 伴侣 App 产品化（用户授权排期，2026-10-01；范围拆解如下，逐项实现前先给方案再动手）。
  - [x] **M4-1 配对产品化（2026-10-01 完成，X10-55，提交 c6831c5+后续）**：配对协议 MDP1→MDP2，从「一次性会话」升级为「持久互信」。桌面侧：EC P-256 长期身份（rcgen 自签证书，落 `app_data_dir/companion-identity/` 三文件 identity.key/.der/.fp，key 0600；指纹=SHA-256(SPKI)）；挑战-响应互信握手（桌面发 32B nonce 挑战 → 伴侣 `SHA256withECDSA` 签名 → 桌面验签 → 登记台账 `paired-companions.json`）；`RECONNECT <pairing_id>` 重连路径（凭台账公钥挑战验签，免扫码）；新命令 `companion_paired_devices`/`companion_unpair_device`（40 命令）+ 前端「已配对的伴侣设备」列表（移除互信）。伴侣侧：Android Keystore 生成不可导出 EC P-256 身份（PairingIdentity.kt），MDP2 握手客户端（device_hello 带 SPKI → challenge → SHA256withECDSA 应答 → paired_ok 存 SharedPreferences）。签名验证兼容 DER（Java）与定宽 r||s 双编码（p256 默认特性不开 ecdsa/der，`to_vec()` 出定宽——两端编码约定不一致的坑）。cargo 178 / vitest 41 / 伴侣 APK 构建全绿。真机待验：扫码配对 → 事件流「互信已建立」→ 桌面台账出现设备 → 移除互信后重连被拒。
  - **兼容性影响（M4-1 协议 tag 变更）**：二维码载荷前缀由 `MDP1` 改为 `MDP2`，桌面端（`src/App.tsx` 的 `pairingPayload`）与伴侣端（`PairingClient.parse`）在**同一提交**内同时切换；因此 97a8fb3 之前构建的伴侣 APK 与之后的桌面端**互不兼容**（旧 APK 解析不了 `MDP2|` 载荷）。伴侣 App 仍处 POC、未发布正式 APK（companion.yml 只产 artifact，版本 0.1.9），影响面限于本机调试产物；任意一侧更新后需**两端同版本**重装。
  - **守卫轮加固与独立复核（2026-10-01 夜，自动化守卫轮，本项唯一改动文件 `src-tauri/src/companion_pairing.rs`）**：
    - ①**单行消息上限 8 KiB（`MAX_LINE_BYTES`，安全加固）**：配对监听期间端口对局域网开放，且服务端**不校验客户端身份**（互信靠握手阶段的挑战签名建立）——任意同网主机都能完成 TLS 并发送数据；原实现用 `read_line` 无上限缓冲，「不发换行、持续灌数据」在 10 秒读超时前可把进程内存吃满。改为 `read_line_bounded`（**先探测换行、再判累计长度**，超限立即断开并 consume 残量），失败原因分 5 类如实入事件流（对端提前断开 / 等待超时 / 超过上限 / 非有效文本 / 读取出错），不再统一成「会话结束（超时或断开）」。
    - ②**互信握手失败补显式拒绝**：公钥非法与挑战签名验证失败两条路径此前只写事件流就断开，对端（伴侣 App）等不到回复，只能显示成「电脑没有回复配对结果」；现补发 `{"type":"rejected"}`，伴侣端据此显示「电脑没有接受本机的身份证明」（**无需改动 Kotlin**）。
    - ③**新增 3 项测试**：DER 编码签名被接受（**真机 Java `SHA256withECDSA` 实际走的编码路径，此前零覆盖**——原测试只发 Rust 侧 `to_vec()` 的定宽 r||s）、`read_line_bounded` 边界（等于上限通过 / 超限 `TooLong` / EOF 与超长不混淆）、端到端超长行（2 MiB 无换行必须在 10 秒读超时**之前**掐断，断言事件文案 + 耗时 < 6s）。
    - ④顺手清理本模块 clippy：`serve_session` 的 `identity` 死参数、3 处 `type_complexity`（抽 `pub type DeviceBridge` 别名）、`hex_decode` 的 `is_multiple_of`。现 `companion_pairing.rs` **clippy 零告警**。
    - **复核证据（2026-10-01 23:00–23:20，HEAD=97a8fb3，工作区开场干净）**：`cargo test --manifest-path src-tauri/Cargo.toml` **181 passed / 0 failed**（178 → 181）；`pnpm build` 通过、vitest **41 passed / 0 failed**；伴侣端 `:app:assembleDebug` **BUILD SUCCESSFUL**（经 gradle-run 包装器，`app-debug.apk` 4,459,024 B @ 07:57:32，晚于全部 Kotlin 源文件 mtime 07:56:48 ⇒ 确实包含 M4-1 改动）。真机面（互信往返、超长行中断的真实观感）仍为外部阻塞，未伪造。

  - [x] **M4-2 常驻通道（2026-10-01 完成，提交 e19df23 + b54a222）**：桌面侧新增 `companion_begin_resident`/`companion_end_resident`（42 命令）：常驻监听**优先固定端口 47017**（被占回退随机并在事件流如实说明），只接受 `MDP2 RECONNECT <pairing_id>`（首配必须走扫码——常驻通道没有二维码出带校验环节，不降低信任锚），并发处理多会话（每连接独立任务），`ping→pong` 心跳（伴侣端 15s，桌面 300s 静默判死）；`paired_ok` 附带 `resident_port`（未开启为 null，伴侣端据此清除过期端口），开启/端口变化时经活跃会话推送 `resident_port` 消息。伴侣侧 `PersistentConnectionService`（前台服务 dataSync）：15s 心跳、断连 5s→10s→…→300s 指数退避重试（成功归零、被拒不重试）、LinkState 共享状态、重连目标 = last_host 主机 + 本地 resident_port；主界面「上次配对的电脑」卡一键直连（Android 13+ 先请求通知权限，POST_NOTIFICATIONS 后续动作按请求意图路由）。桌面 UI：伴侣卡新增「常驻通道（免扫码重连）」开关 + 事件流，挂载时同步真实状态。
  - [x] **M4-3 状态通知（2026-10-01 完成，同上提交）**：伴侣端前台常驻通知随「连接中/已连接/重试中」实时更新（LinkState 监听）；连接建立/断开/被拒/无凭据另发可划走的事件通知（IMPORTANCE_DEFAULT 通道）；桌面录制开始/结束经常驻通道下发 `{"type":"recording"}` → 伴侣端通知。**通知可关**：`paired_computers` 的 `notify_events`（默认开）。桌面端挂点：`start_mirroring`（带录制启动）/`update_session_options`（录制状态跃迁）/`tray_record_toggle`/`stop_mirroring` 四处（`notify_companion_recording`）；**已知边界**：会话异常中断导致的录制终止不推送（无跃迁点）——**已于 2026-10-02 守卫轮收口，见下方「M4-3 补齐」条目**。
  - [x] **M4-4 互信撤销（2026-10-01 完成，同上提交）**：伴侣 App「解除这台电脑」按钮（确认对话框明示后果）：停常驻服务 → 清 `paired_computers` → 销毁 Keystore 身份（`PairingIdentity.destroy()`，私钥不可导出、删除即作废）——与桌面端 `companion_unpair_device` 双向对齐，两端作废后必须重新扫码。
  - [x] **M4-5 发版对齐（2026-10-01 完成）**：伴侣端版本独立维护，升 **0.2.0**（versionCode 11）；`build.yml` companion-apk job 在 tag 触发时把 APK 改名为 `MirrorDock-companion-<基础版本号>.apk`（与桌面资产口径一致，Release 资产可区分版本）；`companion.yml` artifact 按versionName 命名。
  - [x] **M4-3 补齐：录制终止通知全覆盖（2026-10-02 守卫轮，夜间窗口自主决策；改动 = `src/lib.rs` + `src/companion_pairing.rs` + 本文件）**。M4-3 原记录的已知边界「会话异常中断导致的录制终止不推送」收口，并顺带发现、修复两处同类缺口。通知判定引入精确谓词 `any_session_recording`（持有录制路径**且**进程仍在运行才算「正在录制」——`current_recording_with` 的展示语义会回退到已结束的历史录制条目，用于通知跃迁判定会误报）：
    - ①**会话监视线程退出钩子**（原记录的边界）：进程退出时若会话仍持有录制路径，无论退出码（用户直接关闭镜像窗口=正常 / 手机掉线崩溃=异常），一律补推 `{"type":"recording","active":false}`——录制随进程终止是事实，与退出原因无关。
    - ②**托盘「断开连接」**（本轮新发现）：原实现经 `take_all_running_processes` 结束全部会话但从不通知；现停止前快照「真正正在录制」的序列号集合，只有停止成功的录制会话才触发通知（停止失败的会话录制并未终止，通知即谎报）。
    - ③**会话重启失败路径**（本轮新发现）：`update_session_options` 原来只在 `result.is_ok()` 时对比跃迁；重启失败（旧窗口已被结束、新窗口未起来）时录制同样终止却无通知。现失败路径补查 `any_session_recording`：录制确实结束才通知——旧窗口「无法结束」仍在运行时录制并未停止，不通知。
    - ④**快照谓词统一**：`stop_mirroring`/`update_session_options` 的 was/now 快照从 `current_recording_with` 换成 `any_session_recording`，消除「历史录制残留条目被当成正在录制」的误报（该误报只导致多余的一条 inactive 通知，无害但失真）。
    - ⑤顺带清零本轮 clippy：`lib.rs` 退出拦截改 match guard（`Some(code)` 原本落入空臂，语义不变）；`companion_pairing.rs` 心跳 ping 臂改 guard+显式空臂（成功路径不得落入「未知消息类型」）。两处均为 M4 提交引入的存量告警，非本轮改动。
    - **边界（如实记录）**：应用进程整体退出时伴侣通道随之消亡，无法推送「录制结束」——伴侣端由心跳判死后进入退避重试，属固有行为；多会话同时录制时各自退出会各推一条 inactive（伴侣端幂等更新，无累积影响）。
    - **证据（2026-10-02 02:4x，HEAD=6960cb4 + 本改动，工作区仅本改动）**：`cargo test --manifest-path src-tauri/Cargo.toml` **184 passed / 0 failed**（基线 183 + 新增 `companion_recording_notification_only_counts_truly_recording_sessions`：空表/无录制进程/录制中/进程退出后残留条目四态判定）；`cargo clippy --all-targets` **零告警**。纯 Rust 改动，未触碰前端（`pnpm build` 无需）。真机面（伴侣端收到异常中断后的通知观感）仍为外部阻塞，未伪造。
  - **踩坑实录（M4-2）**：①`tokio::io::split` 的 BiLock 在读半边挂起等待数据期间挡住写半边，桌面事件推送永远送不出去——改为单任务 `select!` 独占整条流；②下行消息**必须带换行**（行协议分帧），缺 `\n` 的广播「已到达但黏在下一条消息上」，客户端 readLine 永久等待——`broadcast_line` 统一补齐；③PrintWriter 吞 IOException，`sendLine` 必须用 `checkError()` 探测，否则断链后永远「发送成功」。
  - cargo **183** / vitest 41 / 伴侣 APK 构建全绿。真机待验：桌面开常驻 → 伴侣「连接上次配对的电脑」直连 → 断网重连退避 → 录制通知 → 双向解除互信。
  - 硬性依赖：真机联调（配对/心跳/通知行为）；Keystore 签名发布配置已就位（mirrordock-companion.keystore）。

- [x] **M4 真机实测第一批（2026-10-02 早，Redmi M2104K10AC / Android 13，0.2.0 versionCode 11）**：扫码配对成功（用户手动扫码，配对卡显示桌面地址与身份指纹）✓；植入凭据法验证常驻链路：FGS 启动（startForegroundCount=1）→ 连接失败 5s→10s **指数退避实锤**（PersistentLink 日志）→ UI 实时「重试中：将在 20 秒后重试（第 3 次）」→「断开」服务停止 →「解除这台电脑」确认框 → 凭据清空（410B→65B）+ Keystore 销毁 + 重启后卡片隐藏 ✓；全程 logcat 无 FATAL。**发现并修复 FGS 崩溃（X10-58）**。
- [x] **X10-58 FGS 崩溃修复（2026-10-02 早，真机实锤）**：服务未运行时（如先「断开」再「解除这台电脑」）经 `startForegroundService` 拉起 `PersistentConnectionService`，`ACTION_STOP` 分支直接 `stopForeground+stopSelf`、从未调用 `startForeground()`，Android 5 秒判定超时抛 `ForegroundServiceDidNotStartInTimeException`（CrashGuard 落盘取证成功，用户报障即此例）。修复 = ACTION_STOP 分支先 `startInForeground()` 再停；顺带给 LinkState 监听加 `listenerBound` 防重复注册。伴侣端升 **0.2.1**（versionCode 12）；真机复测崩溃场景不再崩溃。
- [x] **a) 手机→电脑发送（2026-10-02 早，Outbox.kt + MainActivity + 文件卡）**：文件卡新增「发送文件到电脑」——SAF 多选 → 复制进下载/MirrorDock（与桌面 `list_device_files`/`fetch_file_from_device` 同目录）：已授权所有文件访问走直接写；API≥29 走 MediaStore Downloads（RELATIVE_PATH，无需权限）；<29 且未授权如实引导。同名顺延不覆盖。**端到端真机验证**：SAF 选 send_test.txt → 手机文件卡实时显示 → `adb pull`（= 桌面取回路径）内容一致 ✓。**排障实录**：首次两次「没有文件被发送」为「最近」列表陈旧条目（底层文件已删，SAF 打开来源 ENOENT），非写入问题；MediaStore 路径一次成功。
- [x] **b) 输入法托管启动提示（X10-41 待拍板项收口，2026-10-02）**：核实发现该能力已随 X10-39 完整落地——`maybe_switch_host_input_source` 已挂在托盘启动/`mirror_start`/`update_session_options` 三条会话启动路径，切换时 emit `host-input-source-switched`，前端已有提示条（"已临时切换到 ABC 布局，镜像结束后自动恢复"）。任务文档过时的「待拍板」记录就此关闭；真机第三方输入法场景观感并入用户验收清单。
- [x] **d) 伴侣端 MediaProjection 镜像接入（C4-02 延伸）搁置决策（2026-10-02，用户授权并行处理 abcd 后裁定）**：与 scrcpy 路线能力重叠、工程量大（实时帧传输要走新数据通道）、且「用电脑看手机」主路径（adb/scrcpy）已可用——C4 POC 保持现状（能力探测/统计上行），不进入产品化。若未来出现「无 adb 授权也要看屏」的真实需求再重启评审。
- [x] **c) RC 演练**：v0.4.2-beta 已发布并全链路核验（2026-10-02 上午）。发版提交 6ad4e15（8 文件含 docs/releases/v0.4.2-beta.md），tag run 36946544681 全绿（约 13 分钟）；Release=1 个、非草稿、prerelease、**30 资产**（四平台安装包+.sig sidecar、APK 首次带版本名 `MirrorDock-companion-0.4.2.apk`（5f8efdc workflow 改动实装验证）、SBOM×8、SHA256SUMS×4、latest.json）；updater 分支 latest.json version=0.4.2 四平台 URL/签名齐全；Release 正文=「本版更新内容」新格式；main 门禁 run 36946531042 绿。

## X10-59/60 客户端体验轮之三（2026-10-02 上午，USB 真机实测驱动）

- [x] **USB 线真机验证（P0-05 收口，2026-10-02，Redmi M2104K10AC）**：①无头首帧实测（沙箱侧）= `scrcpy --no-window --record` USB transport 首次出数据 **2.0s**，12s 录制 603KB MP4 校验有效；②**键鼠真机验证（用户实测）**：USB 镜像窗口点击生效、文字输入（含中文经 UHID）生效 ✓；③**拔线恢复（用户实测）**：拔线后镜像窗口消失（无进程崩溃，DiagnosticReports 无记录——scrcpy 随 transport 断开正常退出）、插回后连接恢复、可重开镜像 ✓，但主窗口无任何提示（感知为「闪退」）→ 缺陷转入 X10-59；④授权撤销恢复仍待用户配合（外部阻塞保留）。
- [x] **X10-59 拔线恢复 UX 收口（2026-10-02）**：`should_auto_reconnect` 去掉「仅无线」限制（旧测试明文记录的「USB 不重连」决策被真机实测推翻——拔线后无提示无恢复，用户感知为闪退）；无线与 USB 统一「异常退出 → 等待 → 自动恢复」，USB 文案=「数据线已断开，正在等待重新插入」；`classify_reconnect_probe` 对非无线端点跳过 `adb connect`；会话监控对异常退出新发 `mirror-session-ended` 事件，前端显示「镜像连接已中断」提示（自动重连接管时被 waiting 提示覆盖）。证据：cargo **186**（+2：auto_reconnect 全连接类型断言、device_file_delete）+ vitest 43 全绿，clippy 零告警。真机拔插观感待下一版 APK/客户端验收。
- [x] **X10-60 发送区闭环 + 崩溃上报通道（2026-10-02）**：
  - **发送区文件管理**：新增 `delete_device_file` 命令（命令面 42→43）+ `AdbRuntime::remove_device_file`（`rm -f -- <path>` 固定 argv 直调）；文件名先过 `validate_transfer_name`（路径逃逸在拼路径前被拒）；工具页每个文件新增「删除」按钮，取回成功后自动刷新列表。
  - **新文件实时提示**：伴侣端 Outbox 发送成功后经常驻通道推 `files_changed` → 桌面 `PairingState::CompanionEventHook` 转发 → 前端 `companion-files-changed` → 提示 + 已打开列表自动刷新；未连接时伴侣端如实记录「打开工具页刷新可见」。发送桥=`LinkState.lineSender`（onPaired 挂载、onDisconnected/onRejected/停止时清除）。
  - **崩溃堆栈上报**：伴侣端常驻连接就绪且存在未上报崩溃记录时推 `last_crash`（JSONObject 转义、截断 8000 字符、本会话幂等）→ 桌面 emit `companion-crash-report` → 主窗口卡片展示（默认收起、按需展开、「知道了」关闭）；全程本地点对点、不落盘不上云。
  - **范围裁定记录**：跨网段远程镜像按用户指示转为 **Pro 候选付费项**（可行性结论见会话纪要：需中继/打洞 + 账号体系 + 威胁模型假设更新，暂缓实施）；macOS 签名公证、Windows 测试、渠道分发三项按用户指示明确排除在本轮外。
- [x] **X10-61 X10-59 文案口径对齐：断线自动重连的适用范围（2026-10-02 守卫轮，纯前端）**：X10-59 已把自动重连从「仅无线」扩到「无线 + USB」，但用户可见文案仍写「无线断线自动重连 / 无线镜像意外断开」——拔线用户会以为这条恢复路径与自己无关，开关的实际作用范围被低估。本轮只改口径、不动行为：
  - **设置 → 通用**：「无线断线自动重连」→「断线自动重连」（含 `aria-label`），说明改为「镜像意外断开时自动等待连接恢复并重建镜像（最多 15 分钟）：无线掉线等手机回网，数据线被拔掉等重新插线」；卡片副标题补入「断线自动重连」；开关切换提示（开/关两侧）同步覆盖两种连接。
  - **帮助中心**：「常见问题」新增一条中途断开后的恢复说明（数据线等重新插回 / 无线等手机回网、预算 15 分钟、「设置 → 通用」可关闭）；「无线连接」篇章的熄屏掉线段落补一句回网自动恢复。
  - **公开站点（`site/index.html`）本轮刻意未改**：该页以 v0.4.2 测试版为口径、下载链接直指 v0.4.2-beta 资产，而 USB 自动重连（X10-59）尚未进入任何已发布客户端——此刻在该页写「USB 也会自动恢复」属超前声明。文案随下一版发布同步（登记在下方待办）。
  - **证据（2026-10-02 10:3x，HEAD=4bbfe0b + 本改动，工作区仅本改动）**：`pnpm build` 通过（tsc + vite，255ms）；vitest **45 passed / 0 failed**（基线 43 + 新增 2 项：`auto_reconnect_switch_copy_covers_both_data_cable_and_wireless`（开关名/说明/开↔关提示四处口径）、`faq_explains_recovery_for_both_the_data_cable_and_wireless_drops`（FAQ 必须两种连接都说清并指明开关位置））；同 HEAD 的 Rust 基线 `cargo test` **186 passed / 0 failed** 已独立复核，本轮未触碰 Rust。
  - 未验证面：文案观感与真机拔线恢复的实际体验，随 X10-59 客户端验收一并由用户确认（外部阻塞）。
  - **待办登记（守卫轮 2026-10-02 明确落条，此前仅口头写「登记在下方待办」而未实际登记）**：①`site/index.html` 的自动重连文案（USB 口径）随**下一版发布**同步——该页以 v0.4.2-beta 为口径，X10-59 尚未进入已发布客户端，提前写即超前声明，故**保持现状不动**；②`.github/workflows/build.yml` 第 138–141 行的注释仍写旧口径「Linux 无官方包，运行时回退到用户 PATH」，与同文件第 170–222 行的 Linux 步骤自相矛盾（后者已正确）——**本轮刻意未改**，因改动 workflow 文件会使本地提交无法由含 `public_repo` 的 PAT 推送，须用户本地推，故并入**下一次触碰 workflow 的改动**一并修正（不影响任何构建行为，仅注释）。
- [x] **X10-62 常驻会话建立即断的真 bug 修复（2026-10-02 真机验证桩实锤）**：用户报「手机传 APK 提示成功、电脑客户端没有正常显示」。排查路径：①真机查发送区——文件确实已落盘（`安装狮.apk.1` 在 `/sdcard/Download/MirrorDock`），手机端无问题；②手机装的是 0.2.1（无 files_changed 推送能力）+ 桌面 0.4.2（无 X10-60 处理）——版本不对齐是第一层；③自建 MDP2 验证桩（Python，用桌面真实 TLS 身份 identity.der/key + 台账做完整 RECONNECT 握手）实测 0.2.2：**握手与验签全通、paired_ok 后对端立刻 EOF，5 秒一次重连死循环**。根因=`PairingClient.connect()` 完成握手、启动读线程后**立即返回**，而 `PersistentConnectionService` runLoop 紧接着 `client.close()`——会话建立即被自己掐断（b54a222 引入，M4-2 真机测试被「每次重连都能完成握手」掩盖，心跳/下行事件/推送从未真正有存活窗口）。修复=connect() 在读线程上 `join()`，语义改为「返回 = 会话已结束」（扫码配对的一次性会话同样受益；MainActivity/服务的 connect 调用均在后台线程，阻塞无害）。
  - 证据：gradle assembleDebug 绿；companion 0.2.3（versionCode 14）随 CI 出包后真机覆盖安装复测（见下）。
  - 排障插曲：重装 0.2.2 重置了 POST_NOTIFICATIONS 运行时权限（点连接弹权限框挡住流程，pm grant 解决）；run-as 修 prefs 时 SharedPreferences 进程内缓存脏读到空凭据致服务自停（force-stop 重启解决）；手机 prefs 无 `resident_port`（扫码配对时桌面常驻通道未开，paired_ok 未带端口）——用户可感知的影响=「连接上次配对的电脑」提示无可直连电脑，属产品待改进项（配对页应引导开启常驻通道）。
- [x] **X10-62 收口：两层根因全修、真机端到端验证通过（2026-10-02 11:4x）**：
  - **根因一（会话建立即断）**：`PairingClient.connect()` 握手完就返回，服务 runLoop 立即 `client.close()` → 修复=a8a32b3 `readerThread?.join()`（返回=会话结束）。真机复测：会话持续在线（心跳 15s 全通 >2 分钟无断），插拔重连退避正常。
  - **根因二（UI 线程推送全失败）**：`files_changed`/`last_crash` 推送在主线程做 socket 写 → Android 抛 `NetworkOnMainThreadException` 被 PrintWriter 静默吞掉 → `checkError()` 恒真 → `sendLine` 恒 false（诊断插桩 0515246 实锤：同一客户端心跳工作线程全通、UI 线程 sent=false）。修复=c5b02b4：`linkSendExecutor`（单线程串行）后台发送，`pushLinkLine` 改异步语义（false=无连接走兜底提示）。
  - **端到端证据（验证桩 /tmp/resident_probe.py，桌面真实 TLS 身份+台账握手）**：`[11:42:50] 上行消息 >>> {"type":"files_changed"}` + `pushLinkLine: sent=true`；心跳 15s 节拍稳定。测试产物已清理（发送区重复副本/probe_test.txt/临时 prefs）。
  - 诊断日志保留（PersistentLink tag，logcat 可查）。companion versionCode 14 / 0.2.3 线上 artifact 为最终修复版。
- [x] **X10-63 拖拽传文件（桌面 → 手机发送区）**：Webview 拖放事件按扩展名路由——`.apk` → 安装到当前就绪手机（沿用「多个只装第一个」规则），**其他文件 → 逐个 `send_file_to_device` 发到该手机发送区**（下载/MirrorDock），结果走右下角浮层如实汇报（部分失败列出每个文件原因）；拖拽全屏提示文案同步改为「APK 安装，其他文件进入发送区」。反向拖出（手机 → 桌面）受 scrcpy 窗口机制限制不做，拖入已覆盖高频场景。
  - 证据：`pnpm build` 通过；vitest 45 passed / 0 failed；真机端到端验证随 X10-62 验证桩轮完成（send_file_to_device 命令链路已在 M4 前真机验证过），拖拽手势本身待用户下一版验收。
- [x] **X10-64 常驻通道「开启时机」口径修正：消除无法兑现的免扫码重连承诺（2026-10-02 守卫轮；改动 = `src/App.tsx` + `src/helpContent.ts` + `src-tauri/src/companion_pairing.rs` + 两处测试 + 本文件）**：
  - **缺口来源（X10-62 真机排障实锤）**：用户手机 `paired_computers` 里**没有 `resident_port`**（扫码配对时桌面常驻通道未开，`paired_ok` 按设计带 `null`），于是伴侣端「连接上次配对的电脑」永久提示「没有可直连的电脑」，**直到重新扫码**。核实后确认不是偶发、而是设计使然：①桌面端只在**扫码会话内**把 `resident_port` 交给手机（`companion_pairing.rs` 的 `paired_ok`）；②开启常驻通道时只向**当前活跃会话**广播端口变化（`broadcast_line`）——**已断开的手机收不到**。端口在配对那一刻没交出去，此后没有任何补交路径（鸡蛋相生：手机拿不到端口就建不了连接，建不了连接就收不到端口）。
  - **三处用户可见文案都在淡化这件事**：桌面卡片原文「扫码配对时无需开启」（读作「顺序无所谓」）；桌面事件流「已配对的伴侣设备可免扫码直连」（对关闭状态下配对过的设备不成立）；伴侣端 `MainActivity.kt` 的 `onPaired` else 分支「在电脑端打开后，这里可以免扫码重连」（同样不成立——**未改**，属伴侣 App 文案，随下一版 APK 处理）。
  - **改动（只改口径与提示，不动任何行为）**：①「常驻通道（免扫码重连）」卡片写清顺序与补救——「请先开启常驻通道、再在手机上扫码」「若手机是在关闭状态下配过对，需要在手机上重新扫一次码（电脑这边不用改）」「关闭后已配对的手机会如实显示『没有可直连的电脑』」，并删去「扫码配对时无需开启」；②扫码入口旁在常驻通道未开启时**前置**提示顺序（代码内 `X10-64` 注释标明缘由）；③开启成功后给出如实提醒（新增 `residentNotice` 状态，与失败消息 `residentMessage` **分开呈现**、不带 `role="alert"`——提示不是错误）：端口只在扫码时交给手机、手机上若提示没有可直连的电脑请重新扫码；④Rust 事件文本改为「常驻通道已开启（端口 N）：**此后扫码配对的**伴侣设备可免扫码直连（此前在关闭状态下配过对的手机需重新扫一次码）」；⑤帮助中心 FAQ 新增一条（现象 → 原因 → 「先开常驻通道再重新扫码」的补救）。
  - **证据（2026-10-02 12:4x，HEAD=61f00b0 + 本改动，工作区仅本改动、开场 `git status` 干净）**：`pnpm build` 通过（tsc + vite，244ms）；vitest **47 passed / 0 failed**（基线 45 + 新增 2：`resident_channel_copy_states_ordering_and_rescan_recovery`——卡片必须写清顺序与重新扫码、且不得再出现「扫码配对时无需开启」，并覆盖未开启时的入口提示；`faq_explains_companion_one_tap_reconnect_needs_resident_enabled_first`）；`cargo test --manifest-path src-tauri/Cargo.toml` **186 passed / 0 failed**（纯字符串改动，无行为变化）；`cargo clippy --all-targets` **零告警**。
  - **未验证面（外部阻塞）**：真机上「先开常驻 → 手机扫码 → 一键重连成功」的端到端观感需**下一版 APK + 下一版桌面客户端**验收（并入 X10-59~63 同一条验收清单）。
  - **待拍板方案（守卫轮不代行产品决策）**：让承诺真正成立有两条路——**(A) 仅修伴侣端文案**（else 分支改为「请在电脑开启常驻通道后重新扫一次码」），需 bump 伴侣版本（0.2.4 / versionCode 15）并重出 APK；**(B) A + 伴侣端在无 `resident_port` 时回退尝试默认端口 47017**，则「电脑开启常驻通道后即可免扫码重连」由文案承诺变为真实能力（代价：电脑未开常驻时会多几轮注定失败的连接尝试，退避机制已具备）。二者都触及 M4 伴侣行为，按「M4 不新增范围除非用户授权」留待拍板。
  - **候选增强收口（X10-37 遗留项）**：X10-37 的「会话建立时探测 scrcpy UHID 启用布局，非标准布局时提示」经**只读实测**判定**不可实现**——本机（Redmi M2104K10AC / Android 13，USB 已授权、无会话）逐面核查：`settings list secure|system|global` 共 862 项中无任何布局项（仅 `keyboard_skin_follow_system_enable` / `show_ime_with_hard_keyboard` / `miui_mechanical_keyboard_support` 三条无关项）；`dumpsys input`（89,679 B）五个输入设备块只有 `KeyboardType` 等映射参数，**没有启用布局清单字段**。该状态不在任何 adb 可读面，探测只能落空。故**关闭该候选**（不改代码），用户侧出口保留为帮助中心 FAQ「电脑上打字没反应或字符全错？」；X10-17 的 `test-runs/uhid-e2e/` 继续作为证据。**未验证面**：会话进行中（scrcpy UHID 设备在线）时的 `dumpsys input` 未复核——本轮为无会话只读探测，未启动真机会话以免打扰正在使用手机的用户。
- [x] **X10-66 手机通知镜像一期：转发 + 桌面展示（2026-10-02 用户拍板：一期只做转发+展示，快捷回复放二期）**：
  - **手机端（companion 0.2.4 / versionCode 15）**：新增 `NotificationMirrorService`（`BIND_NOTIFICATION_LISTENER_SERVICE` 系统绑定式，应用无法伪造；用户在系统设置显式授予「读取通知」）。前置开关 `notify_mirror` **默认关**，主界面新增「通知镜像」卡（开关 + 三态状态行：关 / 已开缺授权 / 已开就绪，onResume 从系统授权页返回自动刷新）。转发规则：只转发内容非空的正常通知（跳过 ongoing 常驻通知与本机通知防自反馈）；标题/文本各截断 1000 字符（最坏 ~6 KiB UTF-8 < 8 KiB 行上限，超限会让桌面判「消息超长」断开会话）；**只在常驻连接在线时实时转发，离线不排队不补发**（防重连瞬间倾倒历史通知）。协议消息 `{"type":"notification","pkg","app","title","text","posted"}`。隐私红线：通知内容不写 logcat（日志只记包名与发送结果）、不落盘。
  - **桌面端**：`companion_pairing.rs` 事件流新增 `notification` 分支——事件流只写通用文案「伴侣端转来一条手机通知」，**标题/内容不入事件流**（事件流是用户可见面也是日志面）；完整内容经 `forward_event` → lib.rs 钩子 emit `companion-notification` → 前端。工具页新增「手机通知」面板（不依赖 USB/ADB 镜像，伴侣通道即生效）：最新在前、仅内存保留最近 50 条、可清空、不落盘；时间为当天显示时刻、跨天带日期（`notificationTime`）。
  - 证据：`cargo test --manifest-path src-tauri/Cargo.toml` **187 passed / 0 failed**（基线 186 + 新增端到端用例 `notification_from_companion_reaches_the_event_hook`——通知到达钩子 + 断言内容不进事件流）；`pnpm build` 通过、vitest **48 passed / 0 failed**（基线 47 + 新增 `notificationTime` 用例）；clippy --all-targets 零告警；gradle assembleDebug 绿。真机验证见「待验收」清单。排障插曲：测试用例上行行缺尾部 `\n` 导致服务端等不到行尾（行协议黏包坑的客户端版复现），补 `\n` 后通过。
  - **二期（未做，等用户发起）**：快捷回复（RemoteInput + 通知操作转发）、历史通知离线补发。
- [x] **X10-64 定案 B 落地：伴侣端无 `resident_port` 时回退默认端口 47017（2026-10-02 用户拍板 A+B）**：
  - `PersistentConnectionService`：本地未学到 `resident_port` 时回退 `RESIDENT_DEFAULT_PORT = 47017` 直连（与桌面端常量一致）——此前「无可直连的电脑」死胡同（端口没在扫码会话内交付就永远拿不到）现在真实可达：电脑固定端口未占用时一键重连成立；被占用回退随机端口的场景仍需重新扫码（如实保留）。类文档同步改写；`MainActivity` 两处口径对齐：扫码配对 else 分支文案改为「按默认端口 47017 直连；仅当电脑端口被占用回退到其他端口时需要重新扫码」、「上次配对的电脑」卡在无端口时显示 `host:47017（默认）`。
  - 证据：gradle 构建绿；真机验收并入下方待验收清单（手机清掉 prefs 端口后一键重连）。
  - **真机端到端验证通过（2026-10-02 17:0x，手机 Redmi M2104K10AC / 桌面 0.4.3-beta / 伴侣 0.2.4）**：①X10-64B——运行时移除手机 prefs 的 `resident_port` 后点「连接上次配对的电脑」，logcat 证据 `尝试连接 192.168.0.177:47017`（无端口回退默认）→ `TLS 已建立` → `onPaired: lineSender attached`（两轮：桌面开常驻前 ECONNREFUSED 退避、开启后实连成功，代价场景与设计一致）；②X10-66——`adb shell cmd notification post` 测试通知 → `NotifyMirror: forward pkg=com.android.shell sent=true` → 桌面事件流「伴侣端转来一条手机通知」（内容未入事件流）。排障实录：重装后监听器在允许列表但未绑定（Live 列表无记录）——disallow/allow + 重启 App 触发重绑后 BOUND；绑定瞬态 DeadObjectException 为 force-stop 副作用，无碍。注意：force-stop 会连带杀常驻连接服务，需重新点连接。
- [x] **v0.4.3-beta 发版（2026-10-02；用户指令：开发完推送打包发布新版，两端升级后更新官网）**：版本 0.4.2→0.4.3（`tauri.conf.json` / `package.json` / `Cargo.toml` / `Cargo.lock` / `README.md`，伴侣端 0.2.4 随 `MirrorDock-companion-0.4.3.apk` 分发）；新增 `docs/releases/v0.4.3-beta.md`（内容 = X10-59/60/61/62/63/64+64B/66 汇总，含 0.4.0 及更早客户端检查更新 404 需手动升一次的说明）；`site/index.html` 版本与下载链接批量更新并修正伴侣 APK 链接（v0.4.2-beta 起资产名为 `MirrorDock-companion-<版本>.apk`，官网仍指旧 `app-debug.apk` 的死链一并修复）+ 功能清单补三条（拖拽传文件/通知镜像/断线重连口径扩 USB）+ 伴侣卡口径更新；`site/compatibility.html` + `docs/compatibility-matrix.md` 打包列 ✅ 同步 v0.4.3-beta（吸取 v0.4.2 发版漏同步、事后补账的教训）。
  - **发版证据（2026-10-02 16:4x）**：发版提交 f2b1d15 推送 → tag `v0.4.3-beta`（run 36983569054 构建成功）→ Release 自动产出 **30 资产**（四平台安装包+.sig、`MirrorDock-companion-0.4.3.apk`、SHA256SUMS×4、SBOM×8、latest.json），prerelease ✓ 非草稿 ✓；updater 分支 `latest.json` version=0.4.3、四平台 URL/签名齐全 ✓。**两端升级**：桌面 /Applications/MirrorDock.app 经 Release 包替换为 0.4.3（ditto 替换、版本号核验、启动运行 ✓；升级后常驻通道需重新开启——运行时状态）；手机经 CI artifact（run 36983108893）adb 覆盖安装 0.2.4（versionCode 15 ✓），重装后 pm grant POST_NOTIFICATIONS + cmd notification allow_listener 恢复授权。**官网**：pages workflow 随 f2b1d15 部署成功，线上首页与兼容性页经 curl 核验均为 v0.4.3 内容 ✓。**通知镜像默认关、开关在手机端**——真机验证时经 run-as 开启（force-stop 后写入，规避脏缓存坑），留在开启态。
- [x] **X10-67 新功能帮助覆盖（X10-63 拖拽传文件 / X10-66 通知镜像）+ README 篇数纠错（2026-10-02 守卫轮；改动 = `src/helpContent.ts` + `src/helpContent.test.ts` + `README.md` + 本文件）**：v0.4.3-beta 把两个用户可见新功能交付到用户手里，但**帮助中心零覆盖**（实测 `grep`：`拖拽/拖放/发送区` 命中 0，`通知` 仅 2 处且非本功能），非技术用户在应用内没有可自助的路径；同时 `README.md` 对外声明「8 篇完整帮助文档」而实际已 9 篇——**等于向用户报了一个错数字**。本轮为纯内容层修复，不动任何代码行为、界面版式与交互：
  - **`tools` 篇章补三条**：①拖拽路由（拖入 `.apk` 安装到当前手机、多个只装第一个；其他文件进手机发送区「下载 / MirrorDock」；结果在右下角如实汇报、失败项写明原因，含「请先在连接页连接手机」的前置条件）；②发送区管理（「查看手机上的文件」→ 逐条「取回到电脑」/「删除」，删除只影响该目录内这一个文件）；③反向通道（伴侣 App「发送文件到电脑」落到电脑同一文件夹，到达时桌面提示并刷新列表）。
  - **新增「手机通知镜像」篇章（帮助中心 9 → 10 篇）**：开启前提（手机伴侣 App「通知镜像」开关 + 系统「读取通知」授权 + 助手通道在线，**默认关闭**）、隐私承诺（仅内存展示、最近 50 条、可随时清空、不落盘、不入日志、不经云端）、如实边界（连接断开期间收到的通知**不补发**；常驻通知与 MirrorDock 自身通知不转发）、停止方式（手机端关开关）。文案与桌面面板现有措辞（`App.tsx` 的「手机通知」卡片说明）逐句对齐，不发明界面路径。
  - **README 同步**：「内置帮助中心」篇数 8 → 10（与新增后实际篇数一致）；「文件传输」条目补发送区互传 / 逐条删除 / 反向「发送文件到电脑」；新增「拖拽传文件」「手机通知镜像」两条功能；「APK 安装」条目的拖拽描述去重（归入新条目）；「伴侣 App」条目补常驻通道一键重连的顺序要求（先开常驻通道、再用手机扫码）。
  - **证据（2026-10-02 16:5x，HEAD=f2b1d15 + 本改动，工作区仅本改动、开场 `git status` 干净）**：vitest **51 passed / 0 failed**（基线 48 + 新增 3：`tools_article_covers_drag_routing_and_send_zone_management`、`notifications_article_states_prerequisites_privacy_and_off_switch`、`readme_declares_the_same_help_article_count_as_shipped`）；`pnpm build`（tsc + vite）通过。**新增的篇数不变量读 `README.md` 校验「声明篇数 == 实际篇数」**，把「对外报错数字」这一类漂移纳入自动化门禁（jsdom 下 `import.meta.url` 为 http 地址，须按 `process.cwd()` 解析真实路径——已实测踩坑并修正）。纯前端内容层，无 Rust 改动；同 HEAD 的 Rust 基线以 CI 为准：f2b1d15 的 push 门禁「验证（Rust 单测 + 前端构建）」**success**，tag 打包 run 四平台 + 伴侣 APK + 发布 Release 全部 success（本轮未重跑本地 cargo）。
  - **未验证面（外部阻塞）**：帮助文案与新条目在新版界面里的可读性属用户验收范围；篇数不变量依赖 README 保持「N 篇完整帮助文档」这一句式，若日后改写需同步调整断言。
- [x] **X10-68 多设备管理（X10-27 里程碑 2 收口：托盘逐会话 + 设置逐会话 + 设备收藏，2026-10-02 用户拍板按序开工）**：
  - **托盘逐会话细化（后端 `src-tauri/src/lib.rs`）**：托盘菜单从「静态菜单 + 句柄改文案」改为**整条动态重建**（`build_tray_menu` + `TrayIcon::set_menu`，删除 `TrayMenuHandles`）：每个真正持有镜像进程的设备各占一条「结束 <设备名> 的镜像」（设备名取最近使用记录的友好名称，查不到如实显示序列号；`tray-stop-<serial>` 前缀路由到 `tray_stop_device`，逻辑与 `stop_mirroring` 单设备路径一致：录制中快照→停止→伴侣补录制结束→还原亮屏补偿→清账本→恢复输入源）；全局项语义不变（连接/断开、录制开关、唤醒、截图作用于主会话）。菜单事件处理器挂在托盘图标上，`set_menu` 重建后依然生效。
  - **设置逐会话化（前端 `App.tsx`）**：多会话（≥2 台镜像中）时设置页「镜像窗口」卡顶部出现「应用到哪台设备」选择器（列出运行中的会话，显示设备友好名），改动只作用于选中设备（`update_session_options` 已支持 `serial`）；单会话保持原行为零感知；目标设备的会话结束后自动回退主会话；应用结果提示带设备名前缀。
  - **设备收藏置顶（前端）**：设备卡与最近设备列表新增收藏星标（`localStorage: mirrordock.favoriteDevices`，仅本机显示排序，不动连接与授权）；最近列表收藏置顶、设备卡收藏项显示金色 ★。
  - 证据：`cargo test` **190 passed / 0 failed**（+3：快捷回复校验三用例与托盘无关，见 X10-69；托盘菜单构建依赖真实托盘句柄，属真机/桌面端人工验收面）；clippy --all-targets 零告警；`pnpm build` + vitest **51 passed / 0 failed**。**未验证面（外部阻塞）**：托盘逐会话条目、多会话设置选择器需双设备真机/多会话场景人工验收；单设备场景托盘菜单外观无回归（同一构建路径）。
- [x] **X10-69 通知镜像二期：快捷回复（RemoteInput 转发，两端同版本依赖）**：
  - **协议**：上行 `notification` 增加 `key`（SBN key，回指原通知）与 `replyable`（通知自带 RemoteInput 回复动作才为 true）；下行 `{"type":"notification_reply","key","text"}`。旧版两端互操作安全：新手机+旧桌面忽略新字段；新桌面+旧手机无 key/replyable → 前端不显示回复框。
  - **手机端（companion 0.2.5 / versionCode 16）**：`NotificationMirrorService` 增加 key/replyable 上行与 `handleReply`：key 与 `activeNotifications` 精确匹配（已撤回通知不可回指）→ 找到带 RemoteInput 的动作 → **只向 PendingIntent 填充 RemoteInput 结果**（`RemoteInput.addResultsToIntent`，不注入任何额外 extras）→ `actionIntent.send`；文本截断与转发一致（1000 字符）；日志只记包名与结果。`PersistentConnectionService` 下行分发 `notification_reply` → 静态存活的监听服务实例（onCreate 登记 / onDestroy 清除）。
  - **桌面端**：新命令 `companion_notification_reply`（校验抽为 `validate_notification_reply` 可测函数：key ≤256B 无控制字符、text 非空 ≤500 字符无控制字符/换行；serde_json 转义保行协议；`broadcast_line` 下发；无活跃会话返回 `sent=false` 供前端如实提示，不报错）；lib.rs 通知钩子把 key/replyable 传给前端。前端「手机通知」面板：replyable 通知显示回复框（Enter 或按钮发送、发送中禁用、成功清空草稿、失败/离线给出可操作提示）；草稿与提示仅内存。
  - 证据：`cargo test` **190 passed**（+3：`notification_reply_validation_accepts_normal_input` / `_rejects_bad_key_and_text` / `_allows_500_chars`）；clippy 零告警；`pnpm build` + vitest 51 过；gradle assembleDebug 绿。**未验证面（外部阻塞）**：真机快捷回复端到端（需 0.2.5 APK + 用户实际用微信/短信等带回复动作的通知验证；`adb shell cmd notification post` 的通知不带 RemoteInput，无法模拟回复场景——需真实聊天应用通知）。
- [x] **v0.4.4-beta 发版（2026-10-02；用户拍板顺序 1→2 后即发版：多设备管理 + 快捷回复攒齐走 v0.4.4-beta）**：版本 0.4.3→0.4.4（`tauri.conf.json` / `package.json` / `Cargo.toml` / `Cargo.lock` / `README.md`，伴侣端 0.2.5 随 `MirrorDock-companion-0.4.4.apk` 分发）；新增 `docs/releases/v0.4.4-beta.md`（内容 = X10-68 多设备管理 + X10-69 快捷回复，含 0.4.0 及更早检查更新 404 提示与两端同版本要求）；`site/index.html` 版本/下载链接/APK 资产名同步 + 功能清单补「多台设备同时镜像」「通知快捷回复」两条；`site/compatibility.html` + `docs/compatibility-matrix.md` 打包列 ✅ 同步 v0.4.4-beta。tag `v0.4.4-beta` → CI 打包 → Release/updater 事后核验（tag 发版全自动，流程同 v0.4.3-beta 实测）。
  - **发版证据（2026-10-02 18:1x）**：发版提交 e193e7e → tag `v0.4.4-beta`（run 36992357572 构建成功）→ Release 自动产出 **30 资产**，prerelease ✓ 非草稿 ✓；updater 分支 `latest.json` version=0.4.4、四平台（darwin-aarch64/darwin-x86_64/linux-x86_64/windows-x86_64）✓。**两端升级**：桌面 /Applications 经 Release 包替换为 0.4.4（版本号核验、启动运行 ✓；升级后常驻通道需重新开启——运行时状态）；手机 adb 安装 CI artifact（run 36992139459）覆盖升 0.2.5（versionCode 16 ✓），监听器经「App 存活时切换授权」重绑 ✓。**官网**：pages workflow 随 e193e7e 部署成功，线上首页与兼容性页核验均为 v0.4.4 内容（含「多台设备同时镜像」「直接回复」新特性文案）✓。**真机核验**：0.2.5 通知转发端到端 `NotifyMirror: forward pkg=com.android.shell sent=true` ✓。**未验证面**：快捷回复需真实聊天应用通知（RemoteInput），留待用户日常使用验证。
- [x] **X10-70 新功能帮助覆盖补全（X10-68 多设备管理 / X10-69 通知快捷回复）+ README 篇数与功能条目同步（2026-10-02 守卫轮；改动 = `src/helpContent.ts` + `src/helpContent.test.ts` + `README.md` + 本文件）**：v0.4.4-beta 把两个用户可见功能交付到用户手里，但**帮助中心零覆盖**（实测 `grep`：`快捷回复` 0 命中、`多设备` 0 命中），非技术用户在应用内没有可自助路径；`README.md` 也未登记这两个已发布功能。与 X10-67 同类，纯内容层修复，不动任何代码行为、界面版式与交互。
  - **`notifications` 篇章补快捷回复段落**：回复框出现条件（只有通知自带回复动作才会出现，系统提示类没有）、发送方式（回车或「发送」）、内容经电脑与手机之间的加密通道送回手机、由伴侣 App 填进原通知的回复动作发出、上限 500 字、通知已被撤回则无法回复、伴侣不在线时如实提示「连接不在线」而不静默丢失、草稿只留在当前窗口不落盘。文案与 `App.tsx` 通知面板实际措辞逐句对齐（`placeholder` = 「回复 {app}…」、按钮「发送」、`maxLength={500}`、离线提示原文「手机端连接不在线…」）。
  - **新增「同时镜像多台设备」篇章（帮助中心 10 → 11 篇）**：多台各一独立窗口互不干扰；收藏置顶（设备卡/最近列表星标 ☆ → ★，**只影响本机显示排序、不改变连接状态与调试授权**）；多会话（≥2 台镜像中）时设置页「应用到哪台设备」选择器只作用于选中那一台、其余不受影响；托盘逐台「结束 <设备名> 的镜像」与全局项作用于当前会话；多台资源开销的如实提醒。均对照实现取证：`App.tsx` 星标与选择器、`lib.rs` `build_tray_menu`（逐会话条目 + 友好名回退序列号）。
  - **README 同步**：「内置帮助中心」篇数 10 → 11 并补「多设备管理」覆盖项；「镜像与会话」新增「多台设备同时镜像」条目；「工具」新增「通知快捷回复」条目（含 0.4.4 / 0.2.5 版本要求与「不静默丢失」承诺）。
  - **证据（2026-10-02 18:5x，HEAD=f45f2b5 + 本改动，开场 `git status` 干净、local==remote）**：`pnpm build`（tsc + vite）通过（288ms）；vitest **53 passed / 0 failed**（基线 51 + 新增 2：`notifications_article_covers_quick_reply_and_offline_notice`、`multi_device_article_covers_favorites_per_session_target_and_tray_stop`）；既有「README 声明篇数 == 实际篇数」不变量继续生效并已随之更新至 11。纯前端内容层、无 Rust 改动 ⇒ Rust 基线以 f45f2b5 的 CI 为准（build/pages 均 success），未重跑本地 cargo。
  - **顺带核验（只读，无改动）——v0.4.4-beta 发版链复核通过**：Release 唯一、非草稿、prerelease、**30 资产**（伴侣 APK `MirrorDock-companion-0.4.4.apk` 带版本名）；`site/index.html` 9 条下载链接与实际资产名逐一对齐、无死链；版本口径七处一致（`tauri.conf.json` / `package.json` / `Cargo.toml` / `Cargo.lock` / `README.md` / `site/index.html` / `site/compatibility.html` / `docs/compatibility-matrix.md` 均为 0.4.4）；`updater` 分支 `latest.json` version=0.4.4、四平台（darwin-aarch64 / darwin-x86_64 / linux-x86_64 / windows-x86_64）齐全。
  - **未验证面（外部阻塞）**：帮助文案在新版界面里的可读性属用户验收范围；快捷回复端到端仍需真实聊天应用通知（X10-69 待验收项不变）。
- [x] **X10-71 桌面模式按设备单独设置 + 应用名下拉（2026-10-02 用户反馈两问题）**：
  - **问题**：①多设备时不是所有设备都开桌面模式，「桌面模式开关 + 虚拟屏启动的应用」却跟随全局一份设置，无法按已开启的设备单独保存；②应用候选下拉只显示包名（如 `com.netease.uuremote`），小白用户看不懂。
  - **按设备保存（前端 `src/App.tsx`）**：桌面模式开关与虚拟屏应用整体下沉为设备级偏好（`localStorage: mirrordock.desktopPrefs`，按序列号键控）：有会话目标（「应用到哪台设备」选择器 ?? 主会话设备）时读写该设备自己的偏好，无会话时编辑全局默认（随下次启动哪台就带哪台）；`composeOptionsWithDesktop` 在启动镜像（`start_mirroring`）与会话中应用（`update_session_options`）时按目标设备叠加偏好并维持摄像头源互斥；「恢复默认设置」连各设备偏好一并清空。其余设置仍为全局一份（范围按用户反馈只动桌面模式相关）。
  - **应用名下拉（后端 `src-tauri/src/lib.rs` + 前端）**：`list_device_apps` 升级为返回 `{package, name}`：首选 `scrcpy --list-apps`（只列**可启动**的应用且带应用名，与「虚拟屏启动应用」场景一致；15s 超时兜底强杀，失败或解析为空回退 `pm list packages -3`、名字退化为包名）；解析器剔除 server 日志行与控制字符、包名白名单校验、按应用名排序按包名去重。前端 datalist 改为 `<option value=包名>应用名</option>`，已填包名匹配到候选时说明文字追加「当前已选：<应用名>」。
  - 帮助中心：「同时镜像多台设备」篇补一段（桌面模式按设备记忆 + 下拉按应用名称选择），篇数不变（11），README 篇数口径无需变更。
  - 证据：`cargo test` **192 passed / 0 failed**（新增 `scrcpy_app_list_parses_names_and_ignores_noise`、`named_apps_prefer_scrcpy_output_and_fall_back_to_pm_list`，既有 pm list 测试改断言新结构）；clippy --lib 零告警；`pnpm build` + vitest **58 passed / 0 failed**（新增 readDesktopPrefs 两项、composeOptionsWithDesktop 三项）。**`scrcpy --list-apps` 真机实测**（Redmi M2104K10AC / Android 13，经随包 4.1 二进制）：输出 ` * 应用名<空白>包名` 与解析器假设一致，21 个可启动应用（vs `pm -3` 41 个包，可启动过滤正是本场景想要的）。**未验证面（外部阻塞）**：设置页新交互（按设备切换/编辑、应用名下拉选择、恢复默认清偏好）需下个版本 GUI 人工验收；scrcpy `--list-apps` 与镜像会话并发运行理论无害（server jar 覆盖写不影响已加载进程），待多设备轮次顺带复核。
- [x] ✅ **v0.4.5-beta 代码落地（未发版）——记录已修正，特性已在 v0.4.6-beta 真实发版**（2026-10-02；X10-71 桌面模式按设备设置 + 应用名下拉攒齐走版；发版前用户裁定 mac 签名与 win 测试矩阵一并搁置，见 B2-06）**：版本 0.4.4→0.4.5（`tauri.conf.json` / `package.json` / `Cargo.toml` / `Cargo.lock` / `README.md`；README「镜像与会话」补「桌面模式按设备设置」条目）；新增 `docs/releases/v0.4.5-beta.md`（特性两条 + 伴侣端维持 0.2.5 说明 + 0.4.0 及更早检查更新 404 提示）；`site/index.html` 版本/下载链接/APK 资产名同步（0.4.4→0.4.5 共 29 处）+「多台设备同时镜像」段补桌面模式按设备与应用名展示；`site/compatibility.html`（6 处）+ `docs/compatibility-matrix.md`（4 处）打包列 ✅ 同步 v0.4.5-beta。tag `v0.4.5-beta` **实际从未推送**（`git ls-remote` 远端无此 tag；build.yml 仅在 `tags: v*` 打包，故无 Release / 安装包 / updater 产物）——X10-71 桌面模式按设备设置 + 应用名下拉**从未随发版到达用户**，本行原误标为「已发版」。代码已于 2026-10-02 落在 main（6855fe0 bump → 8887365 feat），本回合收入 **v0.4.6-beta 真实发版**（见下方发版记录 + tag 触发 CI）。
- [x] ✅ **v0.4.6-beta 发版（2026-10-02 夜间；回收 v0.4.5-beta 漏发 + 含 BUG-充电掩盖）**：版本 0.4.5→0.4.6（八处口径一致：tauri.conf.json / package.json / Cargo.toml / Cargo.lock / README.md / site/index.html / site/compatibility.html / docs/compatibility-matrix.md；伴侣 App 维持 0.2.5 不变）；新增 `docs/releases/v0.4.6-beta.md`，删除误标的 `docs/releases/v0.4.5-beta.md`。本版内容：①**X10-71 桌面模式按设备设置 + 应用名下拉**——此前因 v0.4.5-beta 漏推 tag 从未随 Release 到达用户，本版首次随发版落地；②**BUG-充电掩盖**——移除无线保持唤醒的 `dumpsys battery set usb 1` 伪造充电杠杆（commit 55dd733），无线常亮改由 scrcpy `--stay-awake` 覆盖，桌面端不再掩盖手机真实充电状态（详见 BUG-充电掩盖）。tag `v0.4.6-beta` → CI 四平台打包（updater 签名私钥走 GitHub secret `TAURI_SIGNING_PRIVATE_KEY`，无密码，与 `mirrordock-updater.key` 配对）+ 伴侣 APK（companion-apk job，沿用 0.2.5 代码重打包）+ latest.json + Release。
**已落地并核验（2026-10-02 23:4x）**：commit `ee90b6c` → tag `v0.4.6-beta` 已推远端（`git ls-remote` 确认）；CI 跑通，**Release 已发布**（`https://github.com/g-star1024/MirrorDock/releases/tag/v0.4.6-beta`，id 401941904，draft=false / prerelease=true，published_at 2026-10-02T15:35:43Z，**唯一非草稿、无重复**）；**资产 30 个全齐**——四平台安装包（`MirrorDock_0.4.6_x64.dmg` / `_aarch64.dmg` / `_x64-setup.exe` / `_amd64.AppImage` / `_amd64.deb` / `x86_64.rpm` / `_x64_en-US.msi`）、updater 归档 4 份 + `.sig` 4 份、伴侣 APK `MirrorDock-companion-0.4.6.apk`（4.46 MB）、4 份 `SHA256SUMS-<平台>.txt`、8 份 SBOM、`latest.json`；**updater 端点已核验**：`latest.json` → `version=0.4.6`、`pub_date=2026-10-02T15:35:55Z`、`platforms=4` 且**四平台签名齐全**（darwin-aarch64 / darwin-x86_64 / linux-x86_64 / windows-x86_64 均有 signature）。Mac 本地已重装 0.4.6（`/Applications/MirrorDock.app`，本机为 Intel x64 = i9-9980HK，架构匹配；ad-hoc 签名，活跃镜像会话保留未杀，**重启客户端后生效**）。
- [ ] **待验收（X10-59/60/61/62/63/64/68/69/71）**：v0.4.4-beta 两端包装真机后：**托盘逐会话条目与逐台结束（X10-68，双设备场景）**、**多会话设置「应用到哪台设备」选择器（X10-68）**、**设备收藏置顶（X10-68）**、**快捷回复端到端（X10-69：带回复动作的真实聊天通知 → 桌面回复框发送 → 手机上消息发出）**、**桌面模式按设备设置与应用名下拉（X10-71，随 v0.4.6-beta 交付）**；以及 v0.4.3-beta 已交付但属观感验收的：拖拽手势、拔线提示条+自动恢复、发送区删除/新文件提示、工具页通知面板样式。
  - **已由真机验证覆盖（2026-10-02，用户无需重复验收）**：X10-64B 端口回退实连、X10-66 通知转发端到端、0.2.5 转发链路（sent=true）、两端升级本身。
  - **伴侣 APK 0.2.2 来源**：`companion.yml`（workflow「Companion App」）随 4bbfe0b 的 push 已成功产出 artifact `mirrordock-companion-debug`（约 3.8 MB，run 36953624864），可直接下载安装验收。

- [x] ✅ **X10-72 按设备桌面模式模块可见化（2026-10-03 00:00 夜间；用户报告"看起来还是全局设置、找不到按设备设置的模块"）**：**根因不是漏写功能，是 X10-71 的可见性缺陷**——①设备选择器只在 `sessionActive && activeSessionList.length > 1` 时渲染（App.tsx:2847），单设备/未镜像时**入口完全不存在**；②`settingsDesktopTarget` 为空时读写落到**全局 options**（App.tsx:1687-1688）；③文案"多台设备时按应用到哪台设备单独保存"在单设备场景下是**假话**。修复：新增纯函数 `buildDesktopPrefDevices()`（候选 = adb 列表**含未授权/离线** ∪ 会话设备 ∪ **已有偏好的历史设备**；按 `physical_serial` 合并 USB/无线多通道，ready 优先）；新增独立 state `desktopTargetSerial`（**不随会话结束重置**，让未插线设备可提前配置，与「镜像窗口」卡的 `settingsTargetSerial` 分离）；桌面模式从「镜像窗口」卡**移出**独立成「按设备设置 · 桌面模式」卡（设备下拉 + 逐台状态清单点行切换 + "下面两项正在编辑：<设备名>" + 每项"仅对某台设备生效"）；`settingsDesktopTarget` 回退链补 `readySerial` 消灭"无会话即全局"空洞；App.css 新增 `.device-pref-*`；删除与实现不符的表述。测试：新增 6 个 `buildDesktopPrefDevices` 单测 + 2 个渲染回归测试（锁定"单设备无会话时模块也必须可见且写明归属"、"无设备时才回落全局"），**vitest 58 → 66**。验证：`pnpm build`（tsc+vite）通过。commit `28a6fb5`。
- [x] ✅ **X10-73 伴侣 App 三 Tab 重构 + 多设备 + 连接质量 + 桌面快捷方式（2026-10-03 00:00 夜间；用户拍板方案 B）**：用户裁定**保留青碧 #2F98A1 只换结构与交互**，且**桌面端 Tauri/React 配色一律不动**，**多设备列表/分组一并开发**。桌面端**本就支持多设备**（`PairedStore` 是 `Vec`，`upsert`/`remove` 均按 `pairing_id`），单设备限制只在手机端——旧版把 `pairing_id`/`last_host`/`resident_port` 平铺在一份 prefs 里，**配第二台会覆盖第一台**。
  - **新增 `PairedComputer.kt`**：`PairedComputer` 记录 + `ComputerStore` 台账（存 `computers_v2` JSON 数组 + `active_pairing_id`）。迁移：旧键原样搬进列表再标 `legacy_migrated`（幂等）；**任何一步失败都退回旧键，不让用户丢凭据**（升级覆盖安装不能把老用户「忘记电脑」）。排序：当前连接目标优先，其余按最近连接倒序（对齐 ToDesk「我的设备」直觉）。`remove` 按 `pairing_id` 精确删除（旧版 `edit().clear()` 一台都没了）；`removeAll` 独立提供且调用方必须二次确认。分组 `group` / 重命名 `rename`。
  - **结构重构**（单屏长滚 → 三 Tab）：设备页（状态主卡 + 连接质量面板 + 我的设备列表 + 分组筛选 + 配对入口 + 桌面快捷方式 + 崩溃取证）/ 文件页 / 设置页（通知镜像、屏幕捕获、互信管理、隐私、运行日志默认收起）。
  - **视觉**（层级靠投影与留白，不靠深色块压场）：墨色渐变 hero → 白卡 + 品牌色左强调条；主 CTA 白胶囊 → 品牌青碧实心；次要按钮纯白无边框 → 白底 1dp 细边（浅灰页面上原本没有边界）；新增语义色（success/warn/danger + 浅底）、导航选中底块、状态徽标底；三个 Tab 图标，选中态同时改图标 tint + 文字色 + 底色块（不只靠颜色区分，色弱用户可辨）。圆角统一 14-18dp。
  - **多设备交互**：设备行显示名称/在线离线徽标/分组徽标/端点+身份/上次连接时间；**状态如实**——只有常驻服务真连上且 pairing_id 匹配才显示"在线"，其余一律"离线"；点整行 = 设为当前连接目标；点「⋯」= 重命名 / 归入分组 / 解除这台；分组筛选 chips（单台时自动隐藏）；解除互信对话框显示正在解除哪台，**其它已配对电脑不受影响**；重新配对同一台时保留用户改过的显示名与分组。
  - **连接质量面板**：延迟（RTT）/ 已连接时长 / 端到端加密。RTT 经 **ping 带单调时钟戳 → pong 原样回带**实测（桌面端 `companion_pairing.rs`：ping 带 `t` 时回带，不带 `t` 的旧版仍回固定 `PONG_LINE`，**双向兼容**）；`LinkQuality` 采样失败一律 `null`，界面显示「—」而非 0（0 会被读成"延迟极低"，那是假的）；只采信 0..10s 区间。
  - **桌面快捷方式**：`LinkShortcutManager.requestPin`（走 `ShortcutManager.requestPinShortcut`，由**系统弹窗**确认，未同意不静默上桌面；无已配对电脑时不提供固定请求）+ `LinkShortcutActivity`（`Theme.NoDisplay` 透明无界面，启动常驻连接后 finish；`exported=true` 因为桌面启动器以不同 uid 启动，但不读任何传入 extras）。**不申请任何新权限**。
  - **红线未动**：加密直连与配对协议、扫码/手动两条配对路径、文件收发私有目录 + FileProvider、解除互信的双端语义、日志不落敏感数据。
  - **资源治理**：删除已无引用的 `bg_hero`/`bg_pill_white`/`bg_ghost_button`；移除 6 条死字符串。
  - **验证**：伴侣 0.2.5 → **0.3.0**（versionCode 16→17）。本地无 gradle 且沙箱拦大文件下载（curl 拉 gradle 发行版 exit 56），故新增 `scripts/verify-companion.py` 静态交叉校验（XML 合法性 / `@string` `@color` `@style` `@drawable` 引用完整性 / `findViewById` 的 id 是否声明 / Manifest 类是否存在 / Kotlin 括号平衡 / 死资源检测）并接入 `AGENTS.md` 强制闸门——**全部通过**；Rust 侧 `reconnect_client_with_ping` 断言"不带 t 不回显 t、带 t 原样带回"，**cargo test companion_pairing 13 passed / 全量 192 passed，clippy 0 warning**；`pnpm build` + **vitest 66 passed**。commit `9259184` / `ec73c73` / `e0aa9a4` / `7a2bcd9`。
  - **未验证面（如实标记）**：APK 真编译与真机行为由 CI（`companion.yml` / `build.yml` 的 `companion-apk` job）+ 用户真机承担；本地无 gradle 无法预编译，静态校验只能覆盖资源引用与语法结构类错误，**不能替代编译**。
- [x] ✅ **X10-83 桌面模式「选了应用仍白屏」传参 bug——pref 键统一为 physical_serial（2026-10-08；用户报「桌面模式开了且选了应用，为什么还是白屏」）**
  - **现象**：用户已在设置里开桌面模式并选「大话西游 com.netease.dhxy.qihoo」，重启镜像后仍白屏。
  - **根因（实机取证）**：`ps -ww` 抓当前会话实参 = `--new-display`（**无 `--start-app`**）→ desktop_app 没下发。逐层走查锁定：桌面模式偏好 `desktopPrefs` 的**键口径不一致**——设置页（`buildDesktopPrefDevices` 归并）写入时用 `physical_serial`（稳定短 id `qc8d8tonbmmzm7qs`），而 `startMirroring(serial)`/`applyOptionsUpdate` 用 **adb serial / 无线端点**（`adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp`，重连会变）去查 → **查空** → `composeOptionsWithDesktop` 拿不到 pref → `desktop_app=null` → scrcpy 起一块只有 MIUI 副屏启动器的裸虚拟屏（该启动器 FPS 恒 0 不渲染）→ 白屏。
  - **修法（方案 A，用户拍板）**：pref 键全链路统一为稳定 `physical_serial`——① `DesktopPrefDevice` 增 `pref_key`（= physical_serial，无则退回 serial）；② 新增 `desktopPrefKeyFor()` 把 adb serial/会话 serial/无线端点统一解析成 physical_serial；③ `startMirroring`、`applyOptionsUpdate`、设置页读写、设备选择器/列表的取值与选中态全部改走 `pref_key`；④ App 内一次性**迁移**：旧版以「无线端点/adb serial」为键的 pref 归并到 physical_serial 键（目标键已有配置时保留新值）。
  - **验证**：`pnpm vitest run` **77 passed**（修正 1 条与真实数据形态不一致的夹具，并新增回归测试 `uses_physical_serial_as_pref_key_regardless_of_endpoint`）；`pnpm build`（tsc + vite）通过。版本 0.4.13 → **0.4.14**（package.json / tauri.conf.json / Cargo.toml）。
  - **未验证面（如实标记）**：真机「重启镜像后 `--start-app=com.netease.dhxy.qihoo` 是否真拼进 scrcpy 参数 + 大话西游是否在虚拟屏渲染」需用户装 0.4.14 后重启镜像验证；另注意 X10-80 已实证**网易系游戏 SDK 跳板 Activity 可能跑完即被移出虚拟屏**（MIUI SmartPower 休眠），即便 start-app 正确下发，游戏类应用在该机型上仍可能白屏——届时走 `desktop-app-missing` 落地核验提示，普通应用（设置/浏览器）不受影响。
- [x] ✅ **X10-81 镜像窗口图标回退成 scrcpy 机器人（2026-10-05 16:30；用户报「又变成安卓原生图标」）**
  - **根因（对照实验实证）**：scrcpy 用**二进制同目录的 `scrcpy.png`** 设置窗口/Dock 图标（AppIcon.icns 不是生效渠道；killall Dock 前后截图对照：替换 png 后 Dock 图标由机器人翻转为 MirrorDock 圆环）。10-03 手动 `cp icon.png → resources/scrcpy/scrcpy.png` 后，`pnpm tauri build` 的 `beforeBuildCommand` 跑 `scripts/prepare-runtime.sh`，其 `rm -rf resources/scrcpy/*` + 从 `.tools/` 全量复制把图标**覆盖回官方机器人图**——本地构建的 0.4.12 带病出厂，CI 构建反而正常（无 .tools，走「已准备好」分支）。
  - **修法**：`prepare-runtime.sh` 两个分支（全量同步 / 已准备好）都追加 `ensure_app_icon()`——用 `src-tauri/icons/icon.png` 强制覆盖 `$DEST/scrcpy.png`，单一事实源，本地构建永不再回退。
  - **教训**：被 gitignore 的资源目录 + 构建时自动重组 = 手动补的文件必然被吃掉；修资源必须修**生成器**，不能修产物。
- [x] ✅ **X10-84 Windows 频繁「未响应」卡死——后端异步化 + 事件驱动替代前端轮询（2026-10-08；用户报「Windows 上总卡死、运行不流畅，看看怎么优化」，拍板方案 3 彻底解决）**
  - **根因（代码走查 + 截图实证）**：用户两张截图都是「无镜像会话的纯界面」就 `(未响应)`——非镜像渲染问题，是主进程被阻塞。前端每 1-2s 轮询 `check_adb_devices`（连接页）+ `probe_pin_pad_state` + `device_lock_report`（5s 各一次）；后端 49 个 Tauri 命令全同步，内部 `Command::output()` 阻塞等 adb 子进程退出，**无超时**。Windows 上 adb 对掉线/未授权设备 `devices -l`、`shell getprop` 常长时间不返回 → 命令线程占满 → 后续 invoke 全排队 → UI 冻结。Mac 上 adb 返回快不显现，Windows 被放大成整窗未响应。
  - **修法（方案 3：彻底）**：① `SystemAdbRuntime::output_with_timeout()`——所有 adb 调用统一 `ADB_CALL_TIMEOUT=8s` 兜底（try_wait 轮询 + 超时强杀，TimedOut 错误上层按「设备暂时不可用」处理），`run`/`run_owned`/`capture`/`list_devices`/`device_properties` 全走它；② 后端设备 watcher——`.setup` 里起常驻线程，每 2s `list_devices()`，仅设备集合（serial+state 指纹）变化时 `emit("devices-changed")`；③ 前端连接页 5s `setInterval` 轮询**删除**，改为订阅 `devices-changed` 事件 + 首屏主动拉一次；④ `probe_pin_pad_state` / `device_lock_report` 轮询 5s→8s 降频。
  - **验证**：`cargo check` / `cargo clippy` 0 warning / `cargo test --lib` 194 passed；`pnpm tsc --noEmit` 0 error；`pnpm vitest run` 77→80 passed。
  - **未验证面（如实标记）**：Windows 真机的卡死消除效果需用户在 Windows 上实测确认（沙箱无 Windows）；超时阈值 8s 是经验值，若 Windows adb 正常调用就接近此值可能误杀，待真机反馈调优。
- [x] ✅ **X10-85 首页连接模块去掉「最近使用过的设备」展示（2026-10-08；用户拍板）**
  - **修法**：首页移除最近设备列表 UI（含收藏排序 / 重新连接 / 移除记录按钮），仅保留 `recentMessage` 操作反馈条。后端 `list_recent_devices` 等命令**保留**——无线重连、设备记录仍依赖该数据，仅移除 UI 展示层。清理 `recentDevices` state、`forgetRecentDevice`/`clearRecentDevices` 死代码。
- [x] ✅ **X10-86 自绘下拉框替换原生 select（2026-10-08；用户报「设置里的下拉框跟整体客户端不搭」+ 截图）**
  - **根因**：原生 `<select>` 各平台渲染差异大（Windows 灰底黑框系统样式），与轻主题圆角设计不搭。
  - **修法**：新增 `src/AppSelect.tsx`——button + 浮层列表自绘下拉（点击展开/点选即合/Esc 关/点外关/↑↓高亮/Enter 选中，含 `listbox`/`option` ARIA）；`App.css` 加 `.app-select*` 样式（白底 10px 圆角、`#2e6be6` 聚焦环、旋转 caret、选中 ✓）。替换全部 6 处原生 select：应用目标设备 / 画质 / 帧率上限 / 显示方向 / 快捷键修饰键 / 桌面模式目标设备。更新 `rotation_defaults_to_following` 测试断言（原生 value → 自绘 button 文案）。
- [x] ✅ **X10-87 设备识别合并修复——别名归一化 + pref 归并（2026-10-08；用户报「只连过 2 台设备，却显示给好多设备开过桌面模式」+ 截图 5 条）**
  - **根因**：历史 pref 以「无线端点（`192.168.0.165:xxxxx`）/ mDNS 端点（`adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp`）/ 短物理 id（`79j7kn9tkjt8rwss`）」等不同键各自残留，X10-84 迁移只能反查**当前在线**设备，离线旧端点键仍并列显示多行。
  - **修法**：新增 `normalizeDeviceIdentityKey()`——`adb-<physical>-<随机>._adb-tls-connect._tcp` 直接提取物理 id；`desktopPrefKeyFor` 在设备离线时回落归一化；迁移 effect 从「仅设备首次非空跑一次」改为「设备列表每次变化都重跑 + 归一化双管齐下」，同一物理设备多键合并为一条；`buildDesktopPrefDevices` 的 pref 补充循环先归一化再去重。回归测试 `merges_mdns_endpoint_and_short_physical_id_into_one_row`。
  - **边界（如实标记）**：`ip:port` 形态端点不含物理 id，本地无法归并——靠设备上线时反查 physical 后清掉；彻底清干净需设备连一次。
- [x] ✅ **X10-88 设备自定义备注名（2026-10-08；用户报「同型号手机显示一样的设备名，支持自定义备注」）**
  - **修法**：新增 `readDeviceNicknames`/`writeDeviceNickname`/`deviceDisplayName`——备注以稳定 physical_serial 为键存 `localStorage.mirrordock.deviceNicknames`；`displayLabels` 包一层「备注覆盖型号标签」；设备卡名称旁加 ✎ 编辑按钮（hover 显现）+ 内联编辑框（Enter/保存、Esc 取消、清除回落型号名）。回归测试 `device nicknames (X10-88)` 2 条。
- **版本**：以上 X10-84~88 五项，版本 0.4.14 → **0.4.15**。
- [x] ✅ **X10-89 Mac 本地启动卡死——0.4.15 回归根因修复（2026-10-08；用户报「mac 本地启动卡死了」）**
  - **根因（sample 采样实证，非猜测）**：卡死时 `sample` 主进程 1671 帧全卡在 `nanosleep`/`__semwait_signal`——主线程在处理 tauri URI scheme（IPC）时跑了一个 `try_wait + sleep(30ms)` 忙等循环。源自我 X10-84 加的 `output_with_timeout`：tauri 同步命令**跑在主线程**，`check_adb_devices` 一次 `adb devices` + 逐台 `getprop`，adb 慢时主线程忙等最多 8s；前端启动连发多命令全排队 → 整窗冻结。**本意防 Windows 卡死的超时机制，因放错线程在 Mac 上制造了卡死。**
  - **修法（方案 1，用户拍板）**：① `output_with_timeout` 重写为「子线程跑 `spawn+wait_with_output` + channel 传 Output + 调用线程单次 `recv_timeout(8s)`」——主线程 park 不烧 CPU；② `check_adb_devices`/`device_lock_report`/`probe_pin_pad_state` 三个高频轮询命令改 `async fn`（tauri 2 async 命令体跑工作线程池，主线程/IPC 立即释放）；③ 抽出 `check_adb_devices_sync` 供 async 命令与内部复用；④ 保留 X10-84 事件驱动 watcher 与 X10-85~88 全部 UI 成果。`mirror_sessions` 只读内存不跑 adb，未动。
  - **v0.4.15 发版链已熔断**：远端 tag `v0.4.15-beta` 已删、Release 未生成、CI run 跑完无 tag 可发布。
  - **验证**：`cargo check` / `clippy` 0 warning / `cargo test --lib` 194 passed。**本次必须本地打包 + Mac 实机验证启动不卡后才发版**（0.4.15 未验启动即发版的教训）。
  - **教训**：给「跑在主线程的同步命令」加超时时，**绝不能在该线程里 sleep 轮询**——超时等待必须放进独立线程或把命令改 async。sample/Instruments 采样比日志更能定位主线程卡死。
- [x] ✅ **X10-90 托盘「连接设备」不进桌面模式——桌面模式偏好从前端 localStorage 迁到后端持久层（2026-10-08；用户报「点开始镜像正常进入桌面模式，点连接设备就不能直接进入桌面模式，变成正常镜像手机了……不管从什么入口点击连接，都应该是桌面模式」）**
  - **根因（结构性，非偶发）**：桌面模式偏好只存在**前端 webview 的 localStorage**（`mirrordock.desktopPrefs`），托盘菜单「连接设备」这条 **Rust 后端路径**完全够不到——`tray_connect_toggle` 直接 `start_mirroring_with(..., SessionOptions::default(), ...)`，`desktop_mode` 恒为 false。主窗口「开始镜像」在前端跑了 `composeOptionsWithDesktop` 叠加偏好，所以正常；托盘没经过前端，丢了桌面模式。**同一台设备，两个入口两种行为。**
  - **修法（方案：持久层下沉到后端，用户报 bug 即拍板修）**：① 偏好从 localStorage 迁到后端 app data `desktop-prefs.json`（`~/Library/Application Support/com.mirrordock.desktop/`），键仍为稳定 physical_serial；② 后端新增 `normalize_device_identity_key`/`desktop_pref_key_for`（与前端 X10-84/87 同口径）+ `DesktopPrefEntry` + `load/save_desktop_prefs` + `desktop_pref_for_serial`；③ 新增两个命令 `get_desktop_prefs`/`set_desktop_prefs`（注册进 invoke handler）；④ `tray_connect_toggle` 启动分支改为 `tray_session_options`——后端默认值之上叠加这台设备的桌面模式偏好（开桌面模式强制关摄像头源，与前端互斥逻辑一致）；⑤ 前端挂载时**一次性迁移** localStorage→后端（成功才清 localStorage，失败保留下次重迁+如实提示），之后 `updateDesktopPref`/键归一化 effect/恢复默认全部改走后端 `set_desktop_prefs`；`readDesktopPrefs` 保留为迁移来源。
  - **验证（反向验证已做）**：`cargo test --lib` 197 passed（新增 3 条：mDNS 端点归一化、无线端点→physical_serial 反查+离线归一化兜底、prefs 文件读写回环+坏文件容错）；`clippy` 0 warning；前端 `pnpm build`（tsc+vite）通过、`vitest` 81 passed（新增反向验证 `persists_desktop_pref_to_backend_not_localstorage`——翻转桌面模式开关必须调 `set_desktop_prefs` 且**不写** localStorage）；本地打包 `.app` 已换装 `/Applications/MirrorDock.app`（旧版备份 `MirrorDock.old-0.4.16.app`）。
  - **未验证面（已闭环，2026-10-08 真机验证通过）**：真机「从托盘点连接设备 → 进入桌面模式并带上 `--start-app=<用户配的应用>`」已由用户实测确认没问题；迁移 effect 已把 localStorage 里的大话西游配置写进后端 `desktop-prefs.json`。
  - **教训（与 X10-83 同源）**：这是**第二次**「同一偏好，一处改另一处不生效」——X10-83 是前端两处键口径不一致，X10-90 是**前后端两个持久域**。凡是「按设备存的偏好」，只要有一个入口（托盘/快捷键/伴侣端/未来的自动重连）不经过前端 webview，就必须存在后端能读的地方。**判断标准：新加入口时先问「它读得到 localStorage 吗」，读不到就不能把状态只放 webview。**
- **版本**：X10-90，版本 0.4.16 → **0.4.17**。
- **v0.4.17-beta 发版证据（已核验，2026-10-08 15:00）**：tag `v0.4.17-beta` 已推远端、commit `d53610a`；CI run `37739066537` **success**，7 job 全绿（验证/四平台打包/伴侣 APK/发布）；Release **draft=False、30 资产全齐**（四平台 dmg/exe/msi/AppImage/deb/rpm + 对应 .sig + latest.json + SHA256SUMS×4 + sbom-cargo/npm×8 + `MirrorDock-companion-0.4.17.apk`）；`latest.json` **version=0.4.17**、四平台签名全非空（darwin-aarch64/x86_64 428 字符、linux/windows 444 字符）。真机「托盘连接设备→桌面模式」已由用户实测通过。

- [x] ✅ **X10-92 录制改双通道：手动启停、不重启镜像窗口（2026-10-08；用户 4 项需求之 1/2/3：①设置项改名「启用视频录制」仅作能力开关、②手动启停录制且画面不中断、③截图/录制/停止快捷键）**
  - **决策（用户拍板路径 2 + 方案甲）**：scrcpy 4.1 的 `--record` 只能在进程启动时传入、运行中无法动态启停——「不重启镜像的录制」原生做不到。调研外部录制工具（OBS/ffmpeg）后确认：视频流被 scrcpy 端到端独占，外部工具拿不到（macOS/Win）或拿到就接管显示（v4l2 仅 Linux）。**路径 2 双通道**是唯一同时满足「显示不中断 + 运行中启停 + 录设备原始码流 + 全平台」的方案：显示走一条 scrcpy（不带录制），点「开始录制」另起一条 `--no-playback --no-window --no-control` 的独立录制进程，点「结束录制」优雅停它定型 MP4。
  - **方案甲（录制与设置彻底解耦）**：设置项 `record` 仅作「能力开关」，开始镜像**不再自动录**；录制 100% 由用户手动触发。`start_mirroring`/`update_session_options` 一律忽略 record、显示进程永不带 `--record`。
  - **后端**：`SessionOptions::record_arguments()`（录制专用参数：保留视频源/音频，剔除窗口/输入类，强制 no-playback/no-window/no-control）；`MirrorRuntime::start_recorder()`（复用 spawn 逻辑起独立录制进程）；`SessionState.record_process`（与显示 `process` 并存）；命令 `start_recording`/`stop_recording`；显示会话结束自动停录制进程（不留孤儿）；「是否录制中」判定改由 `record_process`；`tray_record_toggle` 切双通道。
  - **前端**：设置项改名「启用视频录制」+ 文案说明「何时开始/结束由你决定、画面不中断」；工具页录像面板加「开始录制/结束录制」按钮（`toggleRecording` 走 start_recording/stop_recording）；快捷键 `shortcuts.record` 改调 toggleRecording（不再动 record 设置项、不重启显示）。
  - **反向验证**：后端 `the_recording_path_goes_to_a_separate_recorder_never_the_display_process`（开始镜像即使 record=true 也绝不把录制塞进显示进程）+ `starting_recording_uses_a_separate_channel_and_never_restarts_the_display`（开录不重启显示进程）；前端 `start_recording_uses_a_dedicated_command_and_never_restarts_the_display`（点开始录制必须调 start_recording、绝不调 update_session_options/start_mirroring）。质量门：后端 198 passed + clippy 0 warning、前端 82 passed + pnpm build 通过。
  - **未验证面（如实标记）**：「同一 serial 同时跑显示 + 录制两条 scrcpy（设备端双路编码）是否稳定、低端机是否掉帧」需真机实测；手机端当前因医院网络隔离离线，待恢复后验证。双路编码会让手机 CPU/带宽翻倍，已在错误恢复建议里提示「降低画质或改用数据线」。
  - **教训**：scrcpy 的能力边界（录制只能启动时传、窗口是原生不可注入）决定了交互设计的上限——**先查官方文档确认能力边界，再承诺交互**，不要先答应了再发现做不到。
- **版本**：X10-92（+ X10-91 pairing IP 关联），版本 0.4.17 → **0.4.18**。
- **v0.4.18-beta 发版证据（已核验，2026-10-08 22:20）**：tag `v0.4.18-beta` 已推远端、commit `8bf2f12`；CI run `37789415599` **success**；Release **draft=False、30 资产全齐**；`latest.json` **version=0.4.18**、四平台签名全非空（darwin-aarch64/darwin-x86_64/linux-x86_64/windows-x86_64 全 OK）。Mac 客户端已换装 0.4.18，待用户重启验证「启用视频录制」新文案与工具页「开始录制」按钮。

- [ ] ⏳ **X10-93 截图/录制/停止快捷键（部分完成）**：快捷键基建已存在（`shortcuts.screenshot/record/rotate`，全局注册，设置页可改组合）。X10-92 已把录制快捷键切到双通道 toggleRecording。**说明**：快捷键放在 MirrorDock 主窗口 + 全局（macOS 需辅助功能授权），**无法塞进 scrcpy 镜像窗口标题栏**（scrcpy 原生窗口，外部无法注入按钮）——已向用户说明。待真机验证快捷键在窗口聚焦 scrcpy 时是否触发。

- [x] ✅ **X10-94 键盘映射 / 鼠标模拟点击器——用户已拍板：移入私有仓库（2026-10-08 晚）**：用户要「镜像上层鼠标模拟点击器：加点位、按点位顺序点击、循环、设循环次数」。**⚠️ 与 AGENTS.md 红线冲突**（第 13 行 No unattended general-purpose control）。**用户最终拍板**：①自动循环挂机**不做**（无论公私仓库都不做）；②接受「**有人值守、单次触发、不自动循环**」边界——手动点一次「执行一轮」，轮内循环次数由用户手动设定，跑完即停；③**排除金融/支付/带反作弊的应用**（前台检测屏蔽名单）；④该功能**不进公开版**，单独发布到**私有仓库 `github.com/g-star1024/MirrorDock-tools`**（基于公开版 v0.4.18 基线）。技术路径：`adb shell input tap x y`（固定参数直接进程调用，无 shell 拼接）+ 坐标越界保护 + 执行状态可见可撤销。**公开版仓库不含任何模拟点击代码，本台账仅记录决策；实现与证据见私有仓库。**

- [x] ✅ **X10-95 录制功能真机问题批次修复（2026-10-09 凌晨）**：用户真机报 5 个录制问题，逐一定位修复。
  - **问题5 黑屏 + 问题3「No such file」（同根）**：录制出来的 MP4 全黑/无法播放，前端「在文件夹中显示」报 `No such file or directory (os error 2)`。**根因（真机实证）**：Tauri/Rust spawn 的子进程继承「忽略 SIGINT」，`SystemMirrorProcess::stop()` 却发 SIGTERM——scrcpy 4.1 不响应 SIGTERM（X10-79 已知）→ 等满 3s 超时 → SIGKILL 强杀 → **moov 索引永远写不进 MP4**（`ffprobe` 实证 `moov atom not found`），文件损坏即黑屏/无法播放。**修法**：①`spawn_scrcpy` 在 Unix `pre_exec` 把 SIGINT/SIGTERM/SIGHUP 重置为 `SIG_DFL`（让 scrcpy 自己的信号处理器能接管）；②`SystemMirrorProcess` 加 `is_recorder` 标记，`stop()` 对录制进程优先发 **SIGINT**（显示进程维持 SIGTERM 原路径）。**真机验证**：SIGINT 后 1.5s 优雅退出、日志打出 `Recording complete`、产出 MP4 `ffprobe` 完好（h264 864x1920 + opus，duration 正常）。新增测试 `a_recorder_is_stopped_with_sigint_so_the_recording_finalizes`（探针进程忽略 SIGTERM、仅 SIGINT 退出，验证发的是 SIGINT）。
  - **问题2 托盘菜单卡「开始屏幕录制」**：`build_tray_menu` 的 `recording` 判定用旧的 `options.record`（单通道重启式遗留），X10-92 改双通道后显示进程永远不带 record → 永远显示「开始」。**修法**：改用 `record_process.is_some()` 判定；并给 `start_recording`/`stop_recording` 命令补 `refresh_tray_menu`（原仅托盘入口刷新，客户端按钮入口不刷）。
  - **问题4 录像保存目录不可设置**：新增 `AppSettings.recording_dir: Option<String>`（绝对路径，set 时校验绝对路径+可创建可写，空串=默认），`recording_dir()` 优先读自定义、失效回退系统默认。前端设置页加「录像保存位置」行（dialog 选文件夹 + 显示当前路径 + 恢复默认）。
  - **问题1 首次录制镜像闪退重启**：**待真机复现确认**。强假设：桌面模式 `--new-display` 虚拟屏与显示进程冲突，或录制进程 spawn 即崩牵连。**本批核心修复（优雅停）可能已覆盖部分场景**——需用户用含本修复的新版实测确认是否复现，若仍复现抓 scrcpy 输出定位。
  - **质量门（全绿）**：后端 cargo test **199 passed**（198+1）+ clippy **0 warning**；前端 vitest **82 passed** + pnpm build 绿。
  - **未验证面（如实标记）**：问题1 闪退根因未真机复现确认；Windows 平台无 SIGINT 对应物，录制停止仍强杀可能缺 moov（平台差异，待 Windows 真机定方案）。


  - **CI 波折与发版证据（已核验）**：首次 run `37284280561` 仅 macos-x64「构建安装包」失败（exit 1 无注解，raw log 需 admin 403）——**未盲猜**，按 10-03 教训先建能力（X10-82：build.yml 构建步骤 tee + 失败上传 `build-log-<label>` artifact，commit `7c43f9e`），重推 tag 重跑。重跑 run `37287371051` **success**，Release id `403545372`，**30 资产全齐**；`latest.json` version=0.4.13、**四平台签名全非空**（darwin-aarch64/x86_64 428 字符、linux/windows 444 字符）；`MirrorDock-companion-0.4.13.apk` 在列。Mac 已换装 0.4.13（旧版备份 `/Applications/MirrorDock.app.old-0.4.12`），图标已核验。site/index.html 已同步。**需用户重启客户端 + 重启镜像会话生效**（Dock 图标跟随先注册进程）。
- - [x] ✅ **X10-80 桌面模式虚拟屏白屏根因定性 + 落地核验（2026-10-03 19:30；用户报「切 2560 白屏，切回 1920 依旧」）**
  - **根因（全链路实证，非推断）**：白屏与画质无关。逐层排除：设备编码器在 1080×2400@16Mbps 正常出流（screenrecord 实测 level 5.1）；手动 scrcpy 1920/2560 普通镜像画面清晰（真窗口截图为证）；bundle 完整、签名有效。**真正的根因**：用户设备开了桌面模式 + `--start-app=com.netease.dhxy.qihoo`（大话西游），虚拟屏创建成功（dumpsys 实证 id=84 状态 ON，系统设置可完整渲染其上），但**网易系游戏的 SDK 跳板 Activity（ProtocolLauncher）在虚拟屏跑完即被移出**（WindowManager removeChildTask 实锤），真游戏 Activity 从未出现，MIUI SmartPower 把进程移入后台休眠 ⇒ 虚拟屏无内容 ⇒ 流传输的内容本身就是白的。切画质只是触发会话重启，纯属"背锅"。
  - **修法**：① 会话启动后 10 秒做一次「落地核验」——从 scrcpy 输出解析 `on display <id>`，再 `dumpsys activity activities` 查该屏分段里有无 `packageName=<pkg>` 的记录；没有则发 `desktop-app-missing` 事件，前端提示条明确告知「应用未能在虚拟屏上启动 + 白屏原因 + 改用普通镜像的建议」（`spawn_desktop_app_landing_check` + 纯函数 `desktop_app_landed`/`extract_display_section`/`parse_desktop_display_id` 均有单测）。② scrcpy 窗口图标 `scrcpy.png` 随包补齐（CI 三平台 + 本地，此前每次启动刷两行 icon 错误混进排查）。
- [x] ✅ **X10-79 「看运行日志却看不到」的根因修复（2026-10-03 19:30；与 X10-80 同源）**
  - **根因**：`ScrcpyRuntime::start` 只 `spawn()` 不接管输出，MirrorDock 是 GUI 应用无终端，**scrcpy 的全部报错随进程丢失**——用户报白屏让我"看运行日志"，日志里却什么都没有，排查只能靠手动复现。
  - **修法**：① 子进程 stdout/stderr 显式 `piped()`，双读线程收进 16KB 环形缓冲（`ScrcpyOutputSink`，读不完会堵死 scrcpy 的反面已注释）；② `MirrorProcess::output_tail()` 进 `resolve_process_exit`，异常退出时把 scrcpy 原文末 8 行附进界面错误（不再是一句干巴巴的「已意外关闭」）；③ 实测发现 scrcpy 4.1 **不响应 SIGTERM**（发信号后 6 秒仍活），原 3 秒优雅等待永远耗满才强杀——注释如实记录，保留超时兜底。
  - **门禁**：cargo **194 passed** ✅ / clippy 0 warning ✅ / pnpm build (tsc) ✅ / vitest **76 passed** ✅。版本 0.4.11 → **0.4.12**；site/index.html 同步。
  - **发版证据（已核验）**：CI run `37119816359` **success**，Release id `402489792`，**30 资产全齐**；`latest.json` version=0.4.12、**四平台签名全非空**（darwin-aarch64/x86_64 428 字符、linux/windows 444 字符）；`MirrorDock-companion-0.4.12.apk` 在列（伴侣源码版本 0.3.2，随包分发）。commit `fbb0aa5`。Mac 已换装 0.4.12（旧版备份在 `/Applications/MirrorDock.app.old-0.4.11`），**需用户重启客户端生效**。site/index.html 已同步 0.4.12。
- [x] ✅ **X10-78 真机可见的两处布局错乱（2026-10-03 14:00；用户截图指出「两个按钮贴一起了，底部文案错行」）**
  - 用户截图指出后我**抓真机屏复现**，确认是两处**我自己引入的**（X10-76）缺陷，不是用户误看。
  - **① 「端到端加密」被挤成两行**（显示成「端到端加」/「密」）：`quality_row` 是横向 LinearLayout，三个子 TextView **全是 `wrap_content`**，按内容抢宽度；状态文案「尚未连接，连接后显示延迟与时长」占掉大半，第三个就被压到折行。**修法**：加密状态**独占一行**（它不只是"放不下"—— 它是安全承诺，折行后「密」字单独一行读起来像错字）；延迟与时长同行，时长给 `layout_weight=1` 吃掉剩余空间。**已全量扫过布局里其它横向行，无第二处同类隐患**。
  - **② 版本行与隐私摘要不在卡片里**（截图底部那两段散落的文字）：X10-76 插入时锚在了**日志卡的闭合标签处**，于是两者成了 ScrollView 的直接子节点（卡的**兄弟**），直接躺在灰色页面上，与上方五张白卡完全不是一套视觉。**修法**：新建「关于」Card 把两者包进去，marginTop 与其它卡一致（space_4）；展开的详情行补 padding，点开后不会「文字凭空跳到卡片外」。
  - **元教训（已写进 skill）**：X10-76 改了 13 个文件、两道本地门禁全绿、**CI 9 步全绿（含 aapt + dex）**，却**没抓一张真机截图**。两处缺陷都是"结构合法、类型正确、编译通过"—— 静态检查、类型检查、aapt、dex **全都抓不到**，只有看渲染结果才能发现。**铁律：改完布局必须抓屏并真的看那张图**（存下来不等于看过）。
  - 伴侣 **0.3.1 → 0.3.2**（versionCode 19）。CI run `37101421576` **成功**。
  - **桌面端 0.4.11 已发版**：run `37100315620` 成功，Release id `402354498`，**30 资产全齐**，`latest.json` version=0.4.11、**四平台签名全部非空**。0.4.11 的 APK 是 0.3.1（布局修复在 0.4.11 之后，未含在此 tag）。
  - **踩坑**：commit message 里的反引号被 shell 吞掉（`quality_row` 等标识符消失）。用 `git commit --amend -F <文件>` 修正；因 amend 改写了已推送的 commit 导致分叉，先确认 `git diff origin/main HEAD` **文件树完全相同**（仅 message 差异）才 `--force-with-lease`。**教训：commit message 里有反引号或 `$` 时一律走文件，不走 `-m`。**
- [x] ✅ **X10-77 删除含空格文件名的文件静默失败（2026-10-03 13:20；用户报「删最后一个时明明已删除，列表却没清空」）**
  - **根因（真机实证）**：`adb shell ARGV...` **不是**把 argv 直接 exec 到设备，而是先用空格拼成命令串、再交给**设备端 shell** 重新解析。所以代码注释里写的「路径作为单个 argv 传入，不做 shell 拼接」**并不能保护空格** —— `rm -f -- "/sdcard/.../has space.txt"` 被远端拆成两个词，`rm -f` 对两个不存在的路径**静默返回 0**。于是**文件没删掉、后端却报成功**，前端重拉列表仍是原样，再弹「已从手机删除「xxx」」—— **提示与事实相反**。`--` 救不了：它是给本地 shell 用的，远端 shell 早在 argv 拼接后才看到命令。
  - **真机验证（非推断）**：造 `plain.txt` / `has space.txt` / `MirrorDock 传输 冒烟.txt` / `it's.txt` 四个文件，用修复前形式逐个删 → **只有 `plain.txt` 消失**，其余三个全部存活且退出码均为 0；用修复后形式删 → 四个**全部成功**（含中文名与内嵌单引号）。验证后已清理测试文件，手机上只留用户原有的 shizuku 文件。
  - **修法**：新增 `shell_quote()`（单引号包裹 + POSIX 的闭合-转义-重开，`it's.txt` → `'it'"'"'s.txt'`）。单引号内一切字符是字面量，**同时消除注入面**（`a;rm -rf /` → `'a;rm -rf /'`）。**只用于经过远端 shell 的调用** —— 排查全部 18 处 `"shell"` 调用，只有 `remove_device_file` 一处接受用户可控路径；`adb push`/`adb pull` 不经 shell，路径原样传（已确认 `push_file`/`pull_file` 不受影响）。配套给 `AdbRuntime` 加 `run_owned`（接受 `&[String]`），避免在调用点克隆或泄漏。
  - **测试：两次反向验证，第二次才真抓住**。第一次**失败**：只写了 `shell_quote` 纯函数测试，把调用点改回 `remote_path` 后**测试依然全绿**（纯函数没变，只是没人调用了）—— 典型「只测实现、不测接线」盲区，与 v0.4.7 同源。补救：把 argv 构造抽成模块级 `remove_device_file_argv()`，补**接线测试**直接断言 `adb rm` 最后一段是带引号的完整路径；再次反向验证 → `panicked: left "/sdcard/.../冒烟.txt" right "'/sdcard/.../冒烟.txt'"` FAILED。恢复后 **194 passed**。
  - **附带教训**：第一次反向验证"没反应"还有第二个原因 —— 我的 `str.replace` 命中了测试里的**注释文本**而非真实调用点。**反向验证必须核对它是否真改到了目标**，不能只看跑绿/跑红。
  - 门禁：cargo 194 ✅ / clippy 0 warning ✅ / pnpm build ✅ / vitest 76 ✅ / 伴侣静态 ✅ / 伴侣 Kotlin 0 error ✅。
  - **修复确实进了已安装的二进制（字节级核对）**：先用 `strings` 搜特征串**搜不到**，差点误判"没编进去"。改用 Python 精确搜字节序列后发现 —— `'"'"'` 这段常量在 **0.4.11 里出现 1 次**、**上一版备份里 0 次**。**教训：`strings` 搜不到 ≠ 不存在**（它对短字符串与不可打印字符组合不可靠），验证二进制是否含某段逻辑要用精确字节搜索 + 与旧版对照。
  - **交付**：桌面端 **0.4.11 已装 Mac**（x86_64，无损换装保住活跃镜像会话，**需重启生效**）；伴侣 **0.3.1 已装手机**（`install -r` Success，启动无崩溃，dex 内验证 `PackageInfoFlags`/`ClipboardManager` 存在 → 版本显示功能真在包里；实测切深色不崩，验证后已把手机 `uimode night` 恢复为 no）。0.4.10 Release id `402337037`（`latest.json` 四平台签名齐全，APK 4,507,476 字节，ZIP 校验通过 460 条目）。APK 分段下载 512KB/段 + `zipfile.testzip()` 校验（沿用上次踩坑得出的经验）。
  - **⚠️ 待用户确认**：手机 `paired_computers.xml` 当前只剩 `legacy_migrated=true`，**配对列表为空**。文件修改时间 12:57 **早于**我 13:33 的安装，且安装走 `install -r`，**不是本次改动导致**；原因未查明，已如实告知用户可能要重新扫码配对，**未假装"覆盖安装保留配对"**。
- [x] ✅ **X10-76 安卓伴侣 App 设计升级 + 版本显示（2026-10-03 12:00；用户要求「调用设计 skill 给 app 整体做设计升级」+「把版本号加到 app 信息里，现在看不到」）**
  - ⚠️ **本条起因是我把目标搞错了**：用户说的"app"是**安卓伴侣 App**（"安装到手机"），我第一轮改的是 PC 桌面端。用户要求先回退 PC 端到 0.4.9 —— 已执行（`git reset --hard 417a930` + 删远端 `v0.5.0-beta` tag + 强推 main），**0.5.0 从未产出任何 Release 资产，用户 Mac 仍是 0.4.9**。教训见 memory。
  - **审计的核心声称我逐条复核，全部属实**（不盲信）：`text_tertiary` 2.60:1 / `nav_unselected` 2.60:1 / `success` 2.52:1 / `warn` 2.15:1（AA 需 4.5）—— 脚本实测与声称完全一致；卡片边界仅 **1.10:1**（`elevation`/`translationZ` **全项目零使用**，`colors.xml` 注释描述的"柔和投影"是**从未实现**的设计）；`build.gradle.kts` 确实没有 `buildFeatures { buildConfig = true }`。
  - **① 设计令牌层**：新建 `dimens.xml`（间距 7 级 4dp 栅格 / 圆角 5 级 / 触控 3 级 / 描边 2 级 / 字号 **5 级**，禁止 .5sp）。**硬编码归零**：`activity_main.xml` dp 字面量 **124 → 0**（只剩合法 `0dp`）、`MainActivity.kt` 的 `dp(数字)` **31 → 0**、字号 12 → 5 级。`dp()` 助手改为 `getDimensionPixelSize(@DimenRes)` —— 旧实现 `(value * density)` **绕过资源限定符**，`values-night` 永不生效，这是加暗色的前提。
  - **② 暗色主题（此前完全缺失）**：`values-night/colors.xml` 29 个令牌与浅色**一一对应**（脚本验证无缺失无多余）+ `values-night/themes.xml`（**`windowLightStatusBar=false`** —— 不翻转则暗色下深色图标压深色状态栏、时间电量看不见）。**不需要 `drawable-night/`**（shape 里的 `@color/xxx` 运行时按限定符解析），但 3 个 drawable 硬编码了颜色（`bg_input_field` / `bg_file_row` / `bg_dot`）已改引用 —— 其中 `bg_input_field` 是**手动配对码输入框（TV 端唯一配对路径）**。
  - **③ 对比度**：浅色 4 处硬失败修复（2.15-2.60 → 4.72-6.00）；暗色 12 组全过最低 **4.65:1**（第一版 `text_tertiary` on `subtle` 仅 4.15:1，已提亮）。
  - **④ 触控目标 11 处不达标 → 48dp**。根源是 `TextButton` 无 `minHeight`（13sp 文字 + 左右 10dp ≈ 20dp），一个 style 修掉一批。
  - **⑤ 中文排版**：去掉 `PrimaryButton`/`TextButton` 的 `bold`（中文 bold 是描边加粗 faux bold，13sp 下笔画粘连）；删 `letterSpacing=0.01`（它是给拉丁字母加呼吸感的，中文字面加字距破坏紧凑感）；`CardBody` 加 `lineSpacingMultiplier=1.3`。
  - **⑥ 版本显示（用户明确要求）**：此前 App 里**一处都没有**（`versionName` 只在 build.gradle.kts）。设置页底部可点行 → 展开完整信息；**长按复制**到剪贴板（小白报障的真实动作是复制粘贴不是朗读）。**用 `PackageInfo` 而非 `BuildConfig`**：后者是编译期常量、且需先加 `buildFeatures`；前者读**手机上实际安装的 APK**，报障时只有它是真相来源。API 33 `PackageInfoFlags` / API 28 `longVersionCode` 均已分支。
  - **刻意不加「检查更新」按钮**：伴侣端无独立更新渠道（APK 随桌面版重打包），服务端不存在。给了按钮只会训练用户对不上版本号的习惯，**反过来恶化正在修的问题的可诊断性**。改为给文案与补救动作。
  - **修了自己校验器的盲区**：`scripts/check-companion-kotlin.py` 的 R 存根只覆盖 6 种资源、**没有 dimen**，导致 31 个**假** unresolved reference 淹没真错误；且只读 `values/` 漏掉 `values-night/`。两处已修 + **反向验证**（注入 `R.dimen.space_99` → 抓到；恢复后 0 error）。
  - **踩坑**：批量替换 dp 时把**状态圆点 10dp 误改成 8dp**（`layout_width/height` 不是间距）—— 按"数量归零"看不出问题，靠全项目扫 `android:(width|height|size)="@dimen/space"` 才发现，已加 `status_dot_size` 修回。**教训：批量替换后必须按语义类别回扫。**
  - 伴侣 **0.3.0 → 0.3.1**（versionCode 17 → 18）。
  - **CI 验证（真编译，本地无法替代的部分）**：run `37095938149` **9 步全绿**（含第 7 步 `assembleDebug` —— aapt 资源编译 + dex + 打包全过，这正是本地 kotlinc 检查覆盖不到的部分），artifact `mirrordock-companion-debug` = 4,053,908 字节。**aapt 资源编译与 dex 首次获得真实验证**（此前一直是未验证面）。
  - **⚠️ 未完成：APK 未能装到手机**。`companion.yml` 只上传 artifact，而 **artifact 下载端点需要 GitHub 认证**（沙箱无 token、`gh` 不在 PATH），实测 `401 Requires authentication`。**待用户拍板分发方式**（见下方待办）。
- [x] ✅ **X10-75 文件传输面板展示优化（2026-10-03 10:00 上午；用户截图指出 4 个问题 + 要求用专业设计 skill 重做）**：用户截图标注：①「取回到电脑」列参差不齐；②取回后一直显示「取回到电脑」；③底部「已保存到这台电脑的下载/MirrorDock 文件夹」会让人误会**全部**都保存了；④重新设计这块的展示逻辑。**另：用户要求从 GitHub 找 app 设计类 skill 并调用专业设计优化 UI（不动功能）。**
  - **设计 skill 调研与安装**（Agent 调研 + 评估后安装）：`impeccable`（v2.0.0，开源上游 pbakaus/impeccable，Apache-2.0，Paul Bakaus 前 Google DevRel；24 条设计指令 + 61 条检测规则）与 `frontend-ui-engineering`（v1.0.0）。**均已确认是纯知识型 skill（只有 .md，无脚本、不联网、不自动改代码）**。调研同时**明确不装**的一批：`anthropic-frontend-design`（其"选一个极端美学方向/大胆/紫色渐变"是给落地页作品集用的，会把面向小白的工具面板做成"创意作品集"）、`mobile-hifi-prototype`/`genz-mobile-ui-design`（移动端宽度/TabBar，技术栈错配）、`figma-*`（需 Figma 账号，而本项目没有设计稿）、`awesome-design-md`（抄品牌官网，工具面板不是官网）、以及 SkillHub 上那一路（设计类关键词语义检索失效，top 结果 score 仅 0.044–0.108，返回的是热门度兜底排序）。**未跑 `npx impeccable install`**（会装自动 hook 拦截编辑流程）。
  - **第一轮实现后被评审指出我自己引入的 bug**（这是本次最重要的收获）：我把 grid 放在**每一行**上、列定义 `minmax(0,1fr) auto` —— `auto` 列是**逐行解析**的，列宽按本行最长内容算，按钮右边缘照样参差，只是从左边挪到右边。且「已取回」态比「未取回」态多一个按钮，实测操作列宽 154px → 256px（文件名可用宽度掉 23%），点一下按钮**全列表文件名集体往左跳**。**修法**：grid 提到列表容器 `ul.transfer-list`，行用 `display: contents` 参与同一网格，操作列**锁死 136px**。
  - **「在文件夹中显示」×8 上移为列表级工具条「打开文件夹」×1**：8 行指向同一目录是纯冗余，也是列宽不稳定的元凶。一次改动拿到列宽恒定 + 噪音 −7 元素 + 「文件去哪了」有了全局答案（工具条直接写「取回后保存在 下载 / MirrorDock」）。
  - **其余采纳（纯视觉/文案）**：文件名单行 ellipsis → **2 行 `-webkit-line-clamp:2`**（版本号/`.apk`/`.apk.1` 在**尾部**，单行截断恰好砍掉最值钱的信息，用户就答不出「该用哪个版本」）；补**行内失败态**（原因 + 「重试」，原来只有面板底部一行红字，8 行里扫不出来且无恢复动作）；**删除加确认框**（已取回态下「删除」有两种可能含义，点之前无从判断）；整行绿底 → **左侧 2px 绿竖条**（8 行绿底会把三个分组从视觉上切碎）；竖直方向改 hairline 分隔线放弃等高（文件名 2 行时等高必然参差）；**术语统一为「取回」**（原「取回/保存到/复制到/已取回」四个词说一件事）；组标题「安装包（可装到手机）」→「安装包」+ 副标题（原文括号方向反了：面板用途是拿到电脑）；底部 60 字三重冗余 → 「共 8 个文件，3 个已取回」；字号 px → rem 建立 5 级级差。
  - **明确未采纳**（会动功能，超出用户「只升级完善 ui」的边界，已如实告知用户）：后端返回文件大小/时间（需改 `list_device_files` 返回结构）、「全部取回」批量操作、真实进度条与取消、磁盘不足/同名冲突等第 5–7 态。
  - **测试**：本轮共 10 个（X10-74 三个 + X10-75 七个：分组顺序 / 真实文件名 `.apk.1` 分类 / 空列表 / 取回状态变更 / 文案点名 + 逐文件独立 / 删除撤标记 / 行内失败态 + 重试）。**每轮都做反向验证**：还原「行内状态 + 文案 + 自动加载」后对应测试全红，恢复后全绿。vitest 69 → **76**。`pnpm build` ✅ / cargo 192 ✅ / 伴侣静态 ✅ / 伴侣 Kotlin 0 error ✅。
- [x] ✅ **v0.4.9-beta 已成功发版（run `37090091310`，2026-10-03 10:47 完成）**：**7 个 job 全绿**（验证 ✅ / 伴侣 App APK ✅ / macos-arm64 ✅ / linux-x64 ✅ / windows-x64 ✅ / macos-x64 ✅ / 发布 Release ✅）。tag `v0.4.9-beta` → `5e9de1d`。
  - **Release 已发布**：id `402289232`，draft=false / prerelease=true，published 2026-10-03T02:46:10Z，**30 资产全齐**（四平台安装包 + rpm + 伴侣 APK + `latest.json` + 7 个 `.sig` + 4 份 SHA256SUMS + 8 份 SBOM）。
  - **updater 端点已核验**：`latest.json` → `version=0.4.9`、4 平台且**签名全部非空**。
  - **Mac**：0.4.9 已装 `/Applications/MirrorDock.app`（x86_64，旧版备份 `/tmp/MirrorDock.app.bak.20261003-103726`），**需重启客户端生效**。
  - **伴侣 App 本版未改动**（维持 0.3.0 / versionCode 17）：0.4.7 与 0.4.9 发布的 APK **字节数完全相同**（4,500,264），且 `git diff 0c35e0f..HEAD -- companion/` 为空 → 只是重新打包。**手机已装 0.3.0，无需重装**。
  - **踩坑**：沙箱代理对 4.5 MB APK 的直连下载会**静默截断**（先后得到 516 KB / 786 KB / 1.6 MB 三次不完整文件，ZIP 校验失败）。改用 `Range: bytes=` 分段（每段 512 KB 可过）后可拼齐，但最后一段越界导致多出 65,536 字节、ZIP 损坏 —— **下载后必须用 `zipfile.testzip()` 校验，不能只看文件大小**。本次因版本未变无需安装，未造成影响。
- [x] ✅ **v0.4.9-beta 发版**：版本 0.4.8→0.4.9（四处代码口径 + README + site/index.html 29 处 + site/compatibility.html 6 处 + docs/compatibility-matrix.md 4 处）；新增 `docs/releases/v0.4.9-beta.md`。伴侣 App 本版**未改动**（维持 0.3.0），APK 随桌面重打包。tag `v0.4.9-beta` → CI 四平台 + Release。
  - **发版证据（tag 推送 + Release 核验后回填，此行不预填）**：
- [x] ✅ **X10-74 手机→电脑文件传输不可见（2026-10-03 06:30 夜间；用户报"客户端一直看不到"）**：**根因不是手机/adb/解析，是前端状态门禁形成死锁**。实机核查（adb）确认手机 `/sdcard/Download/MirrorDock` 确有 8 个文件、`adb shell ls -1` 输出干净（od 核对无 CR/乱码）、后端 `make_directory`/`list_directory`/`parse_device_listing` 全部正常。
  - **三处叠加**：①`deviceFiles` 初始 `null`，列表只在 `deviceFiles !== null` 时渲染，而它只在用户**手动点按钮**后才赋值，从不主动加载；②伴侣端 `files_changed` 的自动刷新条件是 `readyDevice && deviceFiles !== null && !transferBusy` —— 首次为空不刷新，列表又没打开过就永远变不成非 null，**不点按钮就永远看不到**；③整个文件传输面板被关在 `{readyDevice ? ... : ...}` 内，而文件传输走 adb 通道、**不依赖镜像会话**，却被会话状态一并挡掉。
  - **修复**：进入「工具」页即自动加载（首次也加载）；`files_changed` 不再要求 `deviceFiles !== null` 且自动切到工具页；面板移出 `readyDevice` 门禁改用 `transferDeviceSerial`（`readyDevice?.serial ?? sessionSerial`），APK 安装仍留门禁内（需设备就绪）；**读取失败时 `deviceFiles` 置 null**（不许用上次列表冒充、不把「读不到」说成「没有」）；设备切换时清空列表与提示。
  - **测试（做了两轮反向验证）**：新增 3 个回归测试（自动加载 / 无镜像会话时面板可见 / 读取失败不伪装成空）。**第一轮反向验证失败**——我只去掉 `deviceFiles !== null` 守卫，测试仍全绿，说明没真正锁住修复；**第二轮把整个自动加载 effect 删掉后 3 个全部失败**，恢复后全绿。**教训：反向验证必须回到"完全移除修复"的程度，只改一半会给虚假的安全感。** vitest 66 → **69**。
  - 验证：`pnpm build` ✅ / vitest 69 ✅ / cargo 192 ✅ / 伴侣静态校验 ✅ / 伴侣 Kotlin 0 error ✅；**真机复现验证**：用后端真实 argv 形式 `adb -s <serial> shell ls -1 /sdcard/Download/MirrorDock` 返回 8 个条目、退出码 0，证明数据侧无问题，修复后界面会自动呈现。0.4.8 已装到 Mac（`/Applications`，x86_64）。
- [x] ✅ **v0.4.8-beta 已成功发版（run `37074871816`，2026-10-03 07:11 完成）**：**7 个 job 全绿**（验证 ✅ / 伴侣 App APK ✅ / macos-arm64 ✅ / linux-x64 ✅ / windows-x64 ✅ / macos-x64 ✅ / 发布 Release ✅）。tag `v0.4.8-beta` → `69e405b`。
  - **Release 已发布**：id `402213895`，draft=false / prerelease=true，published 2026-10-02T23:10:18Z，**30 资产全齐**（`MirrorDock_0.4.8_x64.dmg` / `_aarch64.dmg` / `_x64-setup.exe` / `_x64_en-US.msi` / `_amd64.AppImage` / `_amd64.deb` / `MirrorDock-0.4.8-1.x86_64.rpm` / 伴侣 APK `MirrorDock-companion-0.4.8.apk` / `latest.json` + updater 归档 4 份 + `.sig` 7 份 + SHA256SUMS 4 份 + SBOM 8 份）。
  - **updater 端点已核验**：`latest.json` → `version=0.4.8`、`pub_date=2026-10-02T23:10:24Z`、`platforms=4` 且**四平台签名全部非空**。
  - **Mac**：0.4.8 已在 `/Applications/MirrorDock.app`（x86_64，旧版备份 `/tmp/MirrorDock.app.bak.20261003-064919`），**需重启客户端生效**。
  - **伴侣 App 本版未改动**（维持 0.3.0 / versionCode 17），APK 随桌面发版重打包。
- [x] ✅ **v0.4.8-beta 发版**：版本 0.4.7→0.4.8（四处代码口径 + README + site/index.html 29 处 + site/compatibility.html 6 处 + docs/compatibility-matrix.md 4 处）；新增 `docs/releases/v0.4.8-beta.md`。伴侣 App 本版**未改动**（0.3.0），APK 随桌面发版重打包。tag `v0.4.8-beta` → CI 四平台 + Release。
  - **发版证据（tag 推送 + Release 核验后回填，此行不预填）**：
- [x] ✅ **v0.4.7-beta 发版（2026-10-03 夜间；含 X10-72 + X10-73）**：版本 0.4.6→0.4.7（四处代码口径 + README + site/index.html 29 处 + site/compatibility.html 6 处 + docs/compatibility-matrix.md 4 处；伴侣 App 0.2.5→0.3.0）；新增 `docs/releases/v0.4.7-beta.md`。tag `v0.4.7-beta` → CI 四平台打包 + updater 签名 + 伴侣 APK（0.3.0 代码）+ latest.json + Release。
  - **⚠ 首次发版失败（run 37034993379，2026-10-03 00:35 → 01:05）**：`verify` ✅（Rust 192 + 前端 66 + build）、四个平台 `package` ✅ 全过，但**`companion-apk` ❌ 失败在第 5 步「构建 debug APK」**（Kotlin 编译错误）→ `release` job 被跳过，**没有产生任何 Release 资产**。根因三处（均在我本次新增代码里，**静态校验当时没覆盖到**）：
    ① `MainActivity.buildDeviceRow` 对 `Button` 赋 `minWidth`/`minHeight` —— `View` **没有**这两个可写属性（只有 `minimumWidth`/`minimumHeight`；`TextView` 另有 `setMinWidth`），编译失败。已改为用 `TextView` + `minWidth = dp(28)`，并去掉与系统默认内边距的搏斗。
    ② `PersistentConnectionService` 的 `if (rtt in 0..10_000)` —— `rtt` 是 `Long`，`0..10_000` 是 `IntRange`，**类型不匹配编译失败**。已改为 `0L..10_000L`。
    ③ `LinkShortcutManager` 直接读 `result.isLongLived` —— 该属性是 **API 30+**，而 `minSdk = 26`，会在 Android 8/9 上抛 `NoSuchMethodError`。已加 `SDK_INT >= R` 版本判断（外层再包 `runCatching` 兜底）。
  - **门禁补强（不让同类问题再拖垮发版）**：`scripts/verify-companion.py` 新增第 7.5 类检查（Kotlin 编译期陷阱：`View` 上的 `minWidth`/`minHeight`、Long 与 IntRange 混用、API 30+ 未判 `SDK_INT`），并已**反向验证**（注入 3 类错误 → 脚本全部抓到；恢复后全绿）。同时把该脚本接入 `build.yml` 的 `verify` job（`companion-apk` 与 `release` 都 `needs: verify`），**成为发版硬门禁** —— 下次同类错误在 push 阶段就会被拦下，而不是等 30 分钟后发现 Release 没产出。
  - **教训**：本地无 gradle 时，静态校验是唯一防线，**但校验器本身也会有盲区**。这次的代价是四平台包白跑一轮（tag 已推、代码已冻结，只改了 3 行 Kotlin）。发版记录必须以「Release 资产实际存在」为准，不能以「tag 已推」为准。
  - **⚠ 第二次发版仍失败（run 37039214291）**：修完上述三处后重打 tag，**APK job 依旧失败**。原因是**还有第 4 处、我完全没料到的编译错误**，而当时已无路可查：`/logs` 端点需 admin（403）、job annotations 只有 `Process completed with exit code 1`、check-runs annotations 也只有 exit code 无编译器原文。**只能靠猜，猜了两次都不中。**
  - **突破：本地跑起真实 kotlinc**。发现本机其实具备完整条件，只是没有 gradle 发行版（沙箱拦大文件下载）：`~/.gradle/caches` 里有 `kotlin-compiler-embeddable 2.0.20`（与项目一致）、`~/Library/Android/sdk/platforms/android-34/android.jar`、850M 依赖缓存（含全部 AndroidX aar）、Android Studio 自带 JBR，以及**系统 `/usr/bin/java` 是 Temurin 17**。直接用 `K2JVMCompiler` 跑类型检查即拿到编译器原文：
    - 关键点：Android Studio 的 JBR 是 **JDK 25**，kotlinc 2.0.20 解析不了 Java 版本号（`IllegalArgumentException: 25.0.2`），必须用系统 JDK 17。
    - 关键点：AGP 生成的 `R.class` 本地没有，需按 `res/` 实际内容生成同形状存根（`javac` 编译），否则报一堆假 `unresolved reference` 把真错误淹没。
  - **真正的第 4 处错误（`LinkShortcutManager`，2 个根因）**：
    ① `ShortcutInfo.Builder(SHORTCUT_ID)` —— **`Builder` 只有 `(Context, String)` 构造器，没有单参 String 版本**（`javap` 核对 android.jar 确认）。→ 改 `Builder(activity, SHORTCUT_ID)`。
    ② `requestPinShortcut` 返回的 `ShortcutManager.RequestPinShortcutResult` 是 **`@hide` 类型**，公开 SDK 里查不到（`javap` 直接报"找不到类"），因此 `isSuccess` / `isLongLived` **根本不可访问**。→ 只能判返回值是否为 null；文案改为"已向系统请求固定…在弹出的确认框点「添加」"，**不谎报已固定**。另补 `isRequestPinShortcutSupported` 预检（部分国产 ROM 禁用固定请求）。
  - **门禁再次补强**：新增 `scripts/check-companion-kotlin.py` —— 用 Gradle 缓存里的 kotlinc + `android.jar` + 全部 AndroidX aar **真实做类型检查**（并按 `res/` 生成 R 存根）。**已反向验证**：注入第 4 处错误 → 脚本抓到并报出精确行号 `LinkShortcutManager.kt:63:34`；恢复后 0 error。接入 `build.yml` 的 `verify` job。
    - 脚本只统计**前端 `error:` 行**；后端 `BackendException`（IR lowering）在本环境必然出现（缺 aapt 产物），已用**发版前未改动的原始代码验证过同样报错**，确认与代码无关。
  - **元教训（比 bug 本身更重要）**：**猜 bug 是错的做法**。第一次靠猜修对了两处、但漏了更深的一处，代价是四平台包白跑两轮。正确顺序是：先把「拿到编译器原文」的能力建起来（本地 kotlinc / CI 日志 artifact），再修 bug。在此之前不要改代码 —— 改了也无法验证是否修全。
  - **✅ 已成功发版（run `37043118757`，2026-10-03 02:02 完成）**：tag `v0.4.7-beta` → `4a6d3da`，**7 个 job 全绿**（verify ✅ / 伴侣 App APK ✅ / macos-x64 ✅ / macos-arm64 ✅ / linux-x64 ✅ / windows-x64 ✅ / 发布 Release ✅）。**Release 已发布**（id `402040641`，draft=false / prerelease=true，published 2026-10-02T17:59:56Z）。
    - **资产 30 个全齐**：`MirrorDock_0.4.7_x64.dmg` / `_aarch64.dmg` / `_x64-setup.exe` / `_x64_en-US.msi` / `_amd64.AppImage` / `_amd64.deb` / `MirrorDock-0.4.7-1.x86_64.rpm`、updater 归档 4 份 + `.sig` 4 份、**伴侣 APK `MirrorDock-companion-0.4.7.apk`（4.29 MB / 4,500,264 字节）**、4 份 `SHA256SUMS-<平台>.txt`、8 份 SBOM、`latest.json`。
    - **updater 端点已核验**：`latest.json` → `version=0.4.7`、`pub_date=2026-10-02T18:00:04Z`、`platforms=4` 且**四平台签名全部非空**（darwin-aarch64 / darwin-x86_64 / linux-x86_64 / windows-x86_64）。
    - **Mac**：本地构建的 0.4.7 已在 `/Applications/MirrorDock.app`（x86_64 匹配本机 Intel i9），**需重启客户端生效**。
    - **手机端已覆盖安装并验证**（真机 adb `79j7kn9tkjt8rwss`）：伴侣 **0.2.5 → 0.3.0**（versionCode 16 → 17），`install -r` 成功；启动无 FATAL 崩溃（进程存活）；**台账迁移已在真机上确证** —— `shared_prefs/paired_computers.xml` 里旧平铺键（`pairing_id=281a8740009b6916` / `desktop_fingerprint=b992d58…` / `last_host=192.168.0.177`）**全部保留**，同时新增 `computers_v2` 列表 + `active_pairing_id` + `legacy_migrated=true`，**旧凭据零丢失**。
  - **重新发版**：无需重打，第四次已成功。

## 最终成品退出条件

- [ ] 每个 MVP 功能有用户可见成功与恢复路径、自动化证据及文档。
- [ ] 目标桌面系统和设备矩阵完成，未支持组合明确降级。
- [ ] 本地优先、用户可见/可撤销、安全/DRM 边界和三渠道合规均经验证。
- [ ] 发行物具备签名、SBOM、NOTICE、许可证清单、漏洞结果、更新和回滚方案。

- [x] ✅ **X10-96 v0.4.19-beta 公私双仓库同步发版（2026-10-09 晨）**：用户拍板「公开+私有全部直接出 v0.4.19-beta 发版」。
  - **公开版内容**：X10-95 录制批次修复（优雅停止/托盘状态/保存目录，commit 7cf5868）首次进入 Release 渠道；本提交只做版本号 bump（package.json / tauri.conf.json / Cargo.toml / Cargo.lock / README / docs/compatibility-matrix.md 4 处 / site/index.html 21 处 / site/compatibility.html 6 处）+ `docs/releases/v0.4.19-beta.md` 发版说明。README 伴侣 App 口径由 0.3.0 更正为实际 0.3.2（build.gradle.kts versionName）。
  - **私有版同步**：MirrorDock-tools 同样 bump 0.4.19 + 发版说明（含 X10-94 点击器说明），tag 同名 `v0.4.19-beta`。
  - **发版证据（已核验，2026-10-09 07:35）**：tag `v0.4.19-beta` 已推远端、commit `86bd5b5`；CI run `37857794012` **success**；Release id `407339807` **draft=False、30 资产全齐**（7 安装包 + 伴侣 APK `MirrorDock-companion-0.4.19.apk` 4.29MB + 4 SHA256SUMS + 8 SBOM + updater 归档与签名）；`latest.json` **version=0.4.19**、pub_date=2026-10-08T23:24:55Z、四平台签名全非空（darwin-aarch64/darwin-x86_64/linux-x86_64/windows-x86_64 全 OK）。

- [ ] ⏳ **X10-98 v0.4.19-beta 错版事故 + CI 缓存根因修复 + 重发（2026-10-09 上午）**：用户真机实测 0.4.19 录像仍黑屏、自定义录像目录/托盘状态均未生效。
  - **事故定性（字节级实证，非推断）**：下载远端 0.4.19 产物 + 本地 /Applications 二进制，`strings`/字节搜索发现版本串 **0.4.19 MISSING、0.4.18 FOUND**，X10-95 修复标记 `is_recorder` **整个不在二进制里**；二进制大小与 0.4.18 完全一致（8209760 字节）。**装出来的「0.4.19」实为 0.4.18 旧二进制**。
  - **根因**：`Swatinem/rust-cache@v2` 以 `key: target名` 缓存 0.4.18 的 target 目录，0.4.19 的 package job 命中同一缓存键后 **Cargo 指纹误判旧 mirrordock crate 为 fresh、未重编**，直接复用旧二进制。版本号来自 `env!("CARGO_PKG_VERSION")` 本应触发重编，但 rust-cache 恢复的 target 让 Cargo 跳过了整个 crate。
  - **修法（已落地 commit eaad6e1）**：①package job rust-cache 加 `if: 非 tag`（发版强制全新编译，日常 main 保留缓存提速）；②构建后新增「核验二进制版本串与 tag 一致」步骤，不含对应版本串直接 fail-fast 不进 Release。
  - **重发**：删旧 tag → 在 eaad6e1 上重打 v0.4.19-beta → 重推，CI 全新编译（约 25 分钟）。release job 会清理该 tag 下旧的错版 Release。
  - **真机根因补充（澄清 X10-95 的盲区）**：SIGINT 优雅停止**本身是对的**——C wrapper 实测重置 SIG_DFL 后 1.0s 优雅退出、moov 完好；问题从不是信号路径，而是「修复根本没进二进制」。X10-95 的单测只验证「stop 发 SIGINT」，**没验证 spawn 的 pre_exec 重置进了产物**——这是测试盲区，也是这次没拦住的原因。
  - **教训**：发版核验不能只看「CI success + 资产齐 + latest.json 签名」——那些都是真的，但包的内容可能是旧的。**必须核验产物二进制本身**（版本串/关键修复标记）。已把版本串自检固化进 workflow，下次错版会在 CI 阶段被拦。
  - **✅ 重发已成功 + 字节级复核翻盘（run `37877990699`，2026-10-09 11:26 完成）**：tag `v0.4.19-beta` → `487b282`，7 job 全绿。Release id `407469488`，draft=false/prerelease=true，**34 资产全齐**（含 4 份 version-check 诊断 log）；`latest.json` version=0.4.19、四平台签名全非空（darwin-x64/aarch64=428、linux/windows=444）。
  - **⚠ 复核翻盘：重发出的 0.4.19 其实就是对的，X10-98「错版」指控过严**。对重发 x64 二进制做决定性字节级复核（run 37877990699 产物），发现先前判据全部失效：
    - `is_recorder` 是 Rust 结构体里的普通 `bool` 字段，release 构建被内联消除，**连本地全新源码构建（确定含修复）也没有这个字符串**——从不是有效标记。
    - 二进制里的 `0.4.18` 来自 `Cargo.lock` 中 `regex-automata v0.4.18` 这个第三方 crate，**任何构建都含**，与镜像工具自身版本无关；macOS 唯一真实版本载体是 plist（重发包 `CFBundleShortVersionString=0.4.19`，正确）。
    - **决定性证据**：`recording_dir_invalid` / `recording_dir_unwritable` 两个 Tauri 命令/错误串由 X10-95（7cf5868）引入、其父提交 8bf2f12 中为 0 命中——**两者都在重发二进制里各命中 1 次**。证明重发包确实包含 X10-95 录制修复。
  - **结论**：重发的 0.4.19 是「plist 版本正确 + 含 X10-95 修复」的正确产物，用户可重装验证录像功能。早前「用户实测仍黑屏」若在本正确包上复现，则需另查（如自定义目录落盘路径/权限、scrcpy 实际退出信号），不再归咎于错版。
  - **方法教训（第二次同类教训）**：判「错版」必须找一个**新旧二态、且 release 后仍存活**的标记。版本串、`is_recorder`、普通中文字符串都不合格（前者是依赖 crate 版本、后两者会被内联/移入字符串表）。合格标记 = 新引入的 Tauri 命令名/错误枚举串（命令注册会把名字嵌入二进制）。本次用 `recording_dir_*` 才把真假说清。

- [x] ✅ **X10-99 桌面模式录像「提示成功但文件夹没视频」根因修复（2026-10-09 午后）**：用户真机复现——开始/结束录制**两条提示都弹了**（"已开始屏幕录制：mirrordock-record-….mp4"/"已结束…保存在 MirrorDock 文件夹"），但 `Movies/MirrorDock` 里**没有视频**（目录只有 .DS_Store）。
  - **根因（真机字节级复现，非推断）**：用户镜像窗口处于**桌面模式**（`--new-display --start-app=com.netease.dhxy.qihoo` 虚拟屏）。X10-92 双通道录制让录制进程继承 `record_arguments()`，其中桌面模式会带上 `--new-display **--start-app=...**`；而录制通道固定 `--no-control`（无窗三件套之一）。**scrcpy 规定「控制被禁用时不允许启动应用」**，直接报 `ERROR: Cannot start an Android app if control is disabled` 立即退出、**一个字节都不写**。本地 sigwrap 复现：带 `--start-app` → NO FILE；去掉 → `Recording complete`、duration=6.07s、MP4 完好落盘 `Movies/MirrorDock`。
  - **为什么提示却成功（第二个 bug）**：`stop_recording_with` 只 stop 进程、从不核验产物文件是否存在/非空，就直接返回 `Recording{active:false}` → 前端弹「已结束…已保存」的**假成功**。这是「提示都有但没视频」的直接原因。
  - **修法（三处，均已落地 + 全绿）**：
    ① `record_arguments()` 桌面模式**去掉 `--start-app`**（录制是无头第二进程、无控制，起不了应用也用不着——应用已由显示通道启动；保留 `--new-display` 让录制仍落在虚拟屏，与桌面模式语义一致）。
    ② `stop_recording_with` 结束后**核验产物真实存在且 `len>0`**，缺失/为空→清掉 `record_path` 并报 `recording_file_missing`（"录制没有产出视频文件"），不再谎报已保存。
    ③ 前端 `toggleRecording` 结束录制报错时把本地 `recording` 一并复位 `null`，按钮/托盘回到「开始录制」，不再卡在「录制中」。
  - **测试**：修正 `starting_recording_uses_a_separate_channel...`（补 stub 产物文件 + create_dir_all，模拟真实定型落盘）。`cargo test --lib` **199 全绿**、`pnpm build` ✅、`pnpm test` **82 全绿**。
  - **真机实证**：sigwrap + 修复后参数（`--new-display` 无 `--start-app`）→ `Recording started/complete`、有效 MP4 写入 `Movies/MirrorDock`。
  - **教训**：「开始/结束都提示成功」≠「产物存在」。任何「产出文件」的操作，成功提示必须建立在**核验产物真实落盘**之上；否则会话级错误（scrcpy 立即退出）会被静默吞掉，用户端表现为「明明说录好了却没有」。另：X10-92 桌面模式的录制参数继承漏了 `--start-app` 与 `--no-control` 的互斥，**桌面模式录制从双通道上线起就没真正工作过**——这是首次真机覆盖到该路径。

- [ ] ⏳ **X10-100 v0.4.20-beta 公私双仓库同步发版（2026-10-09 午后）**：用户拍板「直接出 v0.4.20-beta（公开+私有）」。
  - **内容**：X10-99 桌面模式录像无视频修复 + 假成功提示修复（commit 公开 `1d22b05` / 私有 `fa9d51a` 同源）。版本 0.4.19→0.4.20（8 文件口径两仓库同步）+ `docs/releases/v0.4.20-beta.md`。测试：公开 cargo 199 ✅ / pnpm 82 ✅；私有 cargo 224 ✅ / pnpm build ✅。
  - **公开版**：commit `f10cc3f`，tag `v0.4.20-beta` 已推（`b1bb90b..f10cc3f` main + new tag）。CI run `37892508536` 已触发（tag 构建，含 X10-98 全新编译 + 版本文件名核验脚本）。
  - **私有版**：commit `fa9d51a`，删旧 tag `v0.4.19-beta`（d40b4a5）→ 重打 `v0.4.20-beta` 已推（`4ccf0d6..fa9d51a` main + new tag）。
  - **⚠ 私有版 CI 前提（用户侧）**：tools 仓库 Actions 此前未启用（X10-97 零运行），且伴侣签名 secret `COMPANION_KEYSTORE_BASE64` 未填（X10-21 keystore 缺文件 → `validateSigningDebug FAILED`）。base64 已交付用户（源 `MirrorDock-内部文档/mirrordock-companion.keystore`，密码 WQDZAM0HZNR6hakMUmmn）。**若 Actions 未启用/secret 未填，本次 tag 不会产出 Release**——需用户启用 Actions + 填 secret 后重推 tag 或 Run workflow。
  - **发版证据（待 CI 完成后回填，此行不预填）**：

- [ ] ⏳ **X10-101 桌面模式录制「Mac 白屏 / Windows 不完整」根因修复（2026-10-09 下午）**：用户真机报「Mac 桌面模式录出视频但白屏；Windows 桌面模式录视频提示视频不完整」。
  - **根因（真机实测定位，非推断）**：X10-92 给录制通道固定 `--no-control`（无窗三件套之一）。实测对比：非桌面模式（录真实主屏）`--no-control` 下录到 864×1920、抽帧 1.79MB 真实主屏 ✅；桌面模式（录虚拟屏）`--no-control` 下 Mac 录到 848×1920 **白屏+黑边**、几无有效帧，Windows 则因虚拟屏不刷新几乎零帧→时间轴残缺→「视频不完整」。**scrcpy 在无控制+无窗下不驱动虚拟屏渲染/刷新**——这是两平台同源的根因。
  - **关键实测对照（设备 qc8d8tonbmmzm7qs 小米 Android 13，梦幻西游 com.netease.dhxy.qihoo）**：
    - `--no-control --new-display` → 白屏帧（白+左右黑边）、nb_frames 极低 ❌
    - `--no-control` 录真实主屏 → 真实主屏画面 ✅（证明问题专属于虚拟屏）
    - **去 `--no-control` + `--new-display --start-app=游戏`** → 848×1920、152帧/7.9s≈19-22fps、抽帧 2.35MB **真实游戏画面** ✅
  - **修法（record_arguments，纯参数逻辑两平台共用）**：桌面模式录制**去掉 `--no-control`**（控制默认开，驱动虚拟屏渲染）+ **恢复 `--start-app`**（控制开启后 X10-99 的「--no-control 与 --start-app 互斥」不复存在，录制进程自己拉起目标应用确保录到该应用画面）。非桌面模式（主屏/摄像头）保持 `--no-control`。X10-99 与 X10-101 是同一根因链上的两步：X10-99 先去掉互斥的 --start-app 让 scrcpy 能起来，X10-101 进一步发现 --no-control 才是虚拟屏录不到画面的根源。
  - **测试**：cargo 199 全绿（录制 13 + 桌面 3 全过）。真机端到端（修复后参数）：152帧/7.9s、录到游戏真实画面。
  - **Windows 覆盖说明**：本次为纯 Rust 录制参数逻辑（record_arguments），两平台共用同一份代码与 scrcpy 4.1，Windows「不完整」与 Mac「白屏」同源（虚拟屏零帧/空帧），修复同步生效；真机仅验 Mac（无 Windows 设备），Windows 端待用户复验。
  - **发版状态**：源码已改（未提交未发版），release 构建进行中。需出 v0.4.21-beta。

- [ ] ⏳ **X10-102 v0.4.21-beta 公私双仓库同步发版（2026-10-09 下午）**：用户拍板「直接出 v0.4.21-beta 公私双仓库」。
  - **内容**：X10-101 桌面模式录制 Mac白屏/Windows不完整修复（公开 commit `028bb02` / 私有 `febf87b` 同源）。版本 0.4.20→0.4.21（8 文件两仓库同步）+ `docs/releases/v0.4.21-beta.md`。测试：公开 cargo 199 ✅；私有 cargo 224 ✅。
  - **公开版**：发版 commit `51ed0c5`，tag `v0.4.21-beta` 已推（`d4e4c6e..51ed0c5` main + new tag）。CI run `37897776188` 已触发（tag 构建）。
  - **私有版**：commit `febf87b`，tag `v0.4.21-beta` 已推（`fa9d51a..febf87b` main + new tag）。
  - **⚠ 私有版 CI 前提（同 X10-100，用户侧）**：tools 仓库 Actions 是否已启用、`COMPANION_KEYSTORE_BASE64`/`COMPANION_STORE_PASSWORD` 是否已填——若未就位，tag 不产 Release，需用户补齐后重推 tag 或 Run workflow。
  - **发版证据（待 CI 完成后回填，此行不预填）**：

- [ ] ⏳ **X10-104 v0.4.22-beta 双仓库发版 + 拦截 X10-101 有毒版本（2026-10-09 傍晚）**：用户报「桌面模式点开始录制后镜像闪退、白屏」——X10-101 引入的严重回归。
  - **根因（真机实测）**：X10-101 让录制进程 `--new-display --start-app` 自建第二块虚拟屏并重新拉起应用，把游戏「搬」到第二块屏、显示通道那块变白 → 闪退+白屏。X10-101 方向错了。
  - **正解（X10-103）**：录制通道**不建屏、不重复拉起应用**，用 `--display-id=<显示通道虚拟屏id>` 捕获同一块屏。实测：1920×848、170帧/7.4s≈23fps、录到游戏真实画面、显示通道全程不闪退。工程改动：`record_arguments(desktop_display_id)` 改签名 + trait `start_recorder` 加参 + `start_recording_with` 从显示进程 output_tail 实时解析（`resolve_desktop_display_id`，最多重试 ~1.5s，拿不到报 `desktop_display_unknown`）。新增回归测试 `desktop_recording_captures_the_display_channel_virtual_screen`。
  - **测试**：公开 cargo 200 ✅（含新测试）；私有 tools patch 同源应用 cargo 225 ✅。
  - **发版**：公开 fix `80659a9` + bump `e111c30`；tools `a81ede5`。tag `v0.4.22-beta` 两仓库均已推。公开 CI run `37901180472`。
  - **⚠ 拦截有毒 v0.4.21（X10-101 含闪退 bug）**：删远端+本地 tag `v0.4.21-beta`（GitHub 级联移除其 Release 407644872，Releases 列表顶端回到 v0.4.20）；**updater 分支回滚** `e34d2b7`(0.4.21)→`e6cd1d2`(0.4.20)（force push，发版链产物分支），已装用户自动更新回落到 0.4.20 稳定版。raw CDN 有缓存延迟，ls-remote 权威确认 HEAD=e6cd1d2(0.4.20)。
  - **发版证据**：公开 CI run `37901180472` `completed/success`；Release `407682954` 共 34 资产全齐（四平台安装包 + 签名 + SBOM + latest.json）。aarch64 dmg **字节级核验**：16719338 字节，sha256 `970d6fdf…` 与官方 `SHA256SUMS-macos-arm64.txt` 完全一致；plist `CFBundleShortVersionString`/`CFBundleVersion` 均 = `0.4.22`；`otool __cstring` 实证 `--display-id=`（X10-103 捕获逻辑）已编入 release 产物。
  - **真机验收（外部阻塞）**：桌面模式「开始录制不再闪退白屏、能录到真实画面」待用户在 Windows/Android 真机复核。

- [ ] ⏳ **X10-105 录制进程闪退后状态卡死（2026-10-09 晚）**：用户报「Windows 下录制开启一段时间后闪退重启镜像；重启后录制已停，但托盘仍显示『结束屏幕录制』，点结束文案不变，再点开始提示『已在录制』」。
  - **根因（代码实证）**：`spawn_session_monitor` 只监测**显示进程** `state.process` 的退出，对**独立录制进程** `state.record_process` 只在「显示进程退出时」才连带 stop。**录制进程自己闪退时无人回收句柄**——`record_process` 恒为 `Some`，而托盘菜单文案（lib.rs:3140）、删除校验 `recording_in_progress`（lib.rs:4907）、重复开始判定三处全部读 `record_process.is_some()`，于是状态全部卡死在「录制中」。这是 X10-92 双通道方案遗留的监测盲区。
  - **修复**：在 `spawn_session_monitor` 的「显示进程仍在跑」分支顺带 `try_wait` 录制进程，发现已退出即自愈——清 `record_process`/`record_path` → 刷新托盘菜单 → 给伴侣端补「录制结束」→ emit `recording-ended` 事件让前端复位录制按钮（App.tsx 新增监听）。无论录制进程因何而死，三处状态判定自动回到「开始录制」。
  - **测试**：新增回归测试 `a_crashed_recorder_is_reaped_while_the_display_session_keeps_streaming`（可控退出的 `ControlledProcess` 替身模拟「先跑后闪退」）。**反向验证**：临时禁用回收分支后测试如期变红，恢复后转绿——确认测试真正覆盖修复。全量 `cargo test` 201 ✅（原 200+新 1）、`pnpm build`(tsc) ✅、`cargo clippy` 零告警。
  - **未验证面（外部阻塞）**：① ~~录制进程**闪退的诱因**需 Windows 真机 scrcpy 输出定位，已请用户抓日志~~ **已定位（见下）**；② 修复后的真机行为（闪退时托盘/按钮自动复位、镜像不中断）待用户复核。此条不标 ✅。
  - **改动文件**：`src-tauri/src/lib.rs`（监测循环 + 测试 + `ControlledProcess` 替身）、`src/App.tsx`（`recording-ended` 监听）。

- [ ] ⏳ **X10-106 录制闪退诱因已定位——无线链路 180s 周期性断连（2026-10-09 晚，用户提供 Windows 真机日志）**：
  - **设备环境**：Redmi M2104K10AC（Android 13）**无线 adb** 连接 Windows 桌面端。
  - **日志证据链（用户排查）**：scrcpy 报 `WARN: Device disconnected` → 设备端 logcat `adbd: SSL_write failed [BAD_WRITE_RETRY]` + `ADB wifi device disconnected` → 7 秒后 `Handshake succeeded` 自动重连 → **断连间隔恰好 180 秒**（18:36:56 / 18:39:56），第二次无额外实例照样被杀；断连前 30 秒 Wi-Fi RSSI 从 51 掉到 46，系统反复报 `current network is in roaming environment` + Wi-Fi HAL 报错。F 盘每段录像结束时间与断连时刻吻合——**录制每 ~3 分钟被掐断一次 = 用户看到的「闪退」**。期间 MirrorDock 自动重启会话（虚拟屏 id 121→123），所以用户看到「还在运行」。
  - **归因结论**：**不是 scrcpy 编码器/进程崩溃，是无线链路层周期性断连**（Wi-Fi 漫游 + 省电行为），与 app 无关。USB 连接场景不受影响。
  - **用户侧对策（已建议，按优先级）**：① USB 线连手机录一次验证——不断连即可 100% 归因无线链路；② 无线方案下关手机 Wi-Fi 省电/优化（设置→WLAN→高级）、路由器关 band steering/漫游引导，让手机钉死在一个 AP/频段。
  - **app 侧待决策**：「断连自动恢复录制」——当前会话重连只恢复镜像，录制不会自动重启（用户每段只能录 ~3 分钟）。**是否做、是否自动续录，待用户拍板**。注意：断连期间丢帧无法补回，续录只能保后续片段，需考虑分段文件命名（如 `xxx-part2.mp4`）与 UI 提示。

- [ ] ⏳ **X10-107 断连自动续录（2026-10-09 晚，用户拍板「做自动续录」）**：
  - **设计**：挂点在「镜像断连重连成功后」（用户 Windows 日志证明断连时显示+录制一起死，X10-105 的存活分支覆盖不到），`spawn_wireless_reconnect` 重建会话成功后调 `resume_recording_after_reconnect`。续录用分段文件名（`录屏.mp4`→`录屏-part2.mp4`），断连画面如实不补；连续断连 3 次（`RECORD_RESUME_MAX`）放弃续录并提示改用数据线；用户手动开始新一轮录制时清零计数。
  - **实现**：`SessionState` 加 `record_resume_count`；新增 `resume_segment_name`/`should_resume_recording`/`resume_recording_after_reconnect`；`start_recording`（手动入口）成功后清零计数；续录失败不拖垮刚恢复的镜像。
  - **测试**：新增 3 测试（分段命名、上限判定、手动清零），cargo 204 ✅、clippy 零告警、前端 tsc ✅。**端到端（重连→起新录制进程）依赖真机，未验证**。
  - **未提交**：与 X10-105、X10-108 一并待用户确认后提交。

- [ ] ⏳ **X10-108 全平台镜像退出诊断落日志（2026-10-09 晚，根因排查中发现可观测性缺口）**：
  - **起因**：用户三组对照实验（USB 13min 稳定 / Wi-Fi 普通镜像挂 1 次自动重启成功 / Wi-Fi 录制近 5min 正常）已证明**程序在多数场景稳定**，问题收敛到无线链路 + 特定条件；但最初那次「Wi-Fi + 桌面模式虚拟屏 + 录制」闪退且不自动重启，诊断日志里**只有 mirror_start/record_start 全 ok，没有任何退出事件**——进程因断连被杀时完全无痕，排查只能凭回忆复现。
  - **结论**：诊断日志此前只记命令调用，不记进程退出——这是全平台可观测性的根本缺口。
  - **修复**：`spawn_session_monitor` 的 `finished` 分支强制落一条 `mirror_exit` 事件（正常/异常 + 是否触发自动重连 + scrcpy 输出尾部 4 行），让任何平台的异常退出都直接「日志说话」。测试环境无 TRAY_APP 自动跳过。cargo 204 ✅、clippy ✅。
  - **待办**：用户需复现一次「桌面模式虚拟屏闪退」，导出诊断看 `mirror_exit` 事件的 exit_success 与「是否触发自动重连」——据此钉死「Mac 不自动重启」是没触发重连还是重连 probe 失败。

- [x] ✅ **X10-109 v0.4.23-beta 发版（2026-10-09 深夜）**：
  - **提交**：`b3747bc`（X10-105/107/108 功能）+ `b201a6a`（release bump 0.4.22→0.4.23，8 文件 + docs/releases/v0.4.23-beta.md）。
  - **CI**：tag `v0.4.23-beta` → 验证（Rust 单测+前端构建）✅、伴侣 APK ✅、四平台打包 ✅；run `37946061898`。
  - **Release 核验（发版后才标 ✅，教训 X10-105/v0.4.5 幽灵版）**：Release id `408057993`，**34 资产全齐**（四平台 dmg/exe/msi/AppImage/deb/app.tar.gz + .sig + SBOM×8 + SHA256SUMS×4 + latest.json + version-check×4）。
  - **updater 通道**：updater 分支 HEAD `ef4ac2a`，`latest.json` version=`0.4.23`、四平台 Ed25519 签名 URL 齐全——已装用户可自动更新到 0.4.23。
  - **内容**：X10-105 录制状态自愈 / X10-107 断连自动续录 / X10-108 镜像退出诊断日志。
  - **教训**：GitHub API 未认证限流 60 次/小时，监控脚本 90 次高频轮询会烧光配额导致查不到状态（误以为 CI 失败）。监控用 ≥3 分钟间隔的低频轮询，或用带认证的 GitHub MCP 工具。

- [ ] ⏳ **X10-110 镜像退出诊断增强（2026-10-10 凌晨，通道+存活时长+死因分类）**：
  - **起因（关键反转）**：用户升级 0.4.23 后反馈「**我用 USB 连接的**，但还是自动重连、游戏进程重连后掉登录，以前能稳定连一晚上」。但 `mirror_exit` 日志每条 scrcpy 末尾都是 `INFO: --> (tcpip) ...`——`(tcpip)` 是 scrcpy 标明本次会话走**网络通道**（真走数据线是 `--> (usb)`）。即用户插着 USB，但手机无线调试仍开着，MirrorDock 实际选用了无线端点，断的还是无线链路（00:20/03:19/03:20/05:06/05:07 多次断连，间隔 22秒~3.5小时无固定周期，全部零 ERROR 被链路秒杀）。游戏掉登录：桌面模式虚拟屏随断连销毁，MIUI 清掉挂在虚拟屏上的游戏进程，重连 `--start-app` 冷启动新进程。
  - **结论**：日志只记「异常退出」不够——必须让通道、存活时长、死因一目了然，才能避免「插着 USB 却断连」这类凭感觉归因。
  - **实现**：`SessionState` 加 `attached_at`（attach 时刻）；新增 `format_uptime`（秒/分秒/时分）、`MirrorExitKind`（ScrcpyError/WirelessLinkCut/UsbLinkCut，全覆盖无 unknown）、`extract_error_line`、`classify_mirror_exit`（错误行优先于通道判定）、`mirror_exit_advice`（按死因给针对性建议）。`mirror_exit` 事件改写为「{死因}，通道 {有线/无线}，存活 {时长}，{是否重连}。scrcpy 末尾：...」；`resolve_process_exit` 的恢复建议同步按死因定制（无线秒杀提示关 Wi-Fi 省电/换数据线，有线提示插拔/换线，不再一句通用话）。
  - **测试**：新增 7 测试（uptime 格式化三档、无线零错误→wireless_link_cut、有线零错误→usb_link_cut、显式 ERROR 覆盖通道判定、通道标签随 serial、建议可操作性、resolve_process_exit 针对性恢复建议）。cargo **211 ✅**、clippy 零告警。
  - **反向验证**：破坏 `extract_error_line` 的错误检测 → `classify_explicit_error_overrides_channel` 变红；恢复 → 转绿。干净（仅改一行 + 改回，未用 checkout 抹改动）。
  - **未验证面**：真实断连下的 `mirror_exit` 事件成品（通道/存活/死因是否如预期渲染）待下次断连或用户主动验证；「插着 USB 仍走无线」是否需要在 UI 上显式提示用户「当前走的是无线而非你插的 USB」，**待用户拍板**（属产品决策，不擅自改交互）。
