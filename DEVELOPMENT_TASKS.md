# MirrorDock 开发任务清单

> 规则：按依赖顺序完成；只有验证证据和退出标准满足后才将任务改为完成。真机、签名凭据、商店账号或第三方审核不可替代，缺失时必须标记为外部阻塞，不能伪造验收。

## 阶段 0：可行性 POC

- [✅] P0-01 初始化 Tauri 2 + React + TypeScript 桌面项目。
- [✅] P0-02 实现安全的 ADB 设备/授权状态读取与中文恢复引导。
- [✅] P0-03 安装并固定 scrcpy 开发运行时；记录版本、校验信息、许可证与开发依赖来源。
- [✅] P0-04 实现仅对已授权序列号启动 scrcpy 的直接进程接口。
- [ ] P0-05 在真实 Android 设备上验证 USB 首帧、鼠标/键盘、断开与授权撤销恢复。**待测：本次仅完成 Wi-Fi 真机验证。**
- [ ] P0-06 实现无线调试配对、同网连接、保存/忘记可信设备与网络切换恢复。**已在 Android 13 真机完成配对、独立端口连接和镜像；重连、忘记与网络切换待测。**
- [ ] P0-07 建立首帧、FPS、时延估算、掉线与 60 分钟稳定性测试记录。**已采集一组 Wi-Fi FPS/分辨率真实数据；首帧、时延、掉线与 60 分钟数据待测。**
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
- [ ] B2-05 非技术用户可用性测试、帮助中心、客服分流与兼容性页面。
- [ ] B2-06 Windows、macOS、Ubuntu 安装包；更新、回滚、签名和渠道差异验证。

## 阶段 3：公开 v1

- [ ] R3-01 免费/Pro 授权设计与本地权益状态；不将任何授权密钥写入日志。**已实现（2026-09-28 夜间轮次，自主决策已记录）；待真实激活流程的 GUI 端到端验收。**
  - **授权模型（自主决策）**：无账户、无激活服务器、离线激活；免费版保留全部核心体验（USB/无线镜像、截图、文件传输、**音频转发**、会话设置、诊断）；Pro 门控仅 MP4 录制。特别说明：音频转发不做门控——`audio` 默认开启，纳入门控会让免费版默认启动直接报错，违背开箱即用。
  - **许可证格式**：ed25519 签名 JSON 载荷（product/key_id/edition/expires_at），`MD1-` 前缀 + 自实现 base32（长度前缀消歧义、6 字符分组可抄写）；公钥编译进二进制（`LICENSE_VERIFYING_KEY`），私钥由 `examples/license_keygen` 生成、存仓库外内部文档目录，绝不入 Git/CI/日志。
  - **命令面 23 → 26**：`entitlement_status` / `entitlement_activate`（失败诊断脱敏：原始密钥串声明进机密列表，测试断言不回显）/ `entitlement_deactivate`；权益文件 entitlement.json 存许可证原文、加载时重验签，损坏/篡改/过期一律安全回退免费版。
  - **门控位置**：`start_mirroring` 与 `update_session_options` 前置 `ensure_edition_allows`（错误码 `pro_required`，触碰设备之前就拒绝）；录制开关在免费版禁用并给出激活指引。
  - **签发工具**：`examples/license_sign`（env 种子）与 `examples/license_keygen`（/dev/urandom + 签名自检）。
  - **证据**：Rust 89 → 99（base32 往返、验签往返/篡改拒绝/过期/产品不符、损坏回退、门控矩阵、密钥不回显、**真实种子端到端**（无种子环境自动跳过））；前端 23 → 27（免费/专业两态渲染、激活调用契约、失败不回显密钥）；clippy 零告警；`pnpm build` 通过。
  - **未验证面**：GUI 下真实激活/撤销的端到端体验、过期许可在到期后门禁的运行时行为（逻辑有测试、无真机轮次）。
- [ ] R3-02 国内、Google Play、企业侧载各自的隐私、权限、签名、更新与支持材料。**底稿完成（docs/channel/）；法务复核与渠道审核为外部流程。**
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

## 最终成品退出条件

- [ ] 每个 MVP 功能有用户可见成功与恢复路径、自动化证据及文档。
- [ ] 目标桌面系统和设备矩阵完成，未支持组合明确降级。
- [ ] 本地优先、用户可见/可撤销、安全/DRM 边界和三渠道合规均经验证。
- [ ] 发行物具备签名、SBOM、NOTICE、许可证清单、漏洞结果、更新和回滚方案。
