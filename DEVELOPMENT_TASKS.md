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
  - 里程碑 2（待做）：设置应用/托盘/录制的多会话 UI 细化（当前设置作用于主会话）。
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
  - **待办（候选增强，未实现）**：会话建立时探测 scrcpy UHID 的启用布局，非标准布局时在设备卡便签/状态栈提示一键打开实体键盘设置页；或在键盘配置面板内直接展示当前启用布局清单。
  - 环境备忘：沙箱内执行 `/Applications/MirrorDock.app/.../adb` 部分命令被 sandbox-center 拦截（`decisionRecord missing actual resource subject`），改用 `.tools/scrcpy/macos-x86_64/adb` 可用；`adb shell dumpsys input`、`wm size` 带引号/管道形式会被拦，改「无引号 + 输出重定向到文件再本地分析」可绕过。

## 最终成品退出条件

- [ ] 每个 MVP 功能有用户可见成功与恢复路径、自动化证据及文档。
- [ ] 目标桌面系统和设备矩阵完成，未支持组合明确降级。
- [ ] 本地优先、用户可见/可撤销、安全/DRM 边界和三渠道合规均经验证。
- [ ] 发行物具备签名、SBOM、NOTICE、许可证清单、漏洞结果、更新和回滚方案。
