use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// 会话状态机轮询运行中进程的间隔。
const MONITOR_INTERVAL: Duration = Duration::from_millis(200);

/// 优雅结束镜像进程时等待 scrcpy 收尾（写出录像 moov 索引）的上限。
const PROCESS_GRACEFUL_TIMEOUT: Duration = Duration::from_millis(3000);

/// 兜底强杀之后等待进程真正退出的上限。SIGKILL 之后的等待只为回收，通常毫秒级。
const PROCESS_REAP_TIMEOUT: Duration = Duration::from_millis(1000);

/// `scrcpy --list-apps` 的最长等待时间。低端机解析全部应用可能需要数秒；
/// 超时按失败处理，回退到 `pm list packages`（X10-71）。
const LIST_APPS_TIMEOUT: Duration = Duration::from_secs(15);

/// 单次 adb 子进程调用的最长等待时间（X10-84）。
/// Windows 上 adb 对掉线/未授权设备可能长时间不返回，必须兜底超时，
/// 否则 Tauri 命令线程被占死 → 前端「未响应」。
const ADB_CALL_TIMEOUT: Duration = Duration::from_secs(8);

/// MVP 只承诺 Android 8.0（API 26）及以上的画面与控制。
const MIN_SDK_FOR_MIRRORING: u32 = 26;

/// 系统音频转发需要 Android 11（API 30）及以上。
const MIN_SDK_FOR_AUDIO: u32 = 30;

/// 设备属性值的最大保留长度。
///
/// 设备返回的属性属于不可信输入：超长内容一律截断，避免异常设备把超长文本
/// 带进界面或诊断信息。
const MAX_PROPERTY_LEN: usize = 64;

/// 解析设备属性时最多读取的行数，用于给敌意输入设定上界。
const MAX_PROPERTY_LINES: usize = 4096;

/// 序列号或无线端点允许的最大长度。
const MAX_SERIAL_LEN: usize = 128;

// ---------------------------------------------------------------------------
// 设备与结构化错误
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeviceState {
    Ready,
    Unauthorized,
    Offline,
    Unknown,
}

/// 一条连接通道：adb 序列号（USB 为硬件序列号，无线为 `IP:端口`）+ 连接方式 + 状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ConnectionKind {
    Usb,
    Wireless,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ConnectionEndpoint {
    serial: String,
    kind: ConnectionKind,
    state: DeviceState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AdbDevice {
    /// 首选连接通道的 adb 序列号（用于发起镜像/唤醒等动作）。
    serial: String,
    label: String,
    /// 所有通道里最好的状态：任一通道就绪即视为就绪。
    state: DeviceState,
    /// 硬件序列号（`ro.serialno`）。同一台手机无论 USB 还是无线都一致，用于跨连接去重。
    /// 仅当设备已授权（ready）时可读取；读不到时为 `None`。
    physical_serial: Option<String>,
    /// 该物理设备当前在 adb 中出现的全部通道（USB / 无线可能并存）。
    connections: Vec<ConnectionEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AdbCheck {
    adb_available: bool,
    scrcpy_available: bool,
    devices: Vec<AdbDevice>,
    diagnostic: Option<String>,
}

/// 面向非技术用户的结构化错误：稳定错误码 + 原因 + 用户可执行的修复动作。
///
/// 错误码是对前端的稳定契约，不要在修复文案变化时改动它。
/// 已使用的错误码：
///   device_not_selected / device_serial_invalid / device_unauthorized
///   device_offline / device_not_connected
///   adb_missing / adb_unavailable / probe_failed
///   mirror_runtime_missing / mirror_start_failed / mirror_exited / mirror_stop_failed
///   session_unavailable / session_busy / session_not_running / session_restart_failed
///   invalid_rotation / endpoint_invalid / pairing_code_invalid / pairing_failed
///   connect_failed / connect_not_ready
///   trusted_list_unavailable / trusted_list_unreadable / trusted_list_write_failed
///   lock_probe_failed / wake_failed
///   recent_list_unavailable / recent_list_unreadable / recent_list_write_failed
///   media_name_invalid（截图与录像共用的文件名校验）
///   screenshot_dir_unavailable / screenshot_not_image / screenshot_failed
///   screenshot_write_failed / screenshot_missing / screenshot_delete_failed
///   recording_dir_unavailable / recording_in_progress / recording_missing / recording_delete_failed
///   transfer_name_invalid / transfer_local_missing / transfer_dir_unavailable
///   transfer_push_failed / transfer_list_failed / transfer_pull_failed
///   apk_path_invalid / apk_install_failed
///   transfer_push_failed / transfer_pull_failed / transfer_list_failed
///   diagnostics_write_failed（诊断包导出失败）
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AppError {
    code: &'static str,
    message: String,
    recovery: String,
}

impl AppError {
    fn new(code: &'static str, message: &str, recovery: &str) -> Self {
        Self {
            code,
            message: message.to_owned(),
            recovery: recovery.to_owned(),
        }
    }
}

fn adb_missing_error() -> AppError {
    AppError::new(
        "adb_missing",
        "未找到 Android 平台工具。",
        "请重新安装 MirrorDock 或联系支持人员。",
    )
}

/// 把 ADB 子进程的失败翻译成面向用户的错误；可执行文件缺失单独成码。
fn adb_command_error(
    error: std::io::Error,
    code: &'static str,
    message: &str,
    recovery: &str,
) -> AppError {
    if error.kind() == std::io::ErrorKind::NotFound {
        adb_missing_error()
    } else {
        AppError::new(code, message, recovery)
    }
}

// ---------------------------------------------------------------------------
// 会话状态机
// ---------------------------------------------------------------------------

/// 会话阶段。按工程铁律，下列状态必须彼此可区分，禁止塌缩成单一的“连接失败”。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SessionPhase {
    /// 空闲，没有进行中的会话。
    Idle,
    /// 目标设备存在但尚未授权这台电脑。
    Unauthorized,
    /// 目标设备存在但处于离线状态。
    Offline,
    /// 无线设备已配对并已建立连接，可以开始镜像。
    Paired,
    /// 正在启动镜像窗口。
    Connecting,
    /// 镜像进程运行中。注意：这不代表首帧已到达，见 `FirstFrame`。
    Streaming,
    /// 失败，具体原因见 `MirrorSession::error`。
    Failed,
}

/// 首帧到达情况。
///
/// 进程 `Running` 不等于画面已经出现。当前版本尚未实现端到端首帧探针，因此只会
/// 产生 `Unknown`；`Reached` 预留给真实探针接入后使用，不得在无证据时上报。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FirstFrame {
    /// 尚未探测或无法确认。
    Unknown,
    /// 已由端到端探针确认首帧到达。
    Reached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct MirrorSession {
    phase: SessionPhase,
    serial: Option<String>,
    first_frame: FirstFrame,
    error: Option<AppError>,
}

impl MirrorSession {
    fn idle() -> Self {
        Self {
            phase: SessionPhase::Idle,
            serial: None,
            first_frame: FirstFrame::Unknown,
            error: None,
        }
    }

    fn unauthorized(serial: String, error: AppError) -> Self {
        Self {
            phase: SessionPhase::Unauthorized,
            serial: Some(serial),
            first_frame: FirstFrame::Unknown,
            error: Some(error),
        }
    }

    fn offline(serial: String, error: AppError) -> Self {
        Self {
            phase: SessionPhase::Offline,
            serial: Some(serial),
            first_frame: FirstFrame::Unknown,
            error: Some(error),
        }
    }

    fn paired(endpoint: String) -> Self {
        Self {
            phase: SessionPhase::Paired,
            serial: Some(endpoint),
            first_frame: FirstFrame::Unknown,
            error: None,
        }
    }

    fn connecting(serial: String) -> Self {
        Self {
            phase: SessionPhase::Connecting,
            serial: Some(serial),
            first_frame: FirstFrame::Unknown,
            error: None,
        }
    }

    fn streaming(serial: String) -> Self {
        Self {
            phase: SessionPhase::Streaming,
            serial: Some(serial),
            first_frame: FirstFrame::Unknown,
            error: None,
        }
    }

    fn failed(serial: Option<String>, error: AppError) -> Self {
        Self {
            phase: SessionPhase::Failed,
            serial,
            first_frame: FirstFrame::Unknown,
            error: Some(error),
        }
    }
}

impl Default for MirrorSession {
    fn default() -> Self {
        Self::idle()
    }
}

/// 进程正常退出与异常退出分别回到空闲与失败，并给出可执行的恢复动作。
///
/// `scrcpy_output` 是该进程生前的输出尾部（X10-79）：异常退出时把它附在恢复建议里，
/// 用户在界面上能直接看到 scrcpy 的原文（如「ERROR: Could not open icon image」、
/// 编码器拒绝参数、隧道被占用……），而不是一句干巴巴的「已意外关闭」。
fn resolve_process_exit(serial: &str, success: bool, scrcpy_output: &str) -> MirrorSession {
    if success {
        MirrorSession::idle()
    } else {
        let mut recovery = "请检查手机授权与连接后重新启动镜像。".to_owned();
        let tail = scrcpy_output.trim();
        if !tail.is_empty() {
            // 只保留尾部若干行：环形缓冲本身有上限，这里再截一次，避免界面被刷爆。
            let lines: Vec<&str> = tail.lines().collect();
            let excerpt: Vec<&str> = lines
                .iter()
                .rev()
                .take(8)
                .rev()
                .copied()
                .collect();
            recovery.push_str("\n\nscrcpy 输出（末尾）：\n");
            recovery.push_str(&excerpt.join("\n"));
        }
        MirrorSession::failed(
            Some(serial.to_owned()),
            AppError::new("mirror_exited", "镜像窗口已意外关闭。", &recovery),
        )
    }
}

// ---------------------------------------------------------------------------
// 运行时抽象：ADB 与镜像引擎都可替换，便于在无真机时做接口替换测试
// ---------------------------------------------------------------------------

trait AdbRuntime: Send + Sync {
    fn list_devices(&self) -> Result<Vec<AdbDevice>, std::io::Error>;
    /// 读取设备的系统属性原始输出，用于在启动会话前解释这台手机的能力边界。
    fn device_properties(&self, serial: &str) -> Result<String, std::io::Error>;
    /// 读取硬件序列号（`ro.serialno`）。
    ///
    /// 同一台手机无论经 USB 还是无线连接，硬件序列号都一致；USB 连接时它恰好等于
    /// adb 序列号，无线连接时 adb 序列号则是 `IP:端口`。这是跨连接方式识别「同一台设备」
    /// 的唯一可靠依据。仅已授权（ready）设备可读；读不到或失败时返回 `Ok(None)`，
    /// 调用方据此决定不去重（而非报错）。
    fn physical_serial(&self, serial: &str) -> Result<Option<String>, std::io::Error>;
    /// 点亮设备屏幕（`KEYCODE_WAKEUP`）。
    ///
    /// 只唤醒屏幕：不输入任何凭据、不解锁、不解除钥匙锁。锁屏本身不在可绕过范围内。
    fn wake_screen(&self, serial: &str) -> Result<(), std::io::Error>;
    /// 读取系统熄屏时间 `screen_off_timeout`（毫秒）。
    ///
    /// 输出无法解析为非负整数（如设备返回 `null`）时返回 `Ok(None)`——读不到就如实
    /// 上报，由调用方决定跳过补偿，而不是猜一个"默认值"当原值。
    fn screen_off_timeout(&self, serial: &str) -> Result<Option<u64>, std::io::Error>;
    /// 写入系统熄屏时间 `screen_off_timeout`（毫秒）。
    fn set_screen_off_timeout(&self, serial: &str, millis: u64) -> Result<(), std::io::Error>;
    /// 读取系统「充电时保持唤醒」设置 `stay_on_while_plugged_in`（位掩码）。
    ///
    /// 这是 scrcpy `--stay-awake` 真正依赖的开关：位掩码命中且设备处于「已插电」时，
    /// 系统不会因超时熄屏——**锁屏页也一样**。读不到返回 `Ok(None)`。
    fn stay_on_while_plugged_in(&self, serial: &str) -> Result<Option<u64>, std::io::Error>;
    /// 写入 `stay_on_while_plugged_in`（位掩码）。
    fn set_stay_on_while_plugged_in(&self, serial: &str, bits: u64) -> Result<(), std::io::Error>;
    /// 读取 `dumpsys window policy` 原始输出，用于判断钥匙锁状态。
    fn window_policy(&self, serial: &str) -> Result<String, std::io::Error>;
    /// 读取 `dumpsys power` 原始输出，用于判断屏幕是否点亮。
    fn power_state(&self, serial: &str) -> Result<String, std::io::Error>;
    /// 读取 `dumpsys display` 原始输出，用于识别「变暗（DIM）」电源策略。
    ///
    /// 背景：`stay_on_while_plugged_in` 只能拦住熄屏（OFF），拦不住熄屏前的
    /// 变暗阶段（DIM，背光压到 5%、渲染层对虚拟显示器输出黑帧）。要识别这个
    /// 状态必须看 `mPowerRequest=policy=`，它只在 `dumpsys display` 里。
    fn display_state(&self, serial: &str) -> Result<String, std::io::Error>;
    /// 发送一个按键码。`keycode` 只允许来自代码内常量（如 `KEYCODE_BACK`），
    /// 绝不接受用户输入——按键注入属于输入类操作，参数必须可审计。
    fn press_key(&self, serial: &str, keycode: &str) -> Result<(), std::io::Error>;
    /// 截屏探测：返回 `screencap` 输出的字节数。像素内容就地丢弃，不落盘、不回传。
    ///
    /// 密码输入页是安全表面：系统对截屏与镜像同时拒绝输出（实测返回 0 字节，
    /// 正常锁屏壁纸页约 3.7 MB）。这个字节数就是「密码页是否在屏」的可靠信号。
    fn screencap_probe_bytes(&self, serial: &str) -> Result<u64, std::io::Error>;
    /// 读取设备当前屏幕的 PNG 快照（原始字节）。
    ///
    /// 返回值是**屏幕内容**，属于最敏感的数据类别：只允许写入用户可见的本地文件，
    /// 任何情况下都不得写入日志、错误消息或遥测。
    fn screenshot_png(&self, serial: &str) -> Result<Vec<u8>, std::io::Error>;
    /// 在设备上创建目录（含父目录，幂等）。
    fn make_directory(&self, serial: &str, remote_dir: &str) -> Result<(), std::io::Error>;
    /// 列出设备目录内容（`ls -1` 原始输出，每行一个条目）。
    fn list_directory(&self, serial: &str, remote_dir: &str) -> Result<String, std::io::Error>;
    /// 把本机文件推送到设备目录。返回 adb 的输出摘要。
    fn push_file(
        &self,
        serial: &str,
        local: &Path,
        remote_dir: &str,
    ) -> Result<String, std::io::Error>;
    /// 把设备文件拉取到本机路径。返回 adb 的输出摘要。
    fn pull_file(
        &self,
        serial: &str,
        remote_path: &str,
        local: &Path,
    ) -> Result<String, std::io::Error>;
    /// 删除设备上的一个文件（用于清理发送区）。路径经调用方校验。
    fn remove_device_file(&self, serial: &str, remote_path: &str) -> Result<(), std::io::Error>;
    /// 在设备上安装一个 APK。返回 adb 的原始输出（stdout+stderr 合并）。
    ///
    /// 与其它方法不同，这里**业务失败也返回 Ok(原始输出)**：`adb install` 在失败时
    /// 会把 `Failure [INSTALL_FAILED_…]` 写在输出里并置非零退出码，只有把原始输出
    /// 交给上层解析，才能告诉用户「是版本降级、签名冲突还是空间不足」，而不是把
    /// 具体原因塌缩成一句笼统的「安装失败」。仅当进程无法启动（如 adb 缺失）才返回 Err。
    fn install_apk(&self, serial: &str, apk: &Path) -> Result<String, std::io::Error>;
    fn pair(&self, endpoint: &str, pairing_code: &str) -> Result<(), std::io::Error>;
    fn connect(&self, endpoint: &str) -> Result<(), std::io::Error>;
    fn disconnect(&self, endpoint: &str) -> Result<(), std::io::Error>;
    /// 在手机上打开开发者选项页（`am start`，固定 action，无用户输入）。
    ///
    /// 真正清除手机保存的授权必须由本人在手机上操作（Android 安全设计），
    /// 客户端只负责把页面带到用户面前。
    fn open_developer_settings(&self, serial: &str) -> Result<(), std::io::Error>;
    /// 在手机上打开「实体键盘」设置页（`am start`，固定 action，无用户输入）。
    ///
    /// UHID 物理键盘的字符映射依赖手机上为 scrcpy 键盘启用的键盘布局（X10-37
    /// 真机定案：布局被切走/未启用「英语（美国）」时打字无效或字符全错）。
    /// 布局启用集合没有可靠的只读探测面，客户端不猜、只把页面带到用户面前。
    fn open_keyboard_layout_settings(&self, serial: &str) -> Result<(), std::io::Error>;
    /// 关闭手机上的「无线调试」开关（`settings put global adb_wifi_enabled 0`）。
    /// 关闭后所有电脑的无线连接立即失效；等效于收回无线通道的访问权。
    fn disable_wireless_debugging(&self, serial: &str) -> Result<(), std::io::Error>;
    /// 关闭手机上的「USB 调试」开关（`settings put global adb_enabled 0`）。
    /// 关闭后所有电脑的调试访问（USB+无线）立即失效；重新打开需在手机上操作。
    fn disable_usb_debugging(&self, serial: &str) -> Result<(), std::io::Error>;
    /// 列出局域网内 adb 通过 mDNS 发现的服务（`adb mdns services` 原始输出）。
    ///
    /// 用于「开发者中心扫码配对」的自动填地址：手机无线调试处于配对页时，会
    /// 广播 `_adb-tls-pairing._tcp.` 服务；主页面广播 `_adb-tls-connect._tcp.`。
    /// 解析只挑这两类，其余服务一律忽略。
    fn mdns_services(&self) -> Result<String, std::io::Error>;
    /// 列出设备上已安装的第三方应用包名（`pm list packages -3`，X10-53）。
    ///
    /// 返回原始输出（每行 `package:<包名>`），由调用方解析；用于桌面模式
    /// 「虚拟屏启动的应用」候选列表。读操作，不改变设备状态。
    /// 读取 `dumpsys activity activities` 原始输出（X10-80，桌面模式落地核验）。
    ///
    /// 默认实现返回空串：测试替身不关心这个偏运维的探针，真实实现覆盖它。
    fn activity_dumpsys(&self, _serial: &str) -> Result<String, std::io::Error> {
        Ok(String::new())
    }
    fn list_device_apps(&self, serial: &str) -> Result<String, std::io::Error>;
}

/// 一个已启动的镜像进程。抽象出 `try_wait` 与 `kill`，使会话生命周期可在测试中验证。
trait MirrorProcess: Send {
    /// `Some(success)` 表示进程已退出，`None` 表示仍在运行。
    fn try_wait(&mut self) -> Option<bool>;
    fn kill(&mut self) -> Result<(), std::io::Error>;
    /// 结束会话的统一入口。默认强杀；系统实现覆写为「先优雅退出、超时再强杀」。
    ///
    /// 之所以不能直接强杀：录制中的 MP4 依赖 scrcpy 退出前写出 moov 索引，强杀
    /// 会留下一个体积正常却无法播放的文件（真机实验已证实）。测试替身沿用默认
    /// 实现以保持用例快速、确定。
    fn stop(&mut self) -> Result<(), std::io::Error> {
        self.kill()
    }
    /// 取回该进程启动以来的 scrcpy 输出尾部（X10-79）。
    ///
    /// 真实实现会把子进程的 stdout/stderr 收进环形缓冲；测试替身没有子进程，
    /// 默认返回空。**这是「用户报故障但看不到日志」的根因修复**：此前
    /// `ScrcpyRuntime::start` 只 `spawn()` 不接管输出，scrcpy 的报错
    /// （编码器拒绝参数、SDL 起不来窗口、隧道被占…）全部随进程一起消失，
    /// 界面上只剩一句笼统的「无法启动镜像窗口」。
    fn output_tail(&self) -> String {
        String::new()
    }
}

trait MirrorRuntime: Send + Sync {
    fn is_available(&self) -> bool;
    /// 启动镜像进程。`record_path` 非空时同时把画面录制成该文件。
    ///
    /// 录制与画面共用同一个 scrcpy 进程，因此**只能在启动时决定**，无法在会话中
    /// 单独开关；路径由调用方给出（前端不参与拼接本机路径）。
    fn start(
        &self,
        serial: &str,
        options: &SessionOptions,
        record_path: Option<&Path>,
    ) -> Result<Box<dyn MirrorProcess>, std::io::Error>;

    /// X10-92：启动**独立的录制进程**（`--no-playback --no-window --record`），
    /// 与显示进程并存、互不干扰——镜像窗口全程不重启。返回录制进程句柄；结束录制 = kill 它。
    ///
    /// X10-103：`desktop_display_id` 为显示通道虚拟屏 id（桌面模式时由调用方从显示
    /// 进程输出解析）。录制通道据此 `--display-id` 捕获**同一块屏**，而非自建第二块
    /// （那会挤掉显示通道→闪退白屏，或录到一块空屏）。非桌面模式传 `None`。
    fn start_recorder(
        &self,
        serial: &str,
        options: &SessionOptions,
        record_path: &Path,
        desktop_display_id: Option<u32>,
    ) -> Result<Box<dyn MirrorProcess>, std::io::Error>;
}

struct AppRuntimes {
    /// Arc 而非 Box：伴侣会话的桥接回调（设备报到 → 自动连接镜像通道）
    /// 需要在后台任务里持有同一个 adb 运行时。
    adb: Arc<dyn AdbRuntime>,
    mirror: Box<dyn MirrorRuntime>,
    /// 缓存 adb 序列号 → 硬件序列号的映射，避免每次轮询都对已授权设备重新 `getprop`。
    /// 序列号从 `adb devices` 中消失时即清除对应条目，避免陈旧映射。
    serial_cache: Mutex<HashMap<String, String>>,
}

impl AppRuntimes {
    fn system() -> Self {
        Self {
            adb: Arc::new(SystemAdbRuntime),
            mirror: Box::new(ScrcpyRuntime),
            serial_cache: Mutex::new(HashMap::new()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Quality {
    Smooth,
    #[default]
    Balanced,
    Sharp,
}

fn keep_awake_by_default() -> bool {
    true
}

fn keyboard_uhid_by_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SessionOptions {
    quality: Quality,
    fullscreen: bool,
    always_on_top: bool,
    rotation: u16,
    /// 会话期间让设备保持唤醒（对应 scrcpy `--stay-awake`）。
    ///
    /// 这是「手机用着用着就自己锁上、镜像变成锁屏界面」的直接对策：让设备在镜像
    /// 期间不休眠。scrcpy 退出时会恢复设备原有的休眠与电量策略。默认开启。
    #[serde(default = "keep_awake_by_default")]
    keep_awake: bool,
    /// 是否把本会话的画面录制成 MP4 保存到本机。
    ///
    /// **默认关闭**：录制会产生大文件，而且文件内容就是屏幕像素。与 `keep_awake`
    /// 的默认值取向不同——「保护用户」的默认值可以替用户打开，「替用户留存一份屏幕
    /// 副本」必须由用户自己决定。
    record: bool,
    /// 电脑与手机之间自动同步剪贴板（对应 scrcpy 的默认行为；关闭时传
    /// `--no-clipboard-autosync`）。
    ///
    /// **默认开启**：双向剪贴板是镜像控制体验的一部分，设备剪贴板变化时会同步到
    /// 电脑、粘贴前会把电脑剪贴板同步到设备。提供关闭开关的原因是隐私取向——
    /// 有的用户不希望手机上复制的内容自动出现在电脑剪贴板里。剪贴板文本内容
    /// 由 scrcpy 进程内部处理，不经过 MirrorDock，也不入日志。
    clipboard_autosync: bool,
    /// 是否把手机播放的系统声音转发到电脑（对应 scrcpy 默认行为；关闭时传
    /// `--no-audio`）。
    ///
    /// **默认开启**：听得到手机声音是「在电脑上用手机」体验的一部分。两个限制是
    /// 系统行为，须如实告知而非绕过：（1）系统音频捕获要求 Android 11+，更老的
    /// 设备上 scrcpy 会**自动禁用音频**继续镜像（MirrorDock 不假装能转发）；
    /// （2）应用可以通过捕获策略退出（通话、部分受保护应用无声）。另外这里只
    /// 转发系统播放声音（scrcpy `--audio-source` 默认 `output`），**不提供麦克风
    /// 采集**——麦克风是更敏感的隐私面，MVP 不开放。
    audio: bool,
    /// 镜像窗口快捷键的修饰键（scrcpy `--shortcut-mod`）。
    ///
    /// `None` 表示沿用 scrcpy 默认（左 Alt 或 左 Super）。合法值限定为白名单：
    /// `lctrl` / `rctrl` / `lalt` / `ralt` / `lsuper` / `rsuper`。MVP 只提供单键
    /// 选择——scrcpy 支持 `+` 组合与逗号分组，但非技术用户不需要那层自由度。
    #[serde(default)]
    shortcut_mod: Option<String>,
    /// 在镜像画面中显示手机上的**物理**触摸点（scrcpy `--show-touches`）。
    ///
    /// **默认关闭**：它会在演示期间开启设备的系统级「显示触摸点」，虽由 scrcpy
    /// 在退出时恢复原值，但默认替用户改设备设置并不合适——这是演示/教学场景的
    /// 主动选择。scrcpy 文档明确：只显示物理触摸，不显示 scrcpy 自己注入的点击。
    #[serde(default)]
    show_touches: bool,
    /// 键盘输入模式（scrcpy `--keyboard`）。
    ///
    /// `true`（默认）＝ UHID 物理键盘模式：手机把电脑当作外接硬件键盘，
    /// 输入文本框时**全屏软键盘收起为小候选条**，直接用电脑键盘打字（中文候选
    /// 由手机输入法的小候选条完成）。`false` ＝ scrcpy 默认注入模式：电脑输入法
    /// 组好的文字直接注入手机，但手机软键盘会照常弹出、遮住下半屏。
    ///
    /// 默认取 UHID：用户反馈「微信发送时唤起手机自带输入法，不方便输入」——
    /// 软键盘遮挡是镜像控制场景的主要痛点；UHID 不影响点击、剪贴板与快捷键。
    #[serde(default = "keyboard_uhid_by_default")]
    keyboard_uhid: bool,
    /// 只读演示模式（scrcpy `--no-control`）：电脑键鼠不控制手机，只观看画面。
    ///
    /// **默认关闭**。这是「隐私遮罩」的诚实替代：scrcpy 没有任何遮盖画面内容的
    /// 能力（无滤镜、无遮挡层），向他人演示时的真正风险是误操作——只读模式直接
    /// 消除它。需要隐藏敏感内容时，唯一诚实的建议仍是「先在手机上处理（勿扰
    /// 模式/退出应用），或直接结束镜像」。
    #[serde(default)]
    read_only: bool,
    /// 帧率上限（scrcpy `--max-fps`，X10-44）。
    ///
    /// `None`（默认）＝ 跟随设备，不传参数。给老设备降载/省电（长会话）的用户
    /// 一个明确上限：白名单 24/30/60，白名单外的值一律拒绝，不透传任意数字。
    #[serde(default)]
    max_fps: Option<u32>,
    /// 桌面模式（scrcpy `--new-display`，X10-49，v0.4）。
    ///
    /// 开启后不再镜像手机现有屏幕，而是在手机上创建一块**独立虚拟显示器**：
    /// 手机上可以从容操作（比如回微信），电脑上的窗口不被打断——类似三星 DeX
    /// 的体验。需要 Android 10+；低版本系统上 scrcpy 会失败，前端能力说明会
    /// 如实提示。与摄像头源互斥（同一时刻只能有一个视频源）。
    #[serde(default)]
    desktop_mode: bool,
    /// 桌面模式下虚拟屏启动的应用（scrcpy `--start-app`，X10-53）。
    ///
    /// 仅在 `desktop_mode` 为 true 时生效。留空 = 启动系统桌面（launcher）；
    /// 但部分机型（实测 MIUI）的桌面**不在虚拟显示器上渲染**（黑屏/白屏），
    /// 因此提供「直接在虚拟屏打开指定应用」：如 `com.android.browser`。
    /// 包名只允许字母、数字、点、下划线（白名单校验，不透传任意字符串）。
    #[serde(default)]
    desktop_app: Option<String>,
    /// 摄像头源（scrcpy `--video-source=camera`，X10-50，v0.4）。
    ///
    /// 把手机后置摄像头当作电脑上的网络摄像头画面。**默认关闭**且显式选择：
    /// 摄像头是敏感隐私面，绝不默认开启。摄像头源模式下**强制关闭音频**
    /// （scrcpy 默认会转摄像头麦克风；我们不采集任何麦克风，见 A1-05 的承诺）。
    /// 与桌面模式互斥。
    #[serde(default)]
    camera_source: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            quality: Quality::default(),
            fullscreen: false,
            always_on_top: false,
            rotation: 0,
            keep_awake: true,
            record: false,
            clipboard_autosync: true,
            audio: true,
            shortcut_mod: None,
            show_touches: false,
            keyboard_uhid: true,
            read_only: false,
            max_fps: None,
            desktop_mode: false,
            desktop_app: None,
            camera_source: false,
        }
    }
}

impl SessionOptions {
    fn arguments(&self) -> Result<Vec<String>, AppError> {
        if ![0, 90, 180, 270].contains(&self.rotation) {
            return Err(AppError::new(
                "invalid_rotation",
                "旋转角度无效。",
                "请选择 0、90、180 或 270 度。",
            ));
        }
        // 视频源互斥（X10-49/X10-50）：一个会话只能有一个画面来源。
        if self.desktop_mode && self.camera_source {
            return Err(AppError::new(
                "video_source_conflict",
                "桌面模式与摄像头画面不能同时开启。",
                "请只选择其中一个视频来源。",
            ));
        }
        let (size, bitrate) = match self.quality {
            Quality::Smooth => (1024, "2M"),
            Quality::Balanced => (1920, "8M"),
            Quality::Sharp => (2560, "16M"),
        };
        let mut args = vec![
            format!("--max-size={size}"),
            format!("--video-bit-rate={bitrate}"),
            "--video-codec=h264".into(),
        ];
        // rotation == 0 表示「自动（跟随手机）」：不传 --display-orientation，
        // scrcpy 会随设备旋转（如打开横屏游戏）自动转正画面并调整窗口大小。
        // 只有用户显式选择 90/180/270 时才锁定方向。
        if self.rotation != 0 {
            args.push(format!("--display-orientation={}", self.rotation));
        }
        if self.keep_awake {
            args.push("--stay-awake".into());
        }
        // 键盘模式二选一，显式传参不依赖 scrcpy 默认值（见 SessionOptions 文档）。
        args.push(if self.keyboard_uhid {
            "--keyboard=uhid".into()
        } else {
            "--keyboard=scrcpy".into()
        });
        if !self.clipboard_autosync {
            args.push("--no-clipboard-autosync".into());
        }
        if !self.audio {
            args.push("--no-audio".into());
        }
        if let Some(modifier) = &self.shortcut_mod {
            // 白名单校验：不把任意值透传进 scrcpy 参数。
            if !["lctrl", "rctrl", "lalt", "ralt", "lsuper", "rsuper"].contains(&modifier.as_str()) {
                return Err(AppError::new(
                    "shortcut_mod_invalid",
                    "快捷键修饰键无效。",
                    "请重新打开设置；若仍报错请恢复默认设置。",
                ));
            }
            args.push(format!("--shortcut-mod={modifier}"));
        }
        if self.show_touches {
            args.push("--show-touches".into());
        }
        if let Some(fps) = self.max_fps {
            // 白名单校验：不把任意数字透传进 scrcpy 参数（X10-44）。
            if ![24, 30, 60].contains(&fps) {
                return Err(AppError::new(
                    "max_fps_invalid",
                    "帧率上限无效。",
                    "请选择「跟随设备」、24、30 或 60。",
                ));
            }
            args.push(format!("--max-fps={fps}"));
        }
        // 视频源（X10-49/X10-50）：默认「手机屏幕」不传参数。
        // 桌面模式 = --new-display（独立虚拟显示器，主屏尺寸）。
        // 摄像头源 = --video-source=camera，同时强制 --no-audio：scrcpy 的
        // 摄像头源会把音频源切到麦克风，我们绝不采集麦克风（A1-05 承诺）。
        if self.desktop_mode {
            args.push("--new-display".into());
            // X10-53：虚拟屏启动指定应用。实测 MIUI 桌面不在虚拟显示器上渲染
            // （黑屏/白屏），启动普通应用则正常——这是该机型上桌面模式可用的前提。
            if let Some(app) = &self.desktop_app {
                let pkg = app.trim();
                // 白名单校验：Android 包名字符集固定，绝不把任意字符串透传进参数。
                let valid = !pkg.is_empty()
                    && pkg.len() <= 120
                    && pkg
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
                if !valid {
                    return Err(AppError::new(
                        "desktop_app_invalid",
                        "虚拟屏启动的应用包名无效。",
                        "包名只允许字母、数字、点（.）和下划线，例如 com.android.browser；也可以留空。",
                    ));
                }
                args.push(format!("--start-app={pkg}"));
            }
        } else if self.camera_source {
            args.push("--video-source=camera".into());
            if !args.iter().any(|argument| argument == "--no-audio") {
                args.push("--no-audio".into());
            }
        }
        if self.read_only {
            args.push("--no-control".into());
        }
        if self.fullscreen {
            args.push("--fullscreen".into());
        }
        if self.always_on_top {
            args.push("--always-on-top".into());
        }
        Ok(args)
    }

    /// X10-92：录制通道（独立 scrcpy 进程）的参数集。
    ///
    /// 与显示通道 [`arguments`] 的关键差异——录制进程**只录不显示**，因此：
    ///  - 强制 `--no-playback --no-window --no-control`：不开窗、不显示、不接收输入，
    ///    让它纯粹做「设备编码流 → 本地 MP4」的封装器；
    ///  - 只保留**影响视频流本身**的参数（分辨率/码率/编码/朝向/帧率/视频源），
    ///    丢弃窗口形态（全屏/置顶）与输入/交互（键盘/剪贴板/快捷键/触摸显示/
    ///    保持唤醒/只读）——这些对一个无窗录制进程没有意义；
    ///  - 音频保留（除非摄像头源强制静音）：录制要的就是画面+声音。
    ///
    /// 这样录制进程的启停完全不触碰显示进程——镜像窗口全程不重启。
    fn record_arguments(&self, desktop_display_id: Option<u32>) -> Result<Vec<String>, AppError> {
        let (size, bitrate) = match self.quality {
            Quality::Smooth => (1024, "2M"),
            Quality::Balanced => (1920, "8M"),
            Quality::Sharp => (2560, "16M"),
        };
        let mut args = vec![
            format!("--max-size={size}"),
            format!("--video-bit-rate={bitrate}"),
            "--video-codec=h264".into(),
            // 录制通道默认无窗、不播放；控制见下方桌面模式分支。
            "--no-playback".into(),
            "--no-window".into(),
        ];
        if self.rotation != 0 {
            // 录制用 display-orientation 锁定方向（与显示通道同一语义）。
            args.push(format!("--display-orientation={}", self.rotation));
        }
        if let Some(fps) = self.max_fps {
            if ![24, 30, 60].contains(&fps) {
                return Err(AppError::new(
                    "max_fps_invalid",
                    "帧率上限无效。",
                    "请选择「跟随设备」、24、30 或 60。",
                ));
            }
            args.push(format!("--max-fps={fps}"));
        }
        // 视频源必须与显示通道一致：桌面模式录虚拟屏、摄像头录摄像头、否则录手机屏幕。
        if self.desktop_mode {
            // X10-103：录制通道用 `--display-id` 捕获**显示通道那块虚拟屏**，不建第二块。
            //
            // 根因链（三次实测定位）：
            //  - X10-101 让录制进程 `--new-display --start-app` 自己起一块屏并拉起应用 →
            //    把游戏「搬」到第二块屏、显示通道那块变白 → 用户看到「镜像闪退 + 白屏」。
            //  - 去掉 --start-app 只留 --new-display → 录的是录制进程自己的空虚拟屏，
            //    录不到显示通道上的游戏画面。
            //  - 正解：录制通道**不建屏**，用 `--display-id=<显示通道的虚拟屏 id>` 直接捕获
            //    同一块屏。实测：1920×848、139帧/6.4s≈22fps、录到游戏真实画面，且显示
            //    通道全程不闪退。id 由显示通道 scrcpy 输出里的 `New display: ... (id=N)`
            //    解析得到（parse_desktop_display_id）。
            //
            // 拿不到 id（显示通道输出还没产出该行、或解析失败）时**拒绝启动**——宁可让用户
            // 看到「稍候重试」，也不要静默录一块空屏（那正是 X10-101 想修的白屏）。
            let Some(display_id) = desktop_display_id else {
                return Err(AppError::new(
                    "desktop_display_unknown",
                    "暂时无法开始录制。",
                    "桌面模式的虚拟画面还没就绪，请稍候几秒再点开始录制。",
                ));
            };
            args.push(format!("--display-id={display_id}"));
            // 不建屏、不重复 --start-app：显示通道已拉起应用，录制只捕获同一块屏。
        } else if self.camera_source {
            args.push("--video-source=camera".into());
            // 摄像头源绝不采集麦克风（A1-05 承诺），录制同样静音。
            args.push("--no-audio".into());
            // 录摄像头/主屏的真实源无需控制通道，保持无控制以最小化设备占用。
            args.push("--no-control".into());
        } else {
            // 录手机真实主屏：同样无需控制通道。
            args.push("--no-control".into());
            if !self.audio {
                args.push("--no-audio".into());
            }
        }
        Ok(args)
    }
}

struct SystemAdbRuntime;

/// 创建一个「安静」的子进程命令：Windows 上附加 `CREATE_NO_WINDOW`，
/// 避免后台调用 adb（控制台子系统程序）时反复弹出黑色控制台窗口。
/// scrcpy 是 GUI 子系统程序，本就没有控制台，该标志对它无副作用；
/// 非 Windows 平台原样返回。
fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    // `mut` 仅在下方 Windows 分支被真正使用；非 Windows 平台上保持声明不动，
    // 用 allow 压制 rustc 的 unused_mut（删掉 mut 会破坏 Windows 构建）。
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // 0x08000000 = CREATE_NO_WINDOW：只为子进程不分配新控制台，
        // 不影响 GUI 窗口（镜像画面）本身的显示。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

impl SystemAdbRuntime {
    /// 固定参数直接调用 `adb`，不使用 shell，也不做字符串拼接。
    /// 与 `run` 相同，但接受拥有所有权的参数。
    ///
    /// 存在的理由：经过 shell 引号处理的路径（`shell_quote` 的输出）是一个
    /// 临时 `String`，无法借用成 `&str` 塞进 `&[&str]`。与其在调用点
    /// `leak` 或克隆一份，不如让这条路径显式拥有参数。
    fn run_owned(args: &[String]) -> Result<(), std::io::Error> {
        let output = Self::output_with_timeout(args)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    fn run(args: &[&str]) -> Result<(), std::io::Error> {
        let output = Self::output_with_timeout(args)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    /// 读取 `adb` 的 stdout。同样使用固定参数直接调用，不做任何 shell 拼接或插值。
    fn capture(args: &[&str]) -> Result<String, std::io::Error> {
        let output = Self::output_with_timeout(args)?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    /// 带超时执行 adb：Windows 上 adb 对掉线/未授权设备可能长时间不返回，
    /// 无超时会把 Tauri 命令线程占死 → 前端「未响应」（X10-84）。
    /// 超过 `ADB_CALL_TIMEOUT` 即强杀子进程并按超时错误返回，由上层按「设备暂时不可用」处理。
    /// 带超时执行 adb 并取回完整输出。
    ///
    /// 不能在调用线程里 `try_wait + sleep` 轮询：tauri 同步命令跑在主线程，
    /// 忙等循环会把 UI 冻住（0.4.15 实测 Mac 启动即卡死的回归根因）。
    /// 这里把整条 `spawn + wait_with_output` 放进独立线程，调用线程只做
    /// 一次 `recv_timeout` 阻塞等待——主线程被 park（不烧 CPU），超时即返回。
    /// 超时的子进程由子线程负责 kill 回收，主线程不碰。
    fn output_with_timeout<S: AsRef<std::ffi::OsStr>>(
        args: &[S],
    ) -> Result<std::process::Output, std::io::Error> {
        let owned: Vec<std::ffi::OsString> = args.iter().map(|a| a.as_ref().to_os_string()).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = quiet_command(adb_binary())
                .args(&owned)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .stdin(std::process::Stdio::null())
                .spawn()
                .and_then(|child| child.wait_with_output());
            // 发送失败只意味着调用线程已超时离开，丢弃即可。
            let _ = tx.send(result);
        });
        match rx.recv_timeout(ADB_CALL_TIMEOUT) {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "adb 调用超时（设备可能掉线或未授权）",
            )),
        }
    }
}

impl AdbRuntime for SystemAdbRuntime {
    fn list_devices(&self) -> Result<Vec<AdbDevice>, std::io::Error> {
        let output = Self::output_with_timeout(&["devices", "-l"])?;
        if output.status.success() {
            Ok(parse_adb_devices(&String::from_utf8_lossy(&output.stdout)))
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    fn device_properties(&self, serial: &str) -> Result<String, std::io::Error> {
        // 固定参数直接调用：serial 作为单个 argv 传入，不做任何 shell 拼接或插值。
        let output = Self::output_with_timeout(&["-s", serial, "shell", "getprop"])?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    fn physical_serial(&self, serial: &str) -> Result<Option<String>, std::io::Error> {
        // 固定参数直接调用：serial 作为单个 argv 传入，不做任何 shell 拼接或插值。
        // 仅已授权设备能执行 shell；未授权/离线设备会失败，此时返回 `Ok(None)`。
        let raw = Self::capture(&["-s", serial, "shell", "getprop", "ro.serialno"])?;
        Ok(sanitize_property(&raw))
    }

    fn wake_screen(&self, serial: &str) -> Result<(), std::io::Error> {
        // 只发送唤醒键。这里刻意不发送任何解锁相关的输入。
        Self::run(&["-s", serial, "shell", "input", "keyevent", "KEYCODE_WAKEUP"])
    }

    fn screen_off_timeout(&self, serial: &str) -> Result<Option<u64>, std::io::Error> {
        let raw = Self::capture(&[
            "-s",
            serial,
            "shell",
            "settings",
            "get",
            "system",
            "screen_off_timeout",
        ])?;
        Ok(parse_settings_number(&raw))
    }

    fn set_screen_off_timeout(&self, serial: &str, millis: u64) -> Result<(), std::io::Error> {
        // millis 转字符串后作为单个 argv 传入，不做任何 shell 拼接。
        let value = millis.to_string();
        Self::run(&[
            "-s",
            serial,
            "shell",
            "settings",
            "put",
            "system",
            "screen_off_timeout",
            &value,
        ])
    }

    fn stay_on_while_plugged_in(&self, serial: &str) -> Result<Option<u64>, std::io::Error> {
        let raw = Self::capture(&[
            "-s",
            serial,
            "shell",
            "settings",
            "get",
            "global",
            "stay_on_while_plugged_in",
        ])?;
        Ok(parse_settings_number(&raw))
    }

    fn set_stay_on_while_plugged_in(&self, serial: &str, bits: u64) -> Result<(), std::io::Error> {
        let value = bits.to_string();
        Self::run(&[
            "-s",
            serial,
            "shell",
            "settings",
            "put",
            "global",
            "stay_on_while_plugged_in",
            &value,
        ])
    }

    fn window_policy(&self, serial: &str) -> Result<String, std::io::Error> {
        Self::capture(&["-s", serial, "shell", "dumpsys", "window", "policy"])
    }

    fn display_state(&self, serial: &str) -> Result<String, std::io::Error> {
        Self::capture(&["-s", serial, "shell", "dumpsys", "display"])
    }

    fn press_key(&self, serial: &str, keycode: &str) -> Result<(), std::io::Error> {
        // keycode 来自代码内常量，作为单个 argv 传入，不做任何 shell 拼接。
        Self::run(&["-s", serial, "shell", "input", "keyevent", keycode])
    }

    fn screencap_probe_bytes(&self, serial: &str) -> Result<u64, std::io::Error> {
        // 与 screenshot_png 同一条 exec-out 二进制安全通道，但只统计字节数：
        // 屏幕像素就地丢弃，不落盘、不回传、绝不入日志。
        let output = quiet_command(adb_binary())
            .args(["-s", serial, "exec-out", "screencap", "-p"])
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other("adb returned a failing status"));
        }
        Ok(output.stdout.len() as u64)
    }

    fn power_state(&self, serial: &str) -> Result<String, std::io::Error> {
        Self::capture(&["-s", serial, "shell", "dumpsys", "power"])
    }

    fn screenshot_png(&self, serial: &str) -> Result<Vec<u8>, std::io::Error> {
        // 用 `exec-out` 而不是 `shell`：后者会把 stdout 当作文本流，在 Windows 上
        // 可能把 \n 改写成 \r\n，从而破坏 PNG 二进制。
        // 固定参数直接调用：serial 作为单个 argv 传入，不做任何 shell 拼接或插值。
        let output = quiet_command(adb_binary())
            .args(["-s", serial, "exec-out", "screencap", "-p"])
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other("adb returned a failing status"));
        }
        Ok(output.stdout)
    }

    fn make_directory(&self, serial: &str, remote_dir: &str) -> Result<(), std::io::Error> {
        Self::run(&["-s", serial, "shell", "mkdir", "-p", remote_dir])
    }

    fn list_directory(&self, serial: &str, remote_dir: &str) -> Result<String, std::io::Error> {
        // `-1`：每行恰好一个条目，文件名里的空格不会被拆开。
        Self::capture(&["-s", serial, "shell", "ls", "-1", remote_dir])
    }

    fn push_file(
        &self,
        serial: &str,
        local: &Path,
        remote_dir: &str,
    ) -> Result<String, std::io::Error> {
        // 本机路径与设备路径都作为单个 argv 传入：不做 shell 拼接或插值，文件名里
        // 带空格也安全。
        let output = quiet_command(adb_binary())
            .args(["-s", serial, "push"])
            .arg(local)
            .arg(remote_dir)
            .output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    fn pull_file(
        &self,
        serial: &str,
        remote_path: &str,
        local: &Path,
    ) -> Result<String, std::io::Error> {
        let output = quiet_command(adb_binary())
            .args(["-s", serial, "pull"])
            .arg(remote_path)
            .arg(local)
            .output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    fn remove_device_file(&self, serial: &str, remote_path: &str) -> Result<(), std::io::Error> {
        Self::run_owned(&remove_device_file_argv(serial, remote_path))
    }


    fn install_apk(&self, serial: &str, apk: &Path) -> Result<String, std::io::Error> {
        // 固定参数直接调用：APK 路径作为单个 argv 传入，不做 shell 拼接或插值，
        // 路径含空格、中文也安全。
        // `-r` 覆盖安装（保留应用数据）；`-t` 允许 test-only 包（开发版 APK 常见）。
        // 刻意**不用 `-g`**：那会在不与用户确认的情况下批量授予运行时权限，与
        // 「不绕过用户同意」的产品边界冲突——权限仍由用户在手机上逐项确认。
        let output = quiet_command(adb_binary())
            .args(["-s", serial, "install", "-r", "-t"])
            .arg(apk)
            .output()?;
        // 失败原因可能落在 stdout 或 stderr（视 adb 版本与平台而定），两路都收。
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.trim().is_empty() {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&stderr);
        }
        Ok(text)
    }

    fn pair(&self, endpoint: &str, pairing_code: &str) -> Result<(), std::io::Error> {
        Self::run(&["pair", endpoint, pairing_code])
    }

    fn connect(&self, endpoint: &str) -> Result<(), std::io::Error> {
        Self::run(&["connect", endpoint])
    }

    fn mdns_services(&self) -> Result<String, std::io::Error> {
        Self::capture(&["mdns", "services"])
    }

    fn list_device_apps(&self, serial: &str) -> Result<String, std::io::Error> {
        Self::capture(&["-s", serial, "shell", "pm", "list", "packages", "-3"])
    }

    fn activity_dumpsys(&self, serial: &str) -> Result<String, std::io::Error> {
        // argv 形式逐个传参，与后端其余 adb 调用一致（不经过设备端 shell 二次分词）。
        Self::capture(&[
            "-s",
            serial,
            "shell",
            "dumpsys",
            "activity",
            "activities",
        ])
    }

    fn disconnect(&self, endpoint: &str) -> Result<(), std::io::Error> {
        Self::run(&["disconnect", endpoint])
    }

    fn open_developer_settings(&self, serial: &str) -> Result<(), std::io::Error> {
        Self::run(&[
            "-s",
            serial,
            "shell",
            "am",
            "start",
            "-a",
            "android.settings.APPLICATION_DEVELOPMENT_SETTINGS",
        ])
    }

    fn open_keyboard_layout_settings(&self, serial: &str) -> Result<(), std::io::Error> {
        // X10-37 真机实证：该 action 直接落到「实体键盘」页（HARD_KEYBOARD_SETTINGS）。
        Self::run(&[
            "-s",
            serial,
            "shell",
            "am",
            "start",
            "-a",
            "android.settings.HARD_KEYBOARD_SETTINGS",
        ])
    }

    fn disable_wireless_debugging(&self, serial: &str) -> Result<(), std::io::Error> {
        Self::run(&[
            "-s",
            serial,
            "shell",
            "settings",
            "put",
            "global",
            "adb_wifi_enabled",
            "0",
        ])
    }

    fn disable_usb_debugging(&self, serial: &str) -> Result<(), std::io::Error> {
        Self::run(&[
            "-s", serial, "shell", "settings", "put", "global", "adb_enabled", "0",
        ])
    }
}

/// scrcpy 输出环形缓冲的容量（字节）。足够装下十几行报错，又不会无限增长。
const SCRCPY_OUTPUT_CAP: usize = 16 * 1024;

/// 收进环形缓冲的 scrcpy 输出片段。
///
/// 两路输出（stdout / stderr）由两个后台线程持续读取，任一读满即丢弃最旧的
/// 字节，**永远不会因为管道写满而把 scrcpy 进程堵死**——这是本改动最关键的一点：
/// 一旦没人读管道，scrcpy 会在写日志时阻塞，表现为「启动了但画面不出来」，
/// 那正是我们要修的故障本身。
#[derive(Default)]
struct ScrcpyOutputSink {
    text: std::sync::Mutex<String>,
}

impl ScrcpyOutputSink {
    fn append(&self, chunk: &[u8]) {
        let Ok(mut text) = self.text.lock() else {
            return;
        };
        // 只保留可打印文本：scrcpy 的日志是 UTF-8，但混入二进制也不至于让
        // 诊断包变成乱码——逐字节过滤掉控制字符即可。
        for byte in chunk {
            if byte.is_ascii_graphic() || *byte == b' ' || *byte == b'\n' || *byte == b'\t' {
                text.push(*byte as char);
            }
        }
        if text.len() > SCRCPY_OUTPUT_CAP {
            let excess = text.len() - SCRCPY_OUTPUT_CAP;
            text.drain(..excess);
        }
    }

    fn tail(&self) -> String {
        self.text.lock().map(|text| text.clone()).unwrap_or_default()
    }
}

/// 真实的镜像进程。`Drop` 时确保子进程被终止并回收，避免应用退出后残留 scrcpy。
struct SystemMirrorProcess {
    child: Child,
    /// scrcpy 的 stdout/stderr 汇聚到这里，供失败时给出真实原因（X10-79）。
    output: Arc<ScrcpyOutputSink>,
    /// X10-95：是否录制进程。录制进程停止时必须走 SIGINT（写 moov 索引定型 MP4），
    /// 而显示进程维持 SIGTERM→SIGKILL 的既有路径。
    is_recorder: bool,
}

impl SystemMirrorProcess {
    /// 接管子进程两路输出并开读。读线程是 detached 的：进程结束后管道自然 EOF，
    /// 读线程随之结束，不需要 join。
    fn capture_output(child: &mut Child) -> Arc<ScrcpyOutputSink> {
        let sink = Arc::new(ScrcpyOutputSink::default());
        for stream in [
            child.stdout.take().map(StreamKind::Out),
            child.stderr.take().map(StreamKind::Err),
        ]
        .into_iter()
        .flatten()
        {
            let sink = Arc::clone(&sink);
            std::thread::spawn(move || {
                let mut reader: Box<dyn std::io::Read + Send> = match stream {
                    StreamKind::Out(handle) => Box::new(handle),
                    StreamKind::Err(handle) => Box::new(handle),
                };
                let mut buffer = [0u8; 1024];
                while let Ok(count) = reader.read(&mut buffer) {
                    if count == 0 {
                        break;
                    }
                    sink.append(&buffer[..count]);
                }
            });
        }
        sink
    }
}

enum StreamKind {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

impl MirrorProcess for SystemMirrorProcess {
    fn try_wait(&mut self) -> Option<bool> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.success()),
            _ => None,
        }
    }

    fn output_tail(&self) -> String {
        self.output.tail()
    }

    fn kill(&mut self) -> Result<(), std::io::Error> {
        self.child.kill()
    }

    fn stop(&mut self) -> Result<(), std::io::Error> {
        // 已经退出就什么都不做：避免向已回收的 pid 发信号。
        if self.try_wait().is_some() {
            return Ok(());
        }
        #[cfg(unix)]
        {
            // X10-95：录制进程与显示进程用不同的「优雅停止」信号。
            //
            // - **录制进程（is_recorder=true）发 SIGINT**。scrcpy 的录制收尾
            //   （flush 剩余 packet + 写 moov 索引定型 MP4）只在收到 SIGINT 走
            //   正常退出路径时执行；发 SIGTERM 它不认，最终 SIGKILL 会让 MP4 缺
            //   moov 变砖——这就是「录出来全黑屏/无法播放」的根因。配合 spawn 时
            //   的 pre_exec 信号重置，SIGINT 能真正送达，实测约 1.5s 优雅退出。
            // - **显示进程（false）维持 SIGTERM**。scrcpy 4.1 对显示会话本就
            //   不响应 SIGTERM（X10-79 实测），会走下方超时强杀，行为不变。
            let stop_signal = if self.is_recorder { libc::SIGINT } else { libc::SIGTERM };
            // SAFETY: kill 只向本子进程的 pid 发信号，不触碰其它进程。
            let sent = unsafe { libc::kill(self.child.id() as libc::pid_t, stop_signal) };
            if sent != 0 {
                // 发送失败最常见的原因是进程恰好自行退出；能 reap 就视为已结束。
                if self.try_wait().is_some() {
                    return Ok(());
                }
                return Err(std::io::Error::last_os_error());
            }
            // X10-79：显示会话的 scrcpy 4.1 **不响应 SIGTERM**（真机实测：连发 6 秒
            // 仍在运行，只能靠 SIGKILL 结束）；录制会话的 SIGINT 通常 1.5s 内退出。
            // 统一等 PROCESS_GRACEFUL_TIMEOUT，超时再升级 SIGKILL——不再把
            // 整个等待当成「scrcpy 一定会优雅退出」的赌注。
            let graceful = std::time::Instant::now() + PROCESS_GRACEFUL_TIMEOUT;
            while std::time::Instant::now() < graceful {
                if self.try_wait().is_some() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            // 超时兜底强杀：宁可这次录像可能损坏，也不能让镜像窗口挂死。
            self.kill()?;
        }
        #[cfg(not(unix))]
        {
            // Windows 没有 SIGINT/SIGTERM 对应物：TerminateProcess 立即结束进程，
            // 录制中的 MP4 可能缺 moov 索引。该平台差异如实记录，待 Windows
            // 真机验证后决定是否引入平台特定的优雅退出手段。
            self.kill()?;
        }
        // 强杀路径也等进程退出被回收，避免留下僵尸进程。
        let deadline = std::time::Instant::now() + PROCESS_REAP_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if self.try_wait().is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
}

impl Drop for SystemMirrorProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct ScrcpyRuntime;

impl MirrorRuntime for ScrcpyRuntime {
    fn is_available(&self) -> bool {
        is_scrcpy_available()
    }

    fn start(
        &self,
        serial: &str,
        options: &SessionOptions,
        record_path: Option<&Path>,
    ) -> Result<Box<dyn MirrorProcess>, std::io::Error> {
        // macOS：经带图标的 bundle 启动（Dock 显示「MirrorDock 镜像」）；
        // 失败或非 macOS 时回退为直接启动 scrcpy。显式传 ADB 指向随包 adb，
        // bundle 与裸二进制两种形态都能找到它。
        let scrcpy = scrcpy_binary();
        let adb = adb_binary();
        let launch = macos_mirror_bundle_exec(&scrcpy, &adb).unwrap_or_else(|| scrcpy.clone());
        let mut command = quiet_command(launch);
        command.env("ADB", &adb);
        command
            .arg("--serial")
            .arg(serial)
            .args(
                options
                    .arguments()
                    .map_err(|_| std::io::Error::other("invalid session options"))?,
            );
        if let Some(path) = record_path {
            // 固定参数直接调用：路径作为单个 argv 传入，不做任何 shell 拼接或插值。
            command
                .arg(format!("--record={}", path.to_string_lossy()))
                .arg("--record-format=mp4");
        }
        Self::spawn_scrcpy(command, false)
    }

    fn start_recorder(
        &self,
        serial: &str,
        options: &SessionOptions,
        record_path: &Path,
        desktop_display_id: Option<u32>,
    ) -> Result<Box<dyn MirrorProcess>, std::io::Error> {
        // X10-92：录制进程不开窗，**不经 macOS bundle**（bundle 只为 Dock 图标服务，
        // 无窗进程用裸 scrcpy 即可）。参数集用 record_arguments（无窗/不播放）。
        let scrcpy = scrcpy_binary();
        let adb = adb_binary();
        let mut command = quiet_command(scrcpy);
        command.env("ADB", &adb);
        command
            .arg("--serial")
            .arg(serial)
            .args(
                options
                    .record_arguments(desktop_display_id)
                    .map_err(|_| std::io::Error::other("invalid record options"))?,
            )
            .arg(format!("--record={}", record_path.to_string_lossy()))
            .arg("--record-format=mp4");
        Self::spawn_scrcpy(command, true)
    }
}

impl ScrcpyRuntime {
    /// 抽出公共的子进程启动：接管 stdio（避免 GUI 应用丢失 scrcpy 报错 + 防管道写满阻塞）。
    ///
    /// X10-95：Unix 下在 `pre_exec` 把 SIGINT/SIGTERM 重置为 SIG_DFL。
    /// Tauri/桌面环境 spawn 的子进程会继承「忽略 SIGINT」，而 scrcpy 的录制收尾
    /// （写 moov 索引）只在收到 SIGINT 走正常退出路径时执行——不重置的话，
    /// 后续 `stop()` 发 SIGINT 会被进程无视，最终只能 SIGKILL，MP4 缺 moov 变砖。
    fn spawn_scrcpy(mut command: Command, is_recorder: bool) -> Result<Box<dyn MirrorProcess>, std::io::Error> {
        // X10-79：必须显式 piped。默认情况下子进程继承父进程的 stdio，
        // MirrorDock 是 GUI 应用、没有可用终端，scrcpy 的报错会**直接丢失**——
        // 这正是「用户报白屏、日志里什么都看不到」的原因。接管后既能读到原文，
        // 也顺带避免管道写满把 scrcpy 阻塞住。
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        command.stdin(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: pre_exec 里只做 async-signal-safe 的 signal() 重置，不分配内存、
            // 不碰锁、不调任何非信号安全函数。把可能被父进程忽略的信号恢复默认，
            // 让 scrcpy 自己的信号处理器能正常接管 SIGINT。
            unsafe {
                command.pre_exec(|| {
                    libc::signal(libc::SIGINT, libc::SIG_DFL);
                    libc::signal(libc::SIGTERM, libc::SIG_DFL);
                    libc::signal(libc::SIGHUP, libc::SIG_DFL);
                    Ok(())
                });
            }
        }
        let mut child = command.spawn()?;
        let output = SystemMirrorProcess::capture_output(&mut child);
        Ok(Box::new(SystemMirrorProcess { child, output, is_recorder }))
    }
}

// ---------------------------------------------------------------------------
// 会话存储：状态与进程句柄同源，避免状态与真实进程脱节
// ---------------------------------------------------------------------------

struct SessionState {
    session: MirrorSession,
    process: Option<Box<dyn MirrorProcess>>,
    /// 每次状态跃迁递增。监视线程据此判断自己是否仍然拥有当前会话。
    epoch: u64,
    /// 无线会话的熄屏时间备份（见 `enable_wireless_keep_awake`）。
    ///
    /// 会话进行中存在 ⇒ 设备的 `screen_off_timeout` 当前是 MirrorDock 延长后的值；
    /// 为 `None` 表示未做过补偿。还原成功或从未补偿时清空。
    keep_awake_backup: Option<KeepAwakeBackup>,
    /// 当前会话**实际使用**的启动参数。
    ///
    /// 保存在这里的原因是：镜像窗口是独立进程，窗口形态（全屏、置顶、旋转、画质）
    /// 在启动时确定，无法在运行中改写。要让界面能判断「这次修改是否真的需要重启
    /// 镜像窗口」，就必须知道上一次启动到底用了什么参数。
    options: SessionOptions,
    /// 最近一次会话的录制文件路径（若开启了录制）。
    ///
    /// 会话结束后**刻意保留**：录好的文件仍在磁盘上，用户需要能看到它、打开它或删掉
    /// 它。至于「是否仍在录制」，由是否存在运行中的进程决定，而不是由这个字段决定。
    record_path: Option<String>,
    /// X10-92：独立录制进程的句柄（双通道方案）。
    ///
    /// `Some` ⇒ 正在录制（录制进程在跑）；`None` ⇒ 未在录制。它与显示 `process`
    /// 完全解耦：开始/结束录制只 spawn/kill 这条进程，显示进程不动、镜像窗口不重启。
    /// 「是否仍在录制」改由这个字段判定，而非 `record_path + process`。
    record_process: Option<Box<dyn MirrorProcess>>,
}

/// 会话表（X10-27 并发多设备）：**每台设备一个会话**。
///
/// 键为设备序列号（USB 序列号或无线端点），值为该设备的完整会话状态。
/// 旧版本是全局单实例：任意一台设备在镜像中时，其它设备一律 `session_busy`；
/// 现在互斥只收在「同一台设备」上——两台设备可以同时各有一个镜像会话，
/// scrcpy 本身支持多实例，互不干扰。`BTreeMap` 保证遍历顺序按序列号稳定，
/// 「主会话」（primary）的选取因此是确定性的。
struct SessionStore(Arc<Mutex<BTreeMap<String, SessionState>>>);

impl Default for SessionStore {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(BTreeMap::new())))
    }
}

impl SessionStore {
    fn lock(&self) -> Result<MutexGuard<'_, BTreeMap<String, SessionState>>, AppError> {
        self.0.lock().map_err(|_| {
            AppError::new(
                "session_unavailable",
                "会话状态不可用。",
                "请重启应用后再试。",
            )
        })
    }
}

impl Clone for SessionStore {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

fn is_session_active(state: &SessionState) -> bool {
    state.process.is_some()
        || matches!(
            state.session.phase,
            SessionPhase::Connecting | SessionPhase::Streaming
        )
}

/// 取（或创建）某台设备的会话条目。只用于**即将写入**该设备状态的路径；
/// 纯读取路径直接 `map.get`，避免无谓地插入空条目。
fn session_entry_mut<'a>(
    map: &'a mut BTreeMap<String, SessionState>,
    serial: &str,
) -> &'a mut SessionState {
    map.entry(serial.to_owned()).or_insert_with(|| SessionState {
        session: MirrorSession::idle(),
        process: None,
        epoch: 0,
        keep_awake_backup: None,
        options: SessionOptions::default(),
        record_path: None,
        record_process: None,
    })
}

/// 某台设备的会话是否在进行中。
fn device_session_active(map: &BTreeMap<String, SessionState>, serial: &str) -> bool {
    map.get(serial).is_some_and(is_session_active)
}

/// 是否存在任何进行中的会话（任意设备）。
fn any_session_active(map: &BTreeMap<String, SessionState>) -> bool {
    map.values().any(is_session_active)
}

/// 主会话的序列号：优先真正持有镜像进程的设备，其次处于启动中的设备，
/// 最后按序列号取第一台有状态记录的设备。没有任何记录时返回 `None`。
///
/// 主会话用于向后兼容的单会话界面（`mirror_session` 命令、托盘菜单、设置应用）
/// ——多会话界面上线前，这些入口仍按「最相关的一台」工作。
fn primary_session_serial(map: &BTreeMap<String, SessionState>) -> Option<String> {
    map.iter()
        .find(|(_, state)| state.process.is_some())
        .or_else(|| {
            map.iter()
                .find(|(_, state)| is_session_active(state))
        })
        .map(|(serial, _)| serial.clone())
        .or_else(|| map.keys().next().cloned())
}

/// 占位一个新的会话（X10-27）：互斥只收在**同一台设备**上。
///
/// 同一台设备已有会话运行或启动中时拒绝覆盖，即使序列号相同；其它设备的会话
/// 不受影响——这正是并发多设备支持的核心语义。
fn reserve_session(store: &SessionStore, session: MirrorSession) -> Result<(), AppError> {
    let Some(serial) = session.serial.clone().filter(|value| !value.is_empty()) else {
        return Err(AppError::new(
            "device_not_selected",
            "未选择可用设备。",
            "请重新检查连接后选择手机。",
        ));
    };
    let mut map = store.lock()?;
    if device_session_active(&map, &serial) {
        return Err(AppError::new(
            "session_busy",
            "这台设备已有镜像窗口正在运行或启动。",
            "请先结束该设备的当前会话，再启动新的会话。",
        ));
    }
    let state = session_entry_mut(&mut map, &serial);
    // 保留 keep_awake_backup：上一次无线会话若留下未还原的亮屏补偿账本，
    // 新会话启动时按账本幂等重写延长值（而不是把已被我们改过的值再记一次原值）。
    state.epoch = state.epoch.wrapping_add(1);
    state.session = session;
    state.process = None;
    state.options = SessionOptions::default();
    state.record_path = None;
    Ok(())
}

/// 直接改写某台设备的会话状态（会终止对当前进程的跟踪，由 `Drop` 负责回收）。
///
/// 目标阶段为 `Idle` 时移除该设备的条目（按设备无会话 = 表中无记录）。
fn mark_session(store: &SessionStore, session: MirrorSession) {
    let Some(serial) = session.serial.clone() else {
        return;
    };
    if let Ok(mut map) = store.0.lock() {
        if session.phase == SessionPhase::Idle {
            map.remove(&serial);
        } else {
            let state = session_entry_mut(&mut map, &serial);
            state.epoch = state.epoch.wrapping_add(1);
            state.process = None;
            state.session = session;
        }
    }
}

fn fail_session(store: &SessionStore, serial: Option<String>, error: AppError) -> AppError {
    if serial.is_some() {
        mark_session(store, MirrorSession::failed(serial.clone(), error.clone()));
    }
    error
}

/// 把进程接入某台设备的会话，进入 `Streaming`，并返回本次会话的 epoch。
fn attach_process(
    store: &SessionStore,
    process: Box<dyn MirrorProcess>,
    serial: String,
    options: SessionOptions,
    record_path: Option<String>,
) -> Result<u64, AppError> {
    let mut map = store.lock()?;
    let state = session_entry_mut(&mut map, &serial);
    state.epoch = state.epoch.wrapping_add(1);
    state.session = MirrorSession::streaming(serial);
    state.process = Some(process);
    state.options = options;
    state.record_path = record_path;
    Ok(state.epoch)
}

/// 结束某台设备的会话并交还进程句柄；该设备没有运行中的会话时返回 `None`。
fn take_running_process(
    store: &SessionStore,
    serial: &str,
) -> Result<Option<Box<dyn MirrorProcess>>, AppError> {
    let mut map = store.lock()?;
    let Some(state) = map.get_mut(serial) else {
        return Ok(None);
    };
    let process = state.process.take();
    if process.is_some() {
        state.epoch = state.epoch.wrapping_add(1);
        state.session = MirrorSession::idle();
        // X10-92：显示会话结束，独立录制进程一并停掉（写 moov 定型），不留下
        // 「画面没了还在录」的孤儿进程。
        if let Some(mut recorder) = state.record_process.take() {
            let _ = recorder.stop();
        }
    }
    Ok(process)
}

/// 结束会话时从会话表取出的进程句柄：设备序列号 + 镜像子进程。
type TakenProcess = (String, Box<dyn MirrorProcess>);

/// 结束**所有**进行中的会话并交还进程句柄（托盘「断开连接」、应用退出用）。
///
/// 返回各设备交出的进程；任何一台停止失败都不影响其它设备先被结束。
fn take_all_running_processes(store: &SessionStore) -> Result<Vec<TakenProcess>, AppError> {
    let mut map = store.lock()?;
    let mut taken = Vec::new();
    for (serial, state) in map.iter_mut() {
        if let Some(process) = state.process.take() {
            state.epoch = state.epoch.wrapping_add(1);
            state.session = MirrorSession::idle();
            // X10-92：显示会话结束，独立录制进程一并停掉。
            if let Some(mut recorder) = state.record_process.take() {
                let _ = recorder.stop();
            }
            taken.push((serial.clone(), process));
        }
    }
    Ok(taken)
}

/// 清理某台设备的空闲残条目：会话已回 `Idle`、无进程、亮屏账本也已还原时，
/// 把该设备从会话表里移除，保持表里只有「有意义」的记录。
fn prune_idle_entry(store: &SessionStore, serial: &str) {
    if let Ok(mut map) = store.0.lock() {
        let removable = map
            .get(serial)
            .is_some_and(|state| {
                state.process.is_none()
                    && state.session.phase == SessionPhase::Idle
                    && state.keep_awake_backup.is_none()
            });
        if removable {
            map.remove(serial);
        }
    }
}

/// 为「会话中应用新设置」原子地交出某台设备运行中的进程，并把该设备的会话
/// 就地标记为 `Connecting`。
///
/// 关键点：**不能先回到 `Idle` 再重新启动**。那会让界面在两次轮询之间读到“没有会话”，
/// 用户可能误以为镜像已经结束（而且“结束镜像”按钮会闪一下）。会话在整个重启过程中都
/// 应当停在「正在启动」。该设备没有运行中的进程时返回 `None`，会话状态不作改动。
fn begin_session_restart(
    store: &SessionStore,
    serial: String,
) -> Result<Option<Box<dyn MirrorProcess>>, AppError> {
    let mut map = store.lock()?;
    let Some(state) = map.get_mut(&serial) else {
        return Ok(None);
    };
    let Some(process) = state.process.take() else {
        return Ok(None);
    };
    state.epoch = state.epoch.wrapping_add(1);
    state.session = MirrorSession::connecting(serial);
    Ok(Some(process))
}

/// 设备在 ADB 视角下的可用性。用于把失败落到具体状态，而不是统一的“连接失败”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceLookup {
    Ready,
    Unauthorized,
    Offline,
    NotConnected,
    AdbUnavailable,
}

fn device_lookup(runtimes: &AppRuntimes, serial: &str) -> DeviceLookup {
    match runtimes.adb.list_devices() {
        Err(_) => DeviceLookup::AdbUnavailable,
        Ok(devices) => match devices.into_iter().find(|device| device.serial == serial) {
            Some(device) if device.state == DeviceState::Ready => DeviceLookup::Ready,
            Some(device) if device.state == DeviceState::Unauthorized => DeviceLookup::Unauthorized,
            Some(_) => DeviceLookup::Offline,
            None => DeviceLookup::NotConnected,
        },
    }
}

/// 把设备可用性翻译成面向用户的错误。会话启动与能力探测共用同一套判断，
/// 保证同一台设备在任何入口都得到一致的、可恢复的状态说明。
fn device_readiness_error(lookup: DeviceLookup) -> Option<AppError> {
    match lookup {
        DeviceLookup::Ready => None,
        DeviceLookup::Unauthorized => Some(AppError::new(
            "device_unauthorized",
            "手机尚未允许这台电脑进行调试。",
            "请解锁手机，在“允许 USB 调试吗？”提示中选择允许，然后重新检查。",
        )),
        DeviceLookup::Offline => Some(AppError::new(
            "device_offline",
            "手机当前处于离线状态。",
            "请重新插拔数据线或重新连接无线调试，保持手机解锁后重试。",
        )),
        DeviceLookup::NotConnected => Some(AppError::new(
            "device_not_connected",
            "找不到这台手机。",
            "请确认数据线或无线连接仍然有效，然后重新检查。",
        )),
        DeviceLookup::AdbUnavailable => Some(AppError::new(
            "adb_unavailable",
            "Android 调试服务暂时不可用。",
            "请拔下数据线后重新连接，或重新启动手机上的无线调试。",
        )),
    }
}

/// 轮询运行中的进程；进程退出后把结果写回**该设备**的会话状态。
fn spawn_session_monitor(store: SessionStore, epoch: u64, serial: String) {
    std::thread::spawn(move || loop {
        std::thread::sleep(MONITOR_INTERVAL);
        let mut exit_success = false;
        let mut was_recording = false;
        let finished = {
            let Ok(mut map) = store.0.lock() else {
                return;
            };
            let Some(state) = map.get_mut(&serial) else {
                return;
            };
            if state.epoch != epoch {
                return;
            }
            let Some(process) = state.process.as_mut() else {
                return;
            };
            match process.try_wait() {
                Some(success) => {
                    exit_success = success;
                    // 进程还挂在会话上（此处 process 一定是 Some）且持有录制路径
                    // ⇒ 录制随进程一起终止。无论退出是用户关窗（正常）还是手机掉线
                    // （异常），伴侣端都应收到「录制结束」——这是 M4-3 已知边界的补齐。
                    was_recording = state.record_path.is_some();
                    // 先取出输出再交还进程：退出原因就写在 scrcpy 的输出里（X10-79）。
                    let scrcpy_output = process.output_tail();
                    state.process = None;
                    state.session = resolve_process_exit(&serial, success, &scrcpy_output);
                    true
                }
                None => false,
            }
        };
        if finished {
            // 录制中的会话退出：给伴侣端补发「录制结束」（M4-3 会话生命周期钩子）。
            if was_recording {
                if let Some(app) = TRAY_APP.get() {
                    notify_companion_recording(app, false);
                }
            }
            // 异常退出（掉线/拔线）时明确告知前端（X10-59）：镜像窗口是独立进程，
            // 随之消失不等于应用崩溃，但旧版主窗口毫无表示，用户感知为「闪退」。
            // 若自动重连接管，紧随的 waiting 事件会覆盖这条提示。
            if !exit_success {
                if let Some(app) = TRAY_APP.get() {
                    let _ = app.emit(
                        "mirror-session-ended",
                        serde_json::json!({ "serial": serial, "unexpected": true }),
                    );
                }
            }
            // 会话意外退出后同步菜单文案（连接/断开、录制开关、置灰项）。
            // 测试环境没有 TRAY_APP，自动跳过。
            if let Some(app) = TRAY_APP.get() {
                refresh_tray_menu(app);
            }
            // 会话已结束：还原无线亮屏补偿（若有）。设备此时可能已离线导致还原
            // 失败——备份保留在内存与磁盘上，由下次启动/会话重试。
            if let Some(adb) = MONITOR_ADB.get() {
                disable_wireless_keep_awake(adb.as_ref(), &store, &serial);
            }
            prune_idle_entry(&store, &serial);
            // 输入源恢复（X10-39）：这是最后一台运行中的会话时，把输入法还给用户。
            if let Some(app) = TRAY_APP.get() {
                let running = store
                    .lock()
                    .map(|map| count_running_processes(&map))
                    .unwrap_or(usize::MAX);
                maybe_restore_host_input_source(app, running);
            }
            // 无线断线自动重连（X10-45）：只在「异常退出」（手机/网络掉线）时触发；
            // 用户主动关闭镜像窗口是正常退出，不打扰。
            if let Some(app) = TRAY_APP.get() {
                let enabled = app_settings_path(app)
                    .map(|path| load_app_settings(&path).auto_reconnect)
                    .unwrap_or(true);
                if should_auto_reconnect(exit_success, &serial, enabled) {
                    spawn_wireless_reconnect(app, store, epoch, &serial);
                }
            }
            return;
        }
    });
}

// ---------------------------------------------------------------------------
// 桌面模式「虚拟屏启动应用」落地核验（X10-80）
//
// 真机实证（Redmi M2104K10AC / MIUI 14 / Android 13，2026-10-03）：桌面模式 +
// start-app 后 scrcpy 窗口全白。逐层排查确认：编码、传输、渲染管线全部正常
// （系统设置能完整渲染在虚拟屏上），白屏是因为应用根本没落到虚拟屏——网易系
// 游戏的 SDK 跳板 Activity（ProtocolLauncher）在虚拟屏上跑完即被移出
// （WindowManager removeChildTask），真正的游戏 Activity 从未出现，MIUI
// SmartPower 随即把进程移入后台休眠。这与画质参数无关（2560/1920 都一样白）。
// ---------------------------------------------------------------------------

/// 启动后等待应用落地的时间。游戏/应用冷启动普遍要数秒，太短会把「还没起来」
/// 误判成「起不来」；10 秒是「用户等得已经可疑」与「误报」之间的折中。
const DESKTOP_APP_LANDING_DELAY: std::time::Duration = std::time::Duration::from_secs(10);

/// 从 scrcpy 输出解析虚拟屏 id。
///
/// 依据是服务端中继到客户端 stderr 的日志行：
/// `[server] INFO: Starting app "大话西游" [...] on display 84...`
fn parse_desktop_display_id(scrcpy_output: &str) -> Option<u32> {
    const MARKER: &str = "on display ";
    let start = scrcpy_output.rfind(MARKER)? + MARKER.len();
    let digits: String = scrcpy_output[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// 截取 `dumpsys activity activities` 输出里指定虚拟屏的分段。
fn extract_display_section(dumpsys: &str, display_id: u32) -> Option<&str> {
    let start_marker = format!("Display #{} ", display_id);
    let start = dumpsys.find(&start_marker)?;
    let rest = &dumpsys[start..];
    let end = rest[start_marker.len()..]
        .find("\nDisplay #")
        .map(|offset| offset + start_marker.len())
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// 判断应用是否已在（或至少出现在）指定虚拟屏上。
///
/// 判据：该屏分段里存在 `packageName=<package>` 的 ActivityRecord。真机核验：
/// 失败案例（大话西游）中虚拟屏分段里**一条**该应用的记录都没有；成功案例
/// （系统设置）则能看到完整记录。解析不出分段（格式变化）时调用方传 `None`
/// 退化为全局查找——宁可保守地不报错，也不对正常的会话喊狼来了。
fn desktop_app_landed(dumpsys: &str, display_id: Option<u32>, package: &str) -> bool {
    let marker = format!("packageName={}", package);
    match display_id.and_then(|id| extract_display_section(dumpsys, id)) {
        Some(section) => section.contains(&marker),
        None => dumpsys.contains(&marker),
    }
}

/// 落地核验线程：会话起来后延迟检查一次，应用没出现在虚拟屏就给用户明确提示。
///
/// 只做一次、不做重试：提示的目的是解释白屏并给出替代路径，不是监控。
/// 任何一步拿不到证据（进程已退出、adb 失败、输出解析不出）都安静放弃——
/// 这条核验只允许在「确有证据」时说话。
fn spawn_desktop_app_landing_check(
    store: SessionStore,
    epoch: u64,
    serial: String,
    package: String,
) {
    std::thread::spawn(move || {
        std::thread::sleep(DESKTOP_APP_LANDING_DELAY);
        let scrcpy_output = {
            let Ok(map) = store.0.lock() else {
                return;
            };
            let Some(state) = map.get(&serial) else {
                return;
            };
            if state.epoch != epoch {
                return;
            }
            let Some(process) = state.process.as_ref() else {
                return;
            };
            process.output_tail()
        };
        let display_id = parse_desktop_display_id(&scrcpy_output);
        // 输出里连 "Starting app" 都没有 ⇒ 应用启动意图根本没下发（老版本 scrcpy
        // 或参数没透传），此刻提示「应用没落地」反而是误导，放弃。
        if display_id.is_none() && !scrcpy_output.contains("Starting app") {
            return;
        }
        let Some(adb) = MONITOR_ADB.get() else {
            return;
        };
        let Ok(dumpsys) = adb.activity_dumpsys(&serial) else {
            return;
        };
        if desktop_app_landed(&dumpsys, display_id, &package) {
            return;
        }
        if let Some(app) = TRAY_APP.get() {
            let _ = app.emit(
                "desktop-app-missing",
                serde_json::json!({ "serial": serial, "package": package }),
            );
        }
    });
}

// ---------------------------------------------------------------------------
// 无线断线自动重连（X10-45）
//
// 已实证的无线痛点：手机熄屏十余分钟后整台从网络消失（ping 100% 丢包 +
// mDNS 无广播 + adb transport offline），用户感知为「镜像卡住/挂了」；等用户
// 点亮手机、设备回网后，还要手动重连一遍才能回到镜像。这里的实现目标：
// **用户点亮手机的那一刻，镜像自动回来。**
// ---------------------------------------------------------------------------

/// 重连探测间隔。太密会打扰 adb，太疏则「点亮手机 → 镜像回来」的延迟明显；
/// 5 秒是点亮手机后可接受的等待上限。
const RECONNECT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// 重连总预算。手机可能只是暂时锁屏掉网，也可能被拿走换网；15 分钟覆盖
/// 「熄屏下网 → 用户回身点亮」的典型间隔，之后如实放弃并告知。
const RECONNECT_BUDGET: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// 是否值得自动重连：仅**异常退出**（进程非 0 退出，对应手机/网络掉线）+
/// 用户没有关闭该功能。无线与 USB 一视同仁（X10-59，USB 真机实测教训）：
/// 拔线后 scrcpy 窗口随进程消失，旧版既不提示也不重连，用户感知为「闪退」；
/// 现在无线等手机回网、USB 等重新插线，恢复动作在等待期内自动完成。
/// 用户主动关闭镜像窗口是正常退出（0），不重连——那是「我不看了」，不是「断了」。
fn should_auto_reconnect(process_success: bool, _endpoint: &str, enabled: bool) -> bool {
    !process_success && enabled
}

/// 单次重连探测的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconnectProbe {
    /// 设备已在 adb 设备表且就绪——可以重建会话。
    Ready,
    /// 还没回来（不在表里 / offline / unauthorized）——继续等。
    NotReady,
    /// adb 本身不可用——继续等（adb 服务可能正在重启）。
    Unavailable,
}

fn classify_reconnect_probe(adb: &dyn AdbRuntime, endpoint: &str) -> ReconnectProbe {
    // 无线端点先补一次 connect（transport 可能半死但设备表里仍有条目）；
    // USB 串口没有 connect 语义（插回线设备表自然回归），跳过以免无意义报错。
    if is_wireless_endpoint(endpoint) {
        let _ = adb.connect(endpoint);
    }
    match adb.list_devices() {
        Err(_) => ReconnectProbe::Unavailable,
        Ok(devices) => match devices.into_iter().find(|device| device.serial == endpoint) {
            Some(device) if device.state == DeviceState::Ready => ReconnectProbe::Ready,
            _ => ReconnectProbe::NotReady,
        },
    }
}

/// 后台等待设备回网并自动重建镜像会话。
///
/// 退出条件（任一）：
/// * 用户自己动了这台设备（重新开会话 → epoch 变化 / 会话条目被移除）——交还控制权；
/// * 预算（15 分钟）用完——如实告知后放弃；
/// * 设备回网 → 用**原会话参数**重建。唯一例外：录制不再续录（原录像文件已
///   收尾，追加录制既不安全也不符合预期），重建的会话 `record = false`，
///   并在通知里如实说明。
fn spawn_wireless_reconnect(app: &AppHandle, sessions: SessionStore, epoch: u64, endpoint: &str) {
    let app = app.clone();
    let endpoint = endpoint.to_owned();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        // USB 与无线共用同一套等待语义，文案按通道区分（X10-59）。
        let usb = !is_wireless_endpoint(&endpoint);
        let (waiting_title, waiting_recovery) = if usb {
            (
                "数据线连接已断开，正在等待重新插入。",
                "重新插回数据线后会自动恢复镜像（最多等 15 分钟）；也可以手动重新连接。",
            )
        } else {
            (
                "无线连接已断开，正在等待手机重新上线。",
                "手机点亮并回到同一 Wi-Fi 后会自动恢复镜像（最多等 15 分钟）；也可以手动重新连接。",
            )
        };
        // 会话状态里的错误改写为「正在等待」，前端轮询即可见，无需新状态——
        // phase 保持 Failed（连接确实断了），恢复动作变成「等我们自动重连」。
        if let Ok(mut map) = sessions.0.lock() {
            if let Some(state) = map.get_mut(&endpoint) {
                state.session.error = Some(AppError::new(
                    "wireless_reconnect_waiting",
                    waiting_title,
                    waiting_recovery,
                ));
            }
        }
        let _ = app.emit(
            "wireless-reconnect-status",
            serde_json::json!({
                "status": "waiting",
                "endpoint": endpoint,
                "kind": if usb { "usb" } else { "wireless" },
            }),
        );
        loop {
            std::thread::sleep(RECONNECT_POLL_INTERVAL);
            // 先查用户意图：会话条目消失（忘记设备/清空）或已被用户重新占用，
            // 都说明用户在自己处理，自动重连立即让位。
            let session_options = {
                let Ok(map) = sessions.0.lock() else { return; };
                match map.get(&endpoint) {
                    Some(state) => {
                        if state.epoch != epoch || state.process.is_some() {
                            return;
                        }
                        state.options.clone()
                    }
                    None => return,
                }
            };
            if started.elapsed() >= RECONNECT_BUDGET {
                let _ = app.emit(
                    "wireless-reconnect-status",
                    serde_json::json!({ "status": "gave_up", "endpoint": endpoint }),
                );
                return;
            }
            let Some(runtimes) = app.try_state::<AppRuntimes>() else {
                return;
            };
            if !matches!(
                classify_reconnect_probe(runtimes.adb.as_ref(), &endpoint),
                ReconnectProbe::Ready
            ) {
                continue;
            }
            // 设备回来了：用原参数重建会话（录制不续录，见函数注释）。
            let mut options = session_options;
            options.record = false;
            let outcome =
                start_mirroring_with(&runtimes, &sessions, endpoint.clone(), options, None);
            let status = if outcome.is_ok() { "succeeded" } else { "gave_up" };
            let _ = app.emit(
                "wireless-reconnect-status",
                serde_json::json!({ "status": status, "endpoint": endpoint }),
            );
            if outcome.is_ok() {
                refresh_tray_menu(&app);
            }
            return;
        }
    });
}

/// 只有该设备空闲时才把“已配对”写进它的会话，避免覆盖正在运行的镜像会话。
fn mark_paired_if_idle(store: &SessionStore, endpoint: String) {
    if let Ok(mut map) = store.0.lock() {
        if !device_session_active(&map, &endpoint) {
            let state = session_entry_mut(&mut map, &endpoint);
            state.epoch = state.epoch.wrapping_add(1);
            state.process = None;
            state.session = MirrorSession::paired(endpoint);
        }
    }
}

/// 忘记某个端点时，若其会话正停留在“已配对”状态，则把该设备从会话表移除。
fn clear_paired_if_matches(store: &SessionStore, endpoint: &str) {
    if let Ok(mut map) = store.0.lock() {
        let matches = map.get(endpoint).is_some_and(|state| {
            state.session.phase == SessionPhase::Paired
                && state.session.serial.as_deref() == Some(endpoint)
        });
        if matches {
            map.remove(endpoint);
        }
    }
}

// ---------------------------------------------------------------------------
// 输入校验与可信设备列表
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TrustedWirelessDevice {
    endpoint: String,
}

/// 校验序列号或无线端点。
///
/// 该值只会作为**单个 argv** 传给 `adb`/`scrcpy`，不做任何 shell 拼接；但为了
/// 避免取值被下游程序当成选项解释，这里仍然拒绝空值、以 `-` 开头的值、超长值
/// 以及包含空白或控制字符的值。
fn validate_serial(serial: &str) -> Result<String, AppError> {
    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::new(
            "device_not_selected",
            "未选择可用设备。",
            "请重新检查连接后选择手机。",
        ));
    }

    let valid = serial.len() <= MAX_SERIAL_LEN
        && !serial.starts_with('-')
        && serial
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '_' | '-'));

    if valid {
        Ok(serial.to_owned())
    } else {
        Err(AppError::new(
            "device_serial_invalid",
            "设备标识无法识别。",
            "请在设备列表中重新选择这台手机；如果仍然失败，请重新插拔数据线或重连无线调试。",
        ))
    }
}

fn validate_endpoint(endpoint: &str) -> Result<String, AppError> {
    let invalid = || {
        AppError::new(
            "endpoint_invalid",
            "无线调试地址无法识别。",
            "请输入手机无线调试页面显示的 IP 地址和端口，例如 192.168.1.20:37123。",
        )
    };
    let endpoint = endpoint.trim();
    endpoint
        .parse::<SocketAddr>()
        .and_then(|address| {
            if address.port() == 0 {
                "invalid".parse::<SocketAddr>()
            } else {
                Ok(address)
            }
        })
        .map(|address| address.to_string())
        .map_err(|_| invalid())
}

fn validate_pairing_code(pairing_code: &str) -> Result<String, AppError> {
    let pairing_code = pairing_code.trim();
    if pairing_code.len() == 6 && pairing_code.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(pairing_code.to_owned())
    } else {
        Err(AppError::new(
            "pairing_code_invalid",
            "配对码格式不正确。",
            "请输入手机上显示的 6 位配对码。配对码不会被保存。",
        ))
    }
}

fn trusted_devices_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("trusted-wireless-devices.json"))
        .map_err(|_| {
            AppError::new(
                "trusted_list_unavailable",
                "无法访问本机设备列表。",
                "请检查本机文件权限，或重新安装 MirrorDock。",
            )
        })
}

fn load_trusted_devices(path: &Path) -> Result<Vec<TrustedWirelessDevice>, AppError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let unreadable = || {
        AppError::new(
            "trusted_list_unreadable",
            "本机设备列表无法读取。",
            "请在应用内逐个忘记并重新连接设备。",
        )
    };

    let bytes = fs::read(path).map_err(|_| unreadable())?;
    serde_json::from_slice(&bytes).map_err(|_| unreadable())
}

fn save_trusted_devices(path: &Path, devices: &[TrustedWirelessDevice]) -> Result<(), AppError> {
    let directory = path.parent().ok_or_else(|| {
        AppError::new(
            "trusted_list_write_failed",
            "无法创建本机设备列表。",
            "请检查本机文件权限。",
        )
    })?;
    fs::create_dir_all(directory).map_err(|_| {
        AppError::new(
            "trusted_list_write_failed",
            "无法创建本机设备列表目录。",
            "请检查本机文件权限。",
        )
    })?;
    let serialized = serde_json::to_vec_pretty(devices).map_err(|_| {
        AppError::new(
            "trusted_list_write_failed",
            "无法整理本机设备列表。",
            "请重试，或重新安装 MirrorDock。",
        )
    })?;
    fs::write(path, serialized).map_err(|_| {
        AppError::new(
            "trusted_list_write_failed",
            "无法保存本机设备列表。",
            "请检查本机文件权限。",
        )
    })
}

// ---------------------------------------------------------------------------
// 应用级设置（与镜像会话参数无关的本机偏好）
// ---------------------------------------------------------------------------

/// 本机应用偏好。与 SessionOptions 的区别：这些设置不进镜像子进程参数，
/// 只影响客户端自身的行为（窗口、图标、自动重连）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct AppSettings {
    /// 仅 macOS：隐藏 Dock 图标，只保留菜单栏图标与菜单。
    /// Windows/Linux 上恒为 false（写了也不生效，读出原样返回）。
    hide_dock_icon: bool,
    /// 无线镜像因链路断开**异常退出**时，自动等待设备回网并重建会话（X10-45）。
    /// 默认开启：无线掉线（尤其手机熄屏后整台下网）是无线场景第一痛点，
    /// 用户点亮手机的那一刻应当直接回到镜像，而不是再手动连一遍。
    auto_reconnect: bool,
    /// X10-95：录像保存目录（绝对路径）。`None` 用系统默认（视频目录/MirrorDock）。
    /// 用户可在设置里改成任意可写文件夹；非法/不可写路径在保存时拒绝，
    /// 录制时若目录已失效回退默认（不因为目录丢了就录不成）。
    #[serde(default)]
    recording_dir: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { hide_dock_icon: false, auto_reconnect: true, recording_dir: None }
    }
}

fn app_settings_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("app-settings.json"))
        .map_err(|_| {
            AppError::new(
                "settings_unavailable",
                "无法访问本机设置。",
                "请检查本机文件权限，或重新安装 MirrorDock。",
            )
        })
}

/// 读取应用设置。
///
/// 刻意**容错**：设置文件缺失或损坏一律回默认值。设置坏了不应该挡住应用启动——
/// 用户永远可以从界面里重新勾选，让文件被正常覆盖写回。
fn load_app_settings(path: &Path) -> AppSettings {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AppSettings>(&bytes).ok())
        .unwrap_or_default()
}

fn save_app_settings(path: &Path, settings: &AppSettings) -> Result<(), AppError> {
    let directory = path.parent().ok_or_else(|| {
        AppError::new(
            "settings_write_failed",
            "无法保存本机设置。",
            "请检查本机文件权限。",
        )
    })?;
    fs::create_dir_all(directory).map_err(|_| {
        AppError::new(
            "settings_write_failed",
            "无法创建本机设置目录。",
            "请检查本机文件权限。",
        )
    })?;
    let serialized = serde_json::to_vec_pretty(settings).map_err(|_| {
        AppError::new(
            "settings_write_failed",
            "无法整理本机设置。",
            "请重试，或重新安装 MirrorDock。",
        )
    })?;
    fs::write(path, serialized).map_err(|_| {
        AppError::new(
            "settings_write_failed",
            "无法保存本机设置。",
            "请检查本机文件权限。",
        )
    })
}

/// 按设置应用 macOS 的 Dock/菜单栏策略。
///
/// 只在 macOS 上有意义；其他平台什么都不做（不报错——设置在那些平台本来就
/// 不该出现勾选项）。运行中切换由 `set_app_settings` 触发。
fn apply_dock_icon_policy(app: &AppHandle, settings: &AppSettings) {
    #[cfg(target_os = "macos")]
    {
        let policy = if settings.hide_dock_icon {
            tauri::ActivationPolicy::Accessory
        } else {
            tauri::ActivationPolicy::Regular
        };
        if let Err(error) = app.set_activation_policy(policy) {
            eprintln!("set_activation_policy failed: {error}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, settings);
    }
}

/// 「回到主窗口」时是否需要把应用切回 Regular 以抢回前台。
///
/// 开启了隐藏 Dock 的应用必须保持 Accessory：Regular 会把 Dock 图标重新
/// 拉出来（真机反馈 2026-09-29：勾选后点菜单栏打开主窗口，图标又出现了）。
/// Accessory 模式下窗口照样可以 show/focus，只是不抢整个应用的前台。
#[cfg(target_os = "macos")]
fn show_focus_policy(hide_dock_icon: bool) -> Option<tauri::ActivationPolicy> {
    if hide_dock_icon {
        None
    } else {
        Some(tauri::ActivationPolicy::Regular)
    }
}

#[tauri::command]
fn get_app_settings(app: AppHandle) -> Result<AppSettings, AppError> {
    Ok(load_app_settings(&app_settings_path(&app)?))
}

#[tauri::command]
fn set_app_settings(app: AppHandle, settings: AppSettings) -> Result<AppSettings, AppError> {
    // macOS 之外不允许打开隐藏 Dock（前端也不展示该选项；这里再兜一层底，
    // 防止手改 JSON 后在 Windows/Linux 上出现「勾了但没有任何效果」的假开关）。
    // X10-95：录像目录校验——非空必须是绝对路径，且能创建/可写，否则拒存并如实报错。
    let recording_dir = match settings.recording_dir.as_deref() {
        None => None,
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None // 空串等价于「用默认」
            } else {
                let candidate = PathBuf::from(trimmed);
                if !candidate.is_absolute() {
                    return Err(AppError::new(
                        "recording_dir_invalid",
                        "录像保存位置无效。",
                        "请选择一个文件夹（需要完整路径）。",
                    ));
                }
                // 试着把目录建出来，建不出来就是不可写/无权限，拒存。
                if fs::create_dir_all(&candidate).is_err() {
                    return Err(AppError::new(
                        "recording_dir_unwritable",
                        "无法使用这个录像保存位置。",
                        "请换一个可写的文件夹。",
                    ));
                }
                Some(trimmed.to_owned())
            }
        }
    };
    let settings = AppSettings {
        hide_dock_icon: cfg!(target_os = "macos") && settings.hide_dock_icon,
        auto_reconnect: settings.auto_reconnect,
        recording_dir,
    };
    apply_dock_icon_policy(&app, &settings);
    save_app_settings(&app_settings_path(&app)?, &settings)?;
    Ok(settings)
}

// ---------------------------------------------------------------------------
// 菜单栏 / 托盘
//
// 关闭按钮 = 最小化到菜单栏（macOS）/托盘（Windows/Linux），不退出。
// 退出只能从菜单菜单触发，保证用户始终有明确的出口。
// ---------------------------------------------------------------------------

/// 显示并聚焦主窗口（macOS 上还要把应用带回前台，否则窗口在后台不拿焦点）。
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    #[cfg(target_os = "macos")]
    {
        // 以磁盘上的设置为准：未开启隐藏 Dock 才切回 Regular 抢前台；
        // 开启隐藏时保持 Accessory，绝不能把 Dock 图标重新拉出来。
        let hide = app_settings_path(app)
            .map(|path| load_app_settings(&path).hide_dock_icon)
            .unwrap_or(false);
        if let Some(policy) = show_focus_policy(hide) {
            let _ = app.set_activation_policy(policy);
        }
        if let Err(error) = app.show() {
            eprintln!("app.show failed: {error}");
        }
    }
}

/// 取主会话的序列号（仅会话进行中）。没有会话返回 None。
fn tray_active_serial(sessions: &SessionStore) -> Option<String> {
    let map = sessions.lock().ok()?;
    let serial = primary_session_serial(&map)?;
    let state = map.get(&serial)?;
    if !is_session_active(state) {
        return None;
    }
    state.session.serial.clone().or(Some(serial))
}

/// 点亮手机屏幕（供命令与菜单复用）。
///
/// 先识别显示电源策略再选动作：
/// * **DIM**（熄屏前变暗阶段被保活拦住，背光 5%、镜像黑帧）：纯唤醒键无效
///   （实测），必须走 `revive_dimmed_display`（BACK，失败再环）。
/// * **BRIGHT**：屏幕已正常点亮，什么都不做——多发 WAKEUP 无益。
/// * 其它/读不到：退回保守路径 `KEYCODE_WAKEUP`（熄屏状态下的标准唤醒）。
fn wake_screen_for_serial(runtimes: &AppRuntimes, serial: &str) -> Result<(), AppError> {
    if let Ok(dump) = runtimes.adb.display_state(serial) {
        match parse_display_policy(&dump) {
            DisplayPolicy::Dim => {
                return if revive_dimmed_display(runtimes.adb.as_ref(), serial) {
                    Ok(())
                } else {
                    Err(AppError::new(
                        "wake_failed",
                        "屏幕处于系统变暗状态，自动恢复没有成功。",
                        "请按一下手机的电源键点亮屏幕，再回到这里点「屏幕唤醒」。",
                    ))
                };
            }
            DisplayPolicy::Bright => return Ok(()),
            DisplayPolicy::Other => {}
        }
    }
    runtimes.adb.wake_screen(serial).map_err(|error| {
        adb_command_error(
            error,
            "wake_failed",
            "无法点亮手机屏幕。",
            "请确认数据线或无线连接仍然有效；也可以直接按一下手机的电源键。",
        )
    })
}

// ---------------------------------------------------------------------------
// 无线会话的亮屏补偿
//
// 为什么需要补偿（真机实测的根因链，逐条都有证据）：
//   1. scrcpy `--stay-awake` 的实现在 server 侧改两个系统设置
//      （`screen_off_timeout` 与 `stay_on_while_plugged_in`），但手册写明前提是
//      "when the device is plugged in"——无线连接下设备并未插电，前者的延长值
//      会被钥匙锁窗口覆盖，后者根本不满足生效条件。
//   2. 锁屏页由 WindowManager 强制覆盖熄屏超时（实测
//      `mUserActivityTimeoutOverrideFromWindowManager=10000`），所以唤醒后约
//      10~13 秒屏幕必灭（`mLastSleepReason=timeout`），与 `screen_off_timeout`
//      设成多大都无关。
//   3. 屏幕一灭，scrcpy 镜像的画面就是黑的，用户来不及在镜像里输入解锁密码；
//      熄屏久了无线链路还会整条掉线（实测 ping 与 mDNS 全无响应）。
//
// 对策（仍不绕过锁屏——锁还在，密码还得用户本人输，这里只负责让屏幕别灭）：
//   会话期间（仅无线、且用户开了「保持唤醒」）延长屏幕不灭：
//     ① 备份并写长 `screen_off_timeout`；
//     ② 备份并写 `stay_on_while_plugged_in = 7`（已插电时不因超时熄屏，含锁屏页）。
//   无线下设备未插电，② 本身不足以亮屏——这部分由 scrcpy 的 `--stay-awake` 负责
//   （已在启动 scrcpy 时传入），不再用 `dumpsys battery set usb 1` 伪造充电。
//   理由：伪造充电是设备全局 mock 状态，复位按原无线序列号执行；用户切换连接方式
//   （无线→USB）后旧序列号离线，复位静默失败，mock 残留会掩盖真实充电（见 BUG-充电掩盖）。
//   会话结束（正常停止 / 进程退出 / 应用退出）逐项还原 ②①。崩溃时靠落盘账本
//   在下次启动还原，绝不把任何临时状态留在用户手机上。
// ---------------------------------------------------------------------------

/// 补偿期间写入的熄屏时间（12 小时）。取大值的语义与 `--stay-awake` 一致：
/// 会话期间屏幕不再因超时熄灭；会话结束即还原，不靠这个值省电。
const WIRELESS_KEEP_AWAKE_TIMEOUT_MS: u64 = 12 * 60 * 60 * 1000;

/// 「充电时保持唤醒」的位掩码：AC(1) | USB(2) | WIRELESS(4)。
///
/// 与 scrcpy `--stay-awake` 写入的值同一语义（`BatteryManager.BATTERY_PLUGGED_*`）。
const STAY_ON_WHILE_PLUGGED_IN_ALL: u64 = 7;

/// 一次亮屏补偿的账本：哪台设备、被我们改动了什么、原值分别是多少。
///
/// 名字里的 `millis` 沿用最初只备份熄屏时间时的字段名（磁盘备份文件靠它保持兼容，
/// 老版本崩溃残留的文件仍能被解析并还原）。现在账本覆盖两把杠杆：
/// 熄屏时间、充电时保持唤醒。不再伪造充电（见 BUG-充电掩盖）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KeepAwakeBackup {
    serial: String,
    /// 原 `screen_off_timeout`（毫秒）。
    millis: u64,
    /// 原 `stay_on_while_plugged_in`；`None` 表示没读到，因而也没有改它。
        #[serde(default)]
        stay_on_while_plugged_in: Option<u64>,
    }

/// 判断序列号是否为无线传输（决定要不要做亮屏补偿）。
///
/// 无线序列号形如 `192.168.1.9:33739`（ip:port）或 mDNS 派生的
/// `adb-xxxx._adb-tls-connect._tcp`；USB 序列号是纯设备号，不含冒号与 `.tcp`。
fn is_wireless_serial(serial: &str) -> bool {
    serial.contains(':') || serial.contains(".tcp") || serial.starts_with("adb-")
}

/// 解析 `settings get` 返回的单个十进制数值（毫秒、位掩码通用）。
///
/// 合法输出是单独一行十进制整数；`null`（键不存在）或任何非数字都返回 `None`。
fn parse_settings_number(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok()
}

/// 无线会话开始（或重启）时拉满三把亮屏补偿杠杆。
///
/// 幂等约定：若账本已存在（重启场景），**只重复写延长值**，绝不把「已被我们改过的
/// 当前值」再当一次原值记账——那会让原值丢失。读原值失败（设备刚离线等）时静默跳过：
/// 少一次补偿只是回到「熄屏偏快」的旧体验，不值得为它打断会话启动。
fn enable_wireless_keep_awake(adb: &dyn AdbRuntime, store: &SessionStore, serial: &str) {
    // 门禁收在函数内部而不是只靠调用方：任何路径传进 USB 序列号都碰不到设备设置。
    if !is_wireless_serial(serial) {
        return;
    }
    let existing = store
        .lock()
        .ok()
        .and_then(|map| map.get(serial).and_then(|state| state.keep_awake_backup.clone()));
    if let Some(existing) = existing {
        // 重启：严格按账本记下的「当初改过哪几项」原样重写，读都不读，账本不动。
        let _ = adb.set_screen_off_timeout(serial, WIRELESS_KEEP_AWAKE_TIMEOUT_MS);
        if existing.stay_on_while_plugged_in.is_some() {
            let _ = adb.set_stay_on_while_plugged_in(serial, STAY_ON_WHILE_PLUGGED_IN_ALL);
        }
        return;
    }
    // 首轮：熄屏时间读不到就整体跳过——它是最保底的一把杠杆，连它都拿不到原值，
    // 说明设备状态不明，此时不宜再往系统里写任何东西。
    let Ok(Some(original_timeout)) = adb.screen_off_timeout(serial) else {
        return;
    };
    if adb
        .set_screen_off_timeout(serial, WIRELESS_KEEP_AWAKE_TIMEOUT_MS)
        .is_err()
    {
        return;
    }
    // 后两把杠杆逐级降级：读不到原值就不改它。宁可不生效，也绝不留下无法还原的改动。
    let original_stay_on = adb.stay_on_while_plugged_in(serial).ok().flatten();
    let stay_on_armed = original_stay_on.is_some()
        && adb
            .set_stay_on_while_plugged_in(serial, STAY_ON_WHILE_PLUGGED_IN_ALL)
            .is_ok();
    let backup = KeepAwakeBackup {
        serial: serial.to_owned(),
        millis: original_timeout,
        stay_on_while_plugged_in: if stay_on_armed { original_stay_on } else { None },
    };
    if let Ok(mut map) = store.0.lock() {
        session_entry_mut(&mut map, serial).keep_awake_backup = Some(backup.clone());
    }
    // 落盘兜底：应用在会话中崩溃/被强杀时内存账本随之丢失，靠这个文件在下次启动
    // 还原——尤其是别把「假充电」这种临时状态留在用户手机上。写入失败不阻断会话。
    if let Some(app) = TRAY_APP.get() {
        if let Ok(path) = keep_awake_backup_path(app) {
            let _ = save_keep_awake_ledger(&path, &load_keep_awake_ledger(&path), &backup);
        }
    }
}

/// 逐项撤销一次亮屏补偿。返回是否**全部**还原成功。
///
/// 撤销顺序与施加顺序相反，且「假充电」排在最前：它是唯一会改变用户可见状态的一项
/// （状态栏会显示充电中），只要设备可达就该第一时间撤掉。三项都尝试，不短路。
fn restore_keep_awake(adb: &dyn AdbRuntime, backup: &KeepAwakeBackup) -> bool {
    let mut restored = true;
    if let Some(original) = backup.stay_on_while_plugged_in {
        restored &= adb
            .set_stay_on_while_plugged_in(&backup.serial, original)
            .is_ok();
    }
    restored & adb
        .set_screen_off_timeout(&backup.serial, backup.millis)
        .is_ok()
}

/// 会话结束（正常停止、进程退出、应用退出）时还原**该设备**的全部亮屏补偿。
///
/// 返回是否已彻底还原。设备离线时还原会失败：账本**保留**在内存与磁盘上，
/// 交给下次会话开始或下次应用启动重试，绝不悄悄丢弃原值、更不会把「假充电」留下。
fn disable_wireless_keep_awake(adb: &dyn AdbRuntime, store: &SessionStore, serial: &str) -> bool {
    let backup = store
        .lock()
        .ok()
        .and_then(|mut map| map.get_mut(serial).and_then(|state| state.keep_awake_backup.take()));
    let Some(backup) = backup else {
        return true;
    };
    if restore_keep_awake(adb, &backup) {
        if let Some(app) = TRAY_APP.get() {
            if let Ok(path) = keep_awake_backup_path(app) {
                let _ = remove_keep_awake_ledger_entry(&path, serial);
            }
        }
        true
    } else {
        // 还原失败：把账本放回（若期间没有新的补偿写入），等待下次机会。
        if let Ok(mut map) = store.0.lock() {
            if let Some(state) = map.get_mut(serial) {
                if state.keep_awake_backup.is_none() {
                    state.keep_awake_backup = Some(backup);
                }
            }
        }
        false
    }
}

/// 应用退出时还原**所有**设备的亮屏补偿（X10-27：多台设备可能各有账本）。
fn disable_all_wireless_keep_awake(adb: &dyn AdbRuntime, store: &SessionStore) {
    let serials: Vec<String> = store
        .lock()
        .map(|map| {
            map.iter()
                .filter(|(_, state)| state.keep_awake_backup.is_some())
                .map(|(serial, _)| serial.clone())
                .collect()
        })
        .unwrap_or_default();
    for serial in serials {
        disable_wireless_keep_awake(adb, store, &serial);
    }
}

/// 应用启动时检查崩溃遗留的账本文件并尝试还原（best-effort）。
///
/// 设备未连接时还原会失败，文件保留，下次启动再试。这条路径最重要的作用是兜住
/// 「应用在会话中崩溃，手机被留在假充电状态」这种情况。X10-27 后账本里可能有多台
/// 设备的记录，逐台还原。
fn restore_persisted_keep_awake(adb: &dyn AdbRuntime, path: &Path) {
    let ledger = load_keep_awake_ledger(path);
    if ledger.is_empty() {
        return;
    }
    let mut remaining: Vec<KeepAwakeBackup> = Vec::new();
    for backup in ledger {
        if !restore_keep_awake(adb, &backup) {
            remaining.push(backup);
        }
    }
    if remaining.is_empty() {
        let _ = fs::remove_file(path);
    } else {
        let _ = write_keep_awake_ledger(path, &remaining);
    }
}

fn keep_awake_backup_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("screen-timeout-backup.json"))
        .map_err(|_| {
            AppError::new(
                "settings_unavailable",
                "无法访问本机设置。",
                "请检查本机文件权限，或重新安装 MirrorDock。",
            )
        })
}

/// 读取账本文件（X10-27 起为 `Vec<KeepAwakeBackup>`，支持多台设备并存）。
///
/// 兼容旧版本的单条对象格式：老文件解析失败时回退为单条——老版本崩溃残留仍可
/// 被识别并还原，不会因为格式升级而把「假充电」留在用户手机上。
fn load_keep_awake_ledger(path: &Path) -> Vec<KeepAwakeBackup> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    if let Ok(ledger) = serde_json::from_slice::<Vec<KeepAwakeBackup>>(&bytes) {
        return ledger;
    }
    serde_json::from_slice::<KeepAwakeBackup>(&bytes).map(|single| vec![single]).unwrap_or_default()
}

/// 把合并后的账本（既有记录 + 本次更新）写回文件。
fn save_keep_awake_ledger(
    path: &Path,
    existing: &[KeepAwakeBackup],
    backup: &KeepAwakeBackup,
) -> Result<(), AppError> {
    let mut ledger: Vec<KeepAwakeBackup> = existing
        .iter()
        .filter(|entry| entry.serial != backup.serial)
        .cloned()
        .collect();
    ledger.push(backup.clone());
    write_keep_awake_ledger(path, &ledger)
}

/// 从账本文件里移除某台设备的记录；账本清空时删除文件本身。
fn remove_keep_awake_ledger_entry(path: &Path, serial: &str) -> Result<(), AppError> {
    let ledger: Vec<KeepAwakeBackup> = load_keep_awake_ledger(path)
        .into_iter()
        .filter(|entry| entry.serial != serial)
        .collect();
    if ledger.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    write_keep_awake_ledger(path, &ledger)
}

fn write_keep_awake_ledger(path: &Path, ledger: &[KeepAwakeBackup]) -> Result<(), AppError> {
    let directory = path.parent().ok_or_else(|| {
        AppError::new("settings_write_failed", "无法保存本机设置。", "请检查本机文件权限。")
    })?;
    fs::create_dir_all(directory).map_err(|_| {
        AppError::new("settings_write_failed", "无法保存本机设置。", "请检查本机文件权限。")
    })?;
    let serialized = serde_json::to_vec_pretty(ledger).map_err(|_| {
        AppError::new("settings_write_failed", "无法保存本机设置。", "请检查本机文件权限。")
    })?;
    fs::write(path, serialized).map_err(|_| {
        AppError::new("settings_write_failed", "无法保存本机设置。", "请检查本机文件权限。")
    })
}

// ---------------------------------------------------------------------------
// 宿主输入源自动切换（X10-39，X10-41 修订）
//
// UHID 物理键盘语义下，macOS 上只要激活的不是 ABC 布局——第三方输入法
// （微信输入法、搜狗等）或系统拼音等输入法模式——镜像打字都收不到/打不出
// 正确字符（X10-38/X10-41 真机定案：只有 ABC 可用）。会话期间临时切到 ABC
// 布局，结束（或退出/崩溃残留）后恢复。输入源是宿主全局资源：多台并发设备
// 共享同一次切换与同一次恢复。
// ---------------------------------------------------------------------------

/// 本次运行期间已托管的输入源 bundle id（用户原来的输入法）。
static HOST_INPUT_SOURCE_BACKUP: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn host_input_source_backup_cell() -> &'static Mutex<Option<String>> {
    HOST_INPUT_SOURCE_BACKUP.get_or_init(|| Mutex::new(None))
}

/// 崩溃恢复账本：切换发生时落盘，应用下次启动时照此恢复并删除。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct InputSourceBackupEntry {
    bundle_id: String,
}

fn input_source_backup_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("input-source-backup.json"))
        .map_err(|_| {
            AppError::new(
                "settings_unavailable",
                "无法访问本机设置。",
                "请检查本机文件权限，或重新安装 MirrorDock。",
            )
        })
}

/// 镜像会话开始时调用：当前输入源不是 ABC 布局时，临时切到 ABC（X10-41）。
///
/// 幂等：已托管（多台并发/会话重启）时是空操作。切换只在「确实切走了」时落盘
/// 与通知，任何失败都静默跳过——输入法问题绝不能阻断镜像本身。
/// TIS 调用全部走主线程（2026-10-01 崩溃实锤：macOS 15 后台线程调 TIS 直接
/// SIGILL）；因此「读当前源」与「切换」都放在 backup 锁之外，避免后台线程
/// 持锁等主线程、而主线程（托盘退出路径）也要拿这把锁形成等待环。
fn maybe_switch_host_input_source(app: &AppHandle) {
    let current = input_source::current_bundle_id_on_main(app);
    let should_switch = {
        let Ok(backup) = host_input_source_backup_cell().lock() else {
            return;
        };
        input_source::should_attempt_switch(current.as_deref(), backup.is_some())
    };
    if !should_switch {
        return;
    }
    if !input_source::switch_to_system_ascii_on_main(app) {
        return;
    }
    let current = match current {
        Some(id) => id,
        None => return,
    };
    // 重新拿锁写状态：并发启动的另一个会话可能已抢先完成切换并记账；那样我们
    // 只是把 ABC 又选中了一次（幂等），不再重复落盘与通知。
    let Ok(mut backup) = host_input_source_backup_cell().lock() else {
        return;
    };
    if backup.is_some() {
        return;
    }
    // 账本先落盘再改内存：进程在两者之间被杀，下次启动仍能恢复。
    if let Ok(path) = input_source_backup_path(app) {
        if let Ok(payload) = serde_json::to_vec(&InputSourceBackupEntry { bundle_id: current.clone() }) {
            let _ = fs::write(path, payload);
        }
    }
    *backup = Some(current);
    // 通知前端展示一次性提示（会话结束后自动恢复，用户不该被静默换输入法）。
    let _ = app.emit("host-input-source-switched", ());
}

/// 镜像会话结束时调用：没有还在运行的会话时，把输入源还给用户。
fn maybe_restore_host_input_source(app: &AppHandle, running_processes: usize) {
    let taken = host_input_source_backup_cell()
        .lock()
        .ok()
        .and_then(|mut backup| backup.take());
    let Some(bundle_id) = taken else {
        return;
    };
    if !input_source::should_attempt_restore(running_processes, true) {
        // 还有会话在跑：把备份放回去，等最后一台结束再恢复。
        if let Ok(mut backup) = host_input_source_backup_cell().lock() {
            *backup = Some(bundle_id);
        }
        return;
    }
    // 选不回（比如输入法被卸载）也照样清理：留在英文布局无害，用户手动可切。
    // TIS 必须在主线程调（2026-10-01 崩溃实锤）；此时未持有 backup 锁，不存在
    // 与主线程的锁等待环。
    let _ = input_source::switch_to_bundle_on_main(app, &bundle_id);
    if let Ok(path) = input_source_backup_path(app) {
        let _ = fs::remove_file(path);
    }
}

/// 当前仍在运行的镜像会话数量（持有进程的设备）。
fn count_running_processes(map: &BTreeMap<String, SessionState>) -> usize {
    map.values().filter(|state| state.process.is_some()).count()
}

/// 应用启动时调用：恢复上次崩溃残留的输入源账本（读取 → 恢复 → 删除）。
/// setup 回调在主线程执行，但恢复动作仍统一走主线程包装器，线程纪律只有一条。
fn restore_persisted_input_source(app: &AppHandle, path: &Path) {
    let Ok(bytes) = fs::read(path) else {
        return;
    };
    let _ = fs::remove_file(path);
    let Ok(entry) = serde_json::from_slice::<InputSourceBackupEntry>(&bytes) else {
        return;
    };
    let _ = input_source::switch_to_bundle_on_main(app, &entry.bundle_id);
}

// ---------------------------------------------------------------------------
// 变暗（DIM）守护
//
// 第二段根因链（2026-09-29 真机实测，Redmi M2104K10AC / Android 13 / MIUI）：
// stay_on_while_plugged_in 只拦「熄屏（OFF）」，拦不住熄屏前的「变暗（DIM）」。
// 锁屏静置约 3 分钟后电源策略进入 DIM（`mPowerRequest=policy=DIM`，背光压到
// 5%，渲染层对虚拟显示器输出黑帧）——物理上像熄屏，镜像上是黑屏，但
// `mWakefulness` 仍是 Awake。此时：
//   * `KEYCODE_WAKEUP` 无效（设备已经是 Awake，唤醒语义不命中）——这就是用户
//     点「屏幕唤醒」后镜像依旧黑屏的原因；
//   * `KEYCODE_BACK` 可以把电源策略拉回 BRIGHT（锁屏上无副作用，实测有效）；
//   * BACK 无效时退化为 `KEYCODE_SLEEP`→`KEYCODE_WAKEUP` 强制走一次熄屏-点亮
//     环（屏幕会闪黑一下，作为兜底）。
// ---------------------------------------------------------------------------

/// 显示电源策略（`dumpsys display` 的 `mPowerRequest=policy=` 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayPolicy {
    /// 正常亮度。
    Bright,
    /// 熄屏前的变暗阶段：背光压低、内容层可能对虚拟显示器关闭。
    Dim,
    /// OFF / DOZE / 读不到等其它情况，一律按「不是 DIM」处理。
    Other,
}

/// 从 `dumpsys display` 输出解析电源策略。找不到字段按 Other 处理（保守：
/// 不确定时宁可不注入按键）。
fn parse_display_policy(dump: &str) -> DisplayPolicy {
    const NEEDLE: &str = "mPowerRequest=policy=";
    for line in dump.lines() {
        if let Some(index) = line.find(NEEDLE) {
            let rest = &line[index + NEEDLE.len()..];
            return if rest.starts_with("BRIGHT") {
                DisplayPolicy::Bright
            } else if rest.starts_with("DIM") {
                DisplayPolicy::Dim
            } else {
                DisplayPolicy::Other
            };
        }
    }
    DisplayPolicy::Other
}

/// 把处于 DIM 阶段的屏幕拉回正常亮度。返回是否确认恢复。
///
/// 顺序：先注入 `KEYCODE_BACK`（无闪黑、锁屏上无副作用），复查仍 DIM 再走
/// 熄屏-点亮环兜底。任何 adb 失败都如实返回 false，由调用方决定下一步。
fn revive_dimmed_display(adb: &dyn AdbRuntime, serial: &str) -> bool {
    if adb.press_key(serial, "KEYCODE_BACK").is_err() {
        return force_display_cycle(adb, serial);
    }
    std::thread::sleep(std::time::Duration::from_millis(600));
    match adb.display_state(serial) {
        Ok(dump) if parse_display_policy(&dump) != DisplayPolicy::Dim => true,
        _ => force_display_cycle(adb, serial),
    }
}

/// 熄屏-点亮环：强制走一次 OFF 再唤醒，把电源策略从 DIM 拉回 BRIGHT。
/// 屏幕会闪黑一下，只作为 BACK 无效时的兜底。
fn force_display_cycle(adb: &dyn AdbRuntime, serial: &str) -> bool {
    if adb.press_key(serial, "KEYCODE_SLEEP").is_err() {
        return false;
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    if adb.press_key(serial, "KEYCODE_WAKEUP").is_err() {
        return false;
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    match adb.display_state(serial) {
        Ok(dump) => parse_display_policy(&dump) != DisplayPolicy::Dim,
        Err(_) => false,
    }
}

/// 守护线程的单次检查。返回 true 表示继续守护，false 表示应当退出。
///
/// 注入按键有界面副作用（解锁状态下 BACK 会后退界面），所以**只在设备处于
/// 锁屏时**才允许注入——锁屏上 BACK 无副作用，而用户需要输密码的场景恰恰
/// 都在锁屏上。设备未锁屏时的 DIM 是正常省电，一碰即恢复，不做处理。
fn keep_awake_guard_tick(adb: &dyn AdbRuntime, store: &SessionStore, epoch: u64, serial: &str) -> bool {
    let alive = {
        let Ok(map) = store.0.lock() else {
            return false;
        };
        map.get(serial).is_some_and(|state| {
            state.epoch == epoch && state.process.is_some() && state.options.keep_awake
        })
    };
    if !alive {
        return false;
    }
    let Ok(policy_dump) = adb.display_state(serial) else {
        return true; // 设备瞬时离线等，下一轮再试
    };
    if parse_display_policy(&policy_dump) != DisplayPolicy::Dim {
        return true;
    }
    let Ok(window_dump) = adb.window_policy(serial) else {
        return true;
    };
    let (keyguard, _) = parse_keyguard_state(&window_dump);
    if keyguard != KeyguardState::Locked {
        return true; // 未锁屏时不注入，避免后退用户界面
    }
    revive_dimmed_display(adb, serial);
    true
}

/// 会话期间守护锁屏不被 DIM 黑掉（keep_awake 开启时启动）。
///
/// 退出条件：epoch 变化（会话重启/结束）、会话进程消失、keep_awake 被关闭、
/// store 锁不可用。任何 adb 瞬时失败都只是跳过本轮，不打断守护。
fn spawn_keep_awake_guard(store: SessionStore, epoch: u64, serial: String) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(20));
        let Some(adb) = MONITOR_ADB.get() else {
            return;
        };
        if !keep_awake_guard_tick(adb.as_ref(), &store, epoch, &serial) {
            return;
        }
    });
}

/// 菜单栏模板图标（icons/tray.png，44x44 单色 + alpha）。
/// 与应用图标同一设计语言：环形 + 缺口切片 + 中心圆点。
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/tray.png");

/// 监视线程（无 AppHandle 入参）在会话意外退出后刷新菜单用。
/// setup 阶段设置一次；测试环境不设置，刷新自动降级为空操作。
static TRAY_APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

/// 监视线程在会话意外退出（进程崩溃/设备离线）后还原无线亮屏补偿用。
/// 同样 setup 阶段设置一次；测试环境不设置，还原自动跳过（备份留在内存中）。
static MONITOR_ADB: std::sync::OnceLock<Box<dyn AdbRuntime>> = std::sync::OnceLock::new();

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::tray::TrayIconBuilder;

    let handle = app.handle().clone();
    let menu = build_tray_menu(&handle)?;

    let _ = TRAY_APP.set(app.handle().clone());

    // 菜单栏图标：单色模板图（黑色形状 + alpha），macOS 按菜单栏深浅自动反色
    // （深色菜单栏渲染为白色，与系统自带图标一致）。非 macOS 平台仍用彩色应用图标。
    let tray_icon = tauri::image::Image::from_bytes(TRAY_ICON_PNG)
        .expect("embedded tray icon is a valid PNG")
        .to_owned();
    let mut builder = TrayIconBuilder::with_id("mirrordock-tray")
        .icon(tray_icon)
        .tooltip("MirrorDock")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| handle_tray_event(app, event.id().as_ref()));
    #[cfg(target_os = "macos")]
    {
        builder = builder.icon_as_template(true);
    }
    builder.build(app)?;

    // 初始状态与空闲会话对齐（屏幕唤醒/截图在无会话时置灰）。
    refresh_tray_menu(app.handle());
    Ok(())
}

/// 按当前会话状态重建整条托盘菜单（X10-27 里程碑 2：托盘逐会话细化）。
///
/// 动态项：每个真正运行中的镜像会话各占一条「结束 <设备名> 的镜像」，
/// 设备名优先取最近使用记录里的友好名称，查不到时如实显示序列号。
/// 其余项沿用全局语义：连接/断开、录制开关作用于主会话，唤醒/截图作用于主会话。
/// 文案只是提示，**动作以点击时的真实会话状态为准**：即使菜单文案与状态短暂
/// 不一致（状态跃迁与菜单刷新之间有窗口期），也不会执行错误方向的动作。
fn build_tray_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};

    let sessions: State<SessionStore> = app.state();
    let (active, recording, running) = match sessions.lock() {
        Ok(map) => {
            let active = any_session_active(&map);
            // X10-92 双通道：录制状态由独立录制进程 record_process 判定。
            // 旧判定 `options.record` 是单通道重启式遗留——双通道下显示进程
            // 永远不带 record，读它会导致菜单永远停在「开始屏幕录制」（X10-95 修复）。
            let recording = map.values().any(|state| state.record_process.is_some());
            // 逐会话条目：只列真正持有镜像进程的设备（历史账本条目不算）。
            let running: Vec<String> = map
                .iter()
                .filter(|(_, state)| state.process.is_some())
                .map(|(serial, _)| serial.clone())
                .collect();
            (active, recording, running)
        }
        // 锁不可用时按「无会话」渲染：动作侧有状态校验兜底，不会误动作。
        Err(_) => (false, false, Vec::new()),
    };

    // 设备友好名称：最近使用记录按序列号查；查不到用序列号本身，不猜。
    let labels = load_recent_devices(
        &recent_devices_path(app).unwrap_or_else(|_| std::path::PathBuf::from("/dev/null")),
    )
    .unwrap_or_default();

    let open = MenuItemBuilder::with_id("tray-open", "打开 MirrorDock").build(app)?;
    let connect = MenuItemBuilder::with_id(
        "tray-connect",
        if active { "断开连接" } else { "连接设备" },
    )
    .build(app)?;
    let record = MenuItemBuilder::with_id(
        "tray-record",
        if recording {
            "结束屏幕录制"
        } else {
            "开始屏幕录制"
        },
    )
    .build(app)?;

    let mut builder = MenuBuilder::new(app).item(&open).item(&PredefinedMenuItem::separator(app)?).item(&connect);
    if !running.is_empty() {
        // 每个运行中的会话一条「结束镜像」，多设备时逐台可控（X10-27 里程碑 2）。
        for serial in &running {
            let label = labels
                .iter()
                .find(|device| &device.serial == serial)
                .map(|device| device.label.clone())
                .unwrap_or_else(|| serial.clone());
            let item = MenuItemBuilder::with_id(format!("tray-stop-{serial}"), format!("结束 {label} 的镜像"))
                .build(app)?;
            builder = builder.item(&item);
        }
    }
    let mut builder = builder
        .item(&record)
        .item(&PredefinedMenuItem::separator(app)?);
    let wake = MenuItemBuilder::with_id("tray-wake", "屏幕唤醒")
        .enabled(active)
        .build(app)?;
    let shot = MenuItemBuilder::with_id("tray-screenshot", "手机截图")
        .enabled(active)
        .build(app)?;
    let quit = MenuItemBuilder::with_id("tray-quit", "退出 MirrorDock").build(app)?;
    builder = builder
        .item(&wake)
        .item(&shot)
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&quit);
    builder.build()
}

/// 把菜单文案与可用性同步到会话状态（X10-27 里程碑 2：整条菜单动态重建）。
///
/// 所有会话状态跃迁点（启动、停止、重启、监视线程发现进程退出）之后都应调用。
/// 刷新失败静默忽略：菜单文案只是提示，真正的动作以点击时的状态校验为准。
fn refresh_tray_menu(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("mirrordock-tray") else {
        return;
    };
    // 菜单构建失败保持旧菜单可用，动作侧有状态校验兜底。
    if let Ok(menu) = build_tray_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn handle_tray_event(app: &AppHandle, id: &str) {
    match id {
        "tray-open" => show_main_window(app),
        "tray-quit" => {
            // 退出前把临时切换的宿主输入源还给用户（X10-39）。
            maybe_restore_host_input_source(app, 0);
            app.exit(0)
        }
        "tray-wake" => tray_wake(app),
        "tray-screenshot" => tray_screenshot(app),
        "tray-connect" => tray_connect_toggle(app),
        "tray-record" => tray_record_toggle(app),
        other => {
            if let Some(serial) = other.strip_prefix("tray-stop-") {
                tray_stop_device(app, serial);
            }
        }
    }
}

/// 菜单里的「结束 <设备> 的镜像」（X10-27 里程碑 2：多会话逐台可控）。
///
/// 逻辑与 `stop_mirroring` 命令的单设备路径一致：录制中的会话先快照，
/// 停止成功后给伴侣端补「录制结束」、还原无线亮屏补偿并清理账本条目。
fn tray_stop_device(app: &AppHandle, serial: &str) {
    let sessions: State<SessionStore> = app.state();
    let runtimes: State<AppRuntimes> = app.state();
    let log: State<DiagnosticsLog> = app.state();
    let serial = serial.to_owned();
    // 提示用最近使用记录里的友好名称；查不到如实显示序列号。
    let label = recent_devices_path(app)
        .ok()
        .and_then(|path| load_recent_devices(&path).ok())
        .and_then(|labels| {
            labels
                .into_iter()
                .find(|device| device.serial == serial)
                .map(|device| device.label)
        })
        .unwrap_or_else(|| serial.clone());
    let was_recording = any_session_recording(&sessions);
    let result = stop_mirroring_with(&sessions, Some(serial.clone()));
    log.record_outcome("mirror_stop", result.as_ref().err(), &[&serial]);
    match result {
        Ok(()) => {
            if was_recording {
                notify_companion_recording(app, false);
            }
            disable_wireless_keep_awake(runtimes.adb.as_ref(), &sessions, &serial);
            prune_idle_entry(&sessions, &serial);
            let running = sessions
                .lock()
                .map(|map| count_running_processes(&map))
                .unwrap_or(usize::MAX);
            maybe_restore_host_input_source(app, running);
            refresh_tray_menu(app);
            tray_notify_info(app, &format!("已结束 {label} 的镜像会话。"));
        }
        Err(error) => tray_notify_error(app, &error.message),
    }
}

/// 菜单里的「连接设备 / 断开连接」。
///
/// 连接目标：优先最近设备里当前就绪的那台（符合「连我刚才那台」的直觉），
/// 其次任意一台就绪设备；一台就绪的都没有时如实提示。启动参数 = 后端默认值
/// 叠加这台设备的桌面模式偏好（X10-90），与主窗口「开始镜像」走同一条路径。
fn tray_connect_toggle(app: &AppHandle) {
    let sessions: State<SessionStore> = app.state();
    let runtimes: State<AppRuntimes> = app.state();
    let log: State<DiagnosticsLog> = app.state();
    let active = sessions
        .lock()
        .map(|map| any_session_active(&map))
        .unwrap_or(true);
    if active {
        // 多会话语义（X10-27）：托盘「断开连接」结束**所有**设备的镜像会话。
        // 停止前快照：哪些会话真正正在录制（M4-3）——托盘结束时同样要给伴侣端
        // 补「录制结束」，否则录制经托盘终止时伴侣永远等不到跃迁通知。
        let recording_serials: Vec<String> = sessions
            .lock()
            .map(|map| {
                map.iter()
                    .filter(|(_, state)| state.record_path.is_some() && state.process.is_some())
                    .map(|(serial, _)| serial.clone())
                    .collect()
            })
            .unwrap_or_default();
        let taken = take_all_running_processes(&sessions);
        match taken {
            Ok(processes) if processes.is_empty() => {
                tray_notify_error(app, "当前没有正在运行的镜像会话。");
            }
            Ok(processes) => {
                let mut stopped_recording = false;
                for (serial, mut process) in processes {
                    let result = process.stop().map_err(|_| {
                        AppError::new(
                            "mirror_stop_failed",
                            "无法结束镜像窗口。",
                            "请手动关闭镜像窗口后重试。",
                        )
                    });
                    log.record_outcome("mirror_stop", result.as_ref().err(), &[&serial]);
                    if result.is_ok() {
                        // 只有真正停止成功才通知：停止失败的会话录制并未终止。
                        stopped_recording |= recording_serials.contains(&serial);
                        disable_wireless_keep_awake(runtimes.adb.as_ref(), &sessions, &serial);
                    }
                    prune_idle_entry(&sessions, &serial);
                }
                // 托盘断开是全部会话一起结束：这是恢复宿主输入法的时机（X10-39）。
                let running = sessions
                    .lock()
                    .map(|map| count_running_processes(&map))
                    .unwrap_or(usize::MAX);
                maybe_restore_host_input_source(app, running);
                if stopped_recording {
                    notify_companion_recording(app, false);
                }
            }
            Err(error) => tray_notify_error(app, &error.message),
        }
        refresh_tray_menu(app);
        return;
    }
    let recent = recent_devices_path(app)
        .and_then(|path| load_recent_devices(&path))
        .unwrap_or_default();
    let devices = runtimes.adb.list_devices().unwrap_or_default();
    let Some((serial, label)) = pick_tray_target(&recent, &devices) else {
        tray_notify_info(
            app,
            "没有检测到可连接的手机。请先用数据线连接并允许调试，或在主窗口中使用无线连接。",
        );
        return;
    };
    let result = start_mirroring_with(
        &runtimes,
        &sessions,
        serial.clone(),
        tray_session_options(app, &devices, &serial),
        None,
    );
    log.record_outcome("mirror_start", result.as_ref().err(), &[&serial, &label]);
    if result.is_ok() {
        maybe_switch_host_input_source(app);
    }
    if let Err(error) = result {
        tray_notify_error(app, &error.message);
    }
    refresh_tray_menu(app);
}

/// 托盘「连接设备」的启动参数（X10-90）：在后端默认值之上叠加这台设备的
/// 桌面模式偏好——与主窗口「开始镜像」同一口径（`composeOptionsWithDesktop`）。
/// 只覆盖桌面模式相关字段，其余（画质/帧率/快捷键…）托盘路径本来就没有入口，
/// 沿用后端默认。开桌面模式时强制关摄像头源（与前端互斥逻辑一致）。
fn tray_session_options(
    app: &AppHandle,
    devices: &[AdbDevice],
    serial: &str,
) -> SessionOptions {
    let mut options = SessionOptions::default();
    if let Some(pref) = desktop_pref_for_serial(app, devices, serial) {
        options.desktop_mode = pref.desktop_mode;
        options.desktop_app = if pref.desktop_mode {
            pref.desktop_app
        } else {
            None
        };
        if pref.desktop_mode {
            options.camera_source = false;
        }
    }
    options
}

/// 从（最近设备 × 当前就绪设备）里挑出菜单「连接设备」的目标。
fn pick_tray_target(recent: &[RecentDevice], devices: &[AdbDevice]) -> Option<(String, String)> {    let target = |device: &AdbDevice| (device.serial.clone(), device.label.clone());
    // 最近设备优先：符合「连我刚才用的那台」的直觉。
    for entry in recent {
        if let Some(device) = devices
            .iter()
            .find(|device| device.serial == entry.serial && device.state == DeviceState::Ready)
        {
            return Some(target(device));
        }
    }
    // 没有命中的最近设备时，取第一台就绪设备；未授权/离线设备不能作为目标。
    devices
        .iter()
        .find(|device| device.state == DeviceState::Ready)
        .map(target)
}

/// 菜单里的「开始/结束屏幕录制」。
///
/// X10-92 双通道：开始/结束录制只 spawn/kill 独立的录制进程，**不重启镜像窗口**。
/// 画面全程不中断——这是与旧「重启式录制」的本质区别。
fn tray_record_toggle(app: &AppHandle) {
    let sessions: State<SessionStore> = app.state();
    let runtimes: State<AppRuntimes> = app.state();
    let log: State<DiagnosticsLog> = app.state();
    // 没有真正运行中的显示会话时给统一提示，不去触碰一个不存在的会话。
    if running_session_options(&sessions, None).is_err() {
        tray_notify_no_session(app);
        return;
    }
    let currently = any_session_recording(&sessions);
    if currently {
        // 结束录制：优雅停录制进程，定型 MP4。
        match stop_recording_with(&sessions, None) {
            Ok(Some(rec)) => {
                log.record_outcome("record_stop", None, &[]);
                refresh_tray_menu(app);
                notify_companion_recording(app, false);
                tray_notify_info(app, &format!("已结束屏幕录制：{}（保存在本机视频目录的 MirrorDock 文件夹）。", rec.file_name));
            }
            Ok(None) => tray_notify_info(app, "当前没有在录制。"),
            Err(error) => tray_notify_error(app, &error.message),
        }
        return;
    }
    // 开始录制：起一条独立录制通道，文件名用 Unix 时间戳（纯 ASCII，不猜时区）。
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let file_name = format!("mirrordock-record-{seconds}.mp4");
    match start_recording_with(app, &runtimes, &sessions, None, Some(file_name.clone())) {
        Ok(rec) => {
            log.record_outcome("record_start", None, &[]);
            refresh_tray_menu(app);
            notify_companion_recording(app, true);
            tray_notify_info(app, &format!("已开始屏幕录制：{}（画面不中断）。", rec.file_name));
        }
        Err(error) => tray_notify_error(app, &error.message),
    }
}

/// 菜单里的「屏幕唤醒」。没有进行中的会话时用系统对话框如实说明。
fn tray_wake(app: &AppHandle) {
    let sessions: State<SessionStore> = app.state();
    let runtimes: State<AppRuntimes> = app.state();
    let Some(serial) = tray_active_serial(&sessions) else {
        tray_notify_no_session(app);
        return;
    };
    if let Err(error) = wake_screen_for_serial(&runtimes, &serial) {
        tray_notify_error(app, &error.message);
    }
}

/// 菜单里的「手机截图」。
///
/// 文件名用 Unix 时间戳（纯 ASCII，能过 validate_media_name 白名单），**不猜时区**：
/// 与主窗口里由前端按本地时间命名的截图并存，名字风格不同是刻意取舍——
/// 菜单路径拿不到前端的命名逻辑，也不用为此引入时区依赖。
fn tray_screenshot(app: &AppHandle) {
    let sessions: State<SessionStore> = app.state();
    let runtimes: State<AppRuntimes> = app.state();
    let Some(serial) = tray_active_serial(&sessions) else {
        tray_notify_no_session(app);
        return;
    };
    let directory = match screenshot_dir(app) {
        Ok(directory) => directory,
        Err(error) => {
            tray_notify_error(app, &error.message);
            return;
        }
    };
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let file_name = format!("mirrordock-screenshot-{seconds}.png");
    let result = capture_screenshot_into(&runtimes, &directory, serial.clone(), file_name);
    if let Err(error) = result {
        tray_notify_error(app, &error.message);
    }
}

/// 菜单动作没有会话可作用时的提示。
fn tray_notify_no_session(app: &AppHandle) {
    tray_notify_info(
        app,
        "当前没有进行中的镜像会话。先在 MirrorDock 里连接手机，再使用该功能。",
    );
}

fn tray_notify_info(app: &AppHandle, message: &str) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    app.dialog()
        .message(message.to_string())
        .title("MirrorDock")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::Ok)
        .show(|_| {});
}

fn tray_notify_error(app: &AppHandle, message: &str) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    app.dialog()
        .message(message.to_string())
        .title("MirrorDock")
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::Ok)
        .show(|_| {});
}

// ---------------------------------------------------------------------------
// 锁屏与屏幕状态诊断
//
// 产品边界（AGENTS.md 铁律）：不绕过锁屏、DRM/FLAG_SECURE、受保护页面、MDM 与用户同意。
// 因此这里只做三件事——**读取**当前状态、**点亮**屏幕、如实**说明**为什么需要用户
// 本人解锁。没有任何一条路径会尝试在无凭据的情况下越过锁屏。
//
// 真机实测（Android 13 / Redmi M2104K10AC，见 test-runs/）：设备设置安全锁屏时
// `wm dismiss-keyguard` 不会解除锁屏（`showing` 保持 true），`KEYCODE_WAKEUP` 可以把
// 设备从 Asleep 唤醒到 Awake。即：**唤醒可行，绕过不可行**——后者由 Android 自身拒绝。
// ---------------------------------------------------------------------------

/// 钥匙锁（Keyguard）状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KeyguardState {
    /// 钥匙锁正在显示，需要用户本人解锁。
    Locked,
    /// 钥匙锁未显示；设备处于已解锁状态。
    Unlocked,
    /// 无法确认。**不得**在没有证据时上报为已解锁。
    Unknown,
}

/// 屏幕点亮状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScreenState {
    Awake,
    Asleep,
    Unknown,
}

/// 锁屏与屏幕诊断结果。既有精确状态，也有面向非技术用户的说明与下一步动作。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DeviceLockReport {
    keyguard: KeyguardState,
    /// `Some(true)` 表示设备设置了安全锁屏（PIN / 图案 / 密码 / 生物识别）。
    secure_lock: Option<bool>,
    screen: ScreenState,
    /// 当前状况说明。
    explanation: String,
    /// 可执行的下一步。
    recovery: String,
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// 提取 `dumpsys window policy` 中 `KeyguardServiceDelegate` 段落内的键值对。
///
/// 只认该段落内部缩进更深的行，避免误取输出里其他段落中同名的键。
fn keyguard_fields(dump: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let mut header_indent: Option<usize> = None;
    for line in dump.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        match header_indent {
            None => {
                if trimmed == "KeyguardServiceDelegate" {
                    header_indent = Some(indent);
                }
            }
            Some(header) => {
                if indent <= header {
                    break;
                }
                if let Some((key, value)) = trimmed.split_once('=') {
                    fields.insert(key.trim().to_owned(), value.trim().to_owned());
                }
            }
        }
    }
    fields
}

fn parse_keyguard_state(dump: &str) -> (KeyguardState, Option<bool>) {
    let fields = keyguard_fields(dump);
    let secure_lock = fields.get("secure").and_then(|value| parse_bool(value));
    let keyguard = match fields.get("showing").and_then(|value| parse_bool(value)) {
        Some(true) => KeyguardState::Locked,
        Some(false) => KeyguardState::Unlocked,
        None => KeyguardState::Unknown,
    };
    (keyguard, secure_lock)
}

fn parse_screen_state(dump: &str) -> ScreenState {
    for line in dump.lines() {
        if let Some(value) = line.trim().strip_prefix("mWakefulness=") {
            return match value.trim() {
                "Awake" => ScreenState::Awake,
                "Asleep" | "Dozing" => ScreenState::Asleep,
                _ => ScreenState::Unknown,
            };
        }
    }
    ScreenState::Unknown
}

fn describe_lock_state(
    keyguard: KeyguardState,
    secure_lock: Option<bool>,
    screen: ScreenState,
) -> (String, String) {
    match keyguard {
        KeyguardState::Locked => {
            let explanation = if secure_lock == Some(true) {
                "手机已进入安全锁屏。MirrorDock 会显示锁屏画面，但不会、也无法在你没有输入凭据的情况下越过它——这是 Android 的系统限制，与是否授权调试无关。"
            } else {
                "手机停留在锁屏画面。请在手机上手动解锁后继续。"
            };
            (
                explanation.to_owned(),
                "请在手机上解锁，或直接在镜像窗口中输入你自己的解锁凭据（凭据不会被记录）。开启「会话期间保持唤醒」可以避免镜像过程中再次锁屏。"
                    .to_owned(),
            )
        }
        KeyguardState::Unlocked => {
            if screen == ScreenState::Asleep {
                (
                    "手机已解锁，仅屏幕处于关闭状态；设备并未锁定，可以远程点亮后继续操作。"
                        .to_owned(),
                    "点击「屏幕唤醒」，手机亮起后即可直接在镜像窗口中操作。".to_owned(),
                )
            } else {
                (
                    "手机已解锁且屏幕点亮，可以直接在镜像窗口中操作。".to_owned(),
                    "无需额外操作。".to_owned(),
                )
            }
        }
        KeyguardState::Unknown => (
            "无法确认手机当前的锁屏状态。".to_owned(),
            "请查看手机屏幕确认状态；若镜像画面正常即可直接操作。".to_owned(),
        ),
    }
}

fn lock_report_with(
    runtimes: &AppRuntimes,
    serial: String,
) -> Result<DeviceLockReport, AppError> {
    let serial = validate_serial(&serial)?;

    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }

    let policy = runtimes.adb.window_policy(&serial).map_err(|error| {
        adb_command_error(
            error,
            "lock_probe_failed",
            "无法读取手机的锁屏状态。",
            "请确认数据线或无线连接仍然有效，然后重试。",
        )
    })?;
    let (keyguard, secure_lock) = parse_keyguard_state(&policy);

    // 屏幕状态只是辅助信息：读不到就保持 Unknown，不影响锁屏结论。
    let screen = runtimes
        .adb
        .power_state(&serial)
        .map(|dump| parse_screen_state(&dump))
        .unwrap_or(ScreenState::Unknown);

    let (explanation, recovery) = describe_lock_state(keyguard, secure_lock, screen);
    Ok(DeviceLockReport {
        keyguard,
        secure_lock,
        screen,
        explanation,
        recovery,
    })
}

// ---------------------------------------------------------------------------
// 密码输入页（安全表面）探测
//
// 真机定案（2026-09-29，双路取证）：锁屏壁纸页可以镜像；用户上滑调出密码输入页
// （PIN / 图案凭据界面）后，系统对 screencap 与虚拟显示器镜像**同时**拒绝输出——
// 截屏返回 0 字节、scrcpy 流帧全黑，而物理屏亮着、照常可输入。这是平台级安全
// 保护：不可绕过，也不应绕过（各机型屏幕与布局差异大，映射点击代输也不通用）。
//
// 产品上只做一件事：检测到密码页 → 明确告知用户「此画面受系统安全保护，无法
// 镜像，请在手机上直接输入密码解锁」，密码页退出后提示自动消失。
// ---------------------------------------------------------------------------

/// screencap 输出低于该字节数即判定为「安全表面挡住了截屏」。
/// 实测（1080×2400）：正常锁屏壁纸页约 3.4~3.7 MB；密码页实测 0 字节；
/// 个别设备可能输出近乎纯黑的 PNG（压缩后同样远小于真实内容）。
const SECURE_SURFACE_MAX_CAPTURE_BYTES: u64 = 100_000;

/// 一次密码页探测的结论。`detail` 是稳定标记，供诊断与前端区分原因，不是错误文案。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PinPadProbe {
    active: bool,
    detail: &'static str,
}

fn pin_pad_probe_with(runtimes: &AppRuntimes, serial: String) -> Result<PinPadProbe, AppError> {
    let serial = validate_serial(&serial)?;
    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }
    // 三个前置条件任一不满足都如实返回「未激活」：不在锁屏 / 读不到 / 屏幕没点亮
    // 时，镜像黑屏另有原因（熄屏、DIM 等），不该误报成密码页。
    let (keyguard, _) = runtimes
        .adb
        .window_policy(&serial)
        .map(|dump| parse_keyguard_state(&dump))
        .unwrap_or((KeyguardState::Unknown, None));
    if keyguard != KeyguardState::Locked {
        return Ok(PinPadProbe { active: false, detail: "keyguard_not_showing" });
    }
    let screen = runtimes
        .adb
        .power_state(&serial)
        .map(|dump| parse_screen_state(&dump))
        .unwrap_or(ScreenState::Unknown);
    if screen != ScreenState::Awake {
        return Ok(PinPadProbe { active: false, detail: "screen_not_awake" });
    }
    // 截屏探测本身不保存、不回传任何像素——只看字节数。
    match runtimes.adb.screencap_probe_bytes(&serial) {
        Ok(bytes) if bytes <= SECURE_SURFACE_MAX_CAPTURE_BYTES => Ok(PinPadProbe {
            active: true,
            detail: "secure_surface_blocked_capture",
        }),
        Ok(_) => Ok(PinPadProbe { active: false, detail: "capture_ok" }),
        // 探测失败不等于密码页激活：宁可少提示，不可误报。
        Err(_) => Ok(PinPadProbe { active: false, detail: "capture_failed" }),
    }
}

// ---------------------------------------------------------------------------
// 最近设备（本地列表，按最近使用排序）
// ---------------------------------------------------------------------------

const MAX_RECENT_DEVICES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RecentDevice {
    serial: String,
    label: String,
    /// 最近一次成功启动镜像的 Unix 时间戳（秒）。
    last_used_at: u64,
}

/// 把一台设备置顶到最近设备列表：按 serial 去重、新的在前、超出上限截断。
fn record_recent_device(devices: &mut Vec<RecentDevice>, device: RecentDevice) {
    devices.retain(|existing| existing.serial != device.serial);
    devices.insert(0, device);
    devices.truncate(MAX_RECENT_DEVICES);
}

fn recent_devices_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("recent-devices.json"))
        .map_err(|_| {
            AppError::new(
                "recent_list_unavailable",
                "无法访问本机的最近设备记录。",
                "请检查本机文件权限，或重新安装 MirrorDock。",
            )
        })
}

// ---------------------------------------------------------------------------
// 桌面模式偏好（X10-90）：后端持久化，托盘等任意入口都能读取
// ---------------------------------------------------------------------------

/// 单台设备的桌面模式偏好。与前端 `DesktopPref` 同构，键为稳定 physical_serial。
///
/// 此前只存于前端 localStorage（webview 域），托盘「连接设备」这条 Rust 路径
/// 完全够不到——用户配了桌面模式，从托盘点连接却启动成普通镜像（真机实测）。
/// 移入后端 app data 后，托盘与主窗口任何入口都能叠加同一台设备的桌面模式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DesktopPrefEntry {
    desktop_mode: bool,
    #[serde(default)]
    desktop_app: Option<String>,
}

/// 把「adb 无线端点 / mDNS 端点」归一化成物理序列号（与前端
/// `normalizeDeviceIdentityKey` 完全同规则，作为解析 pref 键的兜底）：
/// `adb-<physical>-<随机>._adb-tls-connect._tcp` → `<physical>`；
/// `192.168.x.x:port` 不含物理 id 原样返回；其他（短物理 id / USB serial）原样返回。
fn normalize_device_identity_key(key: &str) -> String {
    if let Some(rest) = key.strip_prefix("adb-") {
        if let Some(head) = rest.strip_suffix("._adb-tls-connect._tcp") {
            // 形如 `<physical>-<随机>`：物理 id 自身不含 `-`（ro.serialno 是字母数字）。
            if let Some((physical, _random)) = head.split_once('-') {
                if !physical.is_empty()
                    && physical
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric())
                {
                    return physical.to_ascii_lowercase();
                }
            }
        }
    }
    key.to_owned()
}

/// 把 adb serial（可能是无线端点）解析成桌面模式偏好的稳定键：
/// 优先经设备列表反查 `physical_serial`；解析不到（设备已下线）时归一化兜底。
/// 与前端 `desktopPrefKeyFor` 同口径——全链路唯一。
fn desktop_pref_key_for(devices: &[AdbDevice], serial: &str) -> String {
    devices
        .iter()
        .find(|device| {
            device.serial == serial
                || device.connections.iter().any(|c| c.serial == serial)
        })
        .and_then(|device| device.physical_serial.clone())
        .unwrap_or_else(|| normalize_device_identity_key(serial))
}

fn desktop_prefs_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("desktop-prefs.json"))
        .map_err(|_| {
            AppError::new(
                "recent_list_unavailable",
                "无法访问本机的桌面模式配置。",
                "请检查本机文件权限，或重新安装 MirrorDock。",
            )
        })
}

/// 读取桌面模式偏好。解析失败（手改坏 / 版本升级格式漂移）返回空表而非报错——
/// 与前端 `readDesktopPrefs` 的容错策略一致，配置丢失可重建，不能挡住连接主流程。
fn load_desktop_prefs(path: &Path) -> std::collections::HashMap<String, DesktopPrefEntry> {
    let Ok(bytes) = fs::read(path) else {
        return std::collections::HashMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn save_desktop_prefs(
    path: &Path,
    prefs: &std::collections::HashMap<String, DesktopPrefEntry>,
) -> Result<(), AppError> {
    if let Some(directory) = path.parent() {
        let _ = fs::create_dir_all(directory);
    }
    let serialized = serde_json::to_vec_pretty(prefs).map_err(|_| {
        AppError::new(
            "desktop_prefs_write_failed",
            "无法整理桌面模式配置。",
            "请重试。",
        )
    })?;
    fs::write(path, serialized).map_err(|_| {
        AppError::new(
            "desktop_prefs_write_failed",
            "无法保存桌面模式配置。",
            "请检查本机文件权限。",
        )
    })
}

/// 查出某台设备的桌面模式偏好（供托盘等后端入口叠加）。
fn desktop_pref_for_serial(
    app: &AppHandle,
    devices: &[AdbDevice],
    serial: &str,
) -> Option<DesktopPrefEntry> {
    let path = desktop_prefs_path(app).ok()?;
    let prefs = load_desktop_prefs(&path);
    prefs.get(&desktop_pref_key_for(devices, serial)).cloned()
}

fn load_recent_devices(path: &Path) -> Result<Vec<RecentDevice>, AppError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let unreadable = || {
        AppError::new(
            "recent_list_unreadable",
            "最近设备记录无法读取。",
            "请重新选择设备开始镜像；这条记录会被重建。",
        )
    };
    let bytes = fs::read(path).map_err(|_| unreadable())?;
    serde_json::from_slice(&bytes).map_err(|_| unreadable())
}

fn save_recent_devices(path: &Path, devices: &[RecentDevice]) -> Result<(), AppError> {
    let directory = path.parent().ok_or_else(|| {
        AppError::new(
            "recent_list_write_failed",
            "无法创建最近设备记录。",
            "请检查本机文件权限。",
        )
    })?;
    fs::create_dir_all(directory).map_err(|_| {
        AppError::new(
            "recent_list_write_failed",
            "无法创建最近设备记录目录。",
            "请检查本机文件权限。",
        )
    })?;
    let serialized = serde_json::to_vec_pretty(devices).map_err(|_| {
        AppError::new(
            "recent_list_write_failed",
            "无法整理最近设备记录。",
            "请重试。",
        )
    })?;
    fs::write(path, serialized).map_err(|_| {
        AppError::new(
            "recent_list_write_failed",
            "无法保存最近设备记录。",
            "请检查本机文件权限。",
        )
    })
}

/// 从最近设备列表中移除一台设备；返回是否真的移除了。
///
/// 最近设备是**本地便利记录**，用户必须能撤销它——否则“本地优先”就只是口号。
fn forget_recent_device_from(devices: &mut Vec<RecentDevice>, serial: &str) -> bool {
    let before = devices.len();
    devices.retain(|device| device.serial != serial);
    before != devices.len()
}

/// 从磁盘上的记录中移除一台设备，并返回移除后的完整列表（省去前端再取一次）。
fn forget_recent_device_at(path: &Path, serial: &str) -> Result<Vec<RecentDevice>, AppError> {
    let mut devices = load_recent_devices(path)?;
    forget_recent_device_from(&mut devices, serial);
    save_recent_devices(path, &devices)?;
    Ok(devices)
}

/// 记录一台成功启动过镜像的设备。这是本地便利功能：读写失败不得影响镜像主流程。
fn remember_recent_device(app: &AppHandle, serial: &str, label: &str) {
    let Ok(path) = recent_devices_path(app) else {
        return;
    };
    let Ok(mut devices) = load_recent_devices(&path) else {
        return;
    };
    record_recent_device(
        &mut devices,
        RecentDevice {
            serial: serial.to_owned(),
            label: label.to_owned(),
            last_used_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
        },
    );
    let _ = save_recent_devices(&path, &devices);
}

// ---------------------------------------------------------------------------
// 截图：把手机当前画面保存为本机文件（可查看、可撤销）
// ---------------------------------------------------------------------------

/// PNG 文件头。用它确认拿到的确实是图片，而不是被文本模式改写的字节流。
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// 文件名长度上限。文件名由前端按本地时间生成，必须当作不可信输入。
const MAX_MEDIA_NAME_LEN: usize = 128;

/// 同名文件自动加序号时最多尝试的次数，避免异常情况下无界循环。
const MAX_MEDIA_NAME_ATTEMPTS: u32 = 100;

/// 一张已保存的截图。
///
/// 只返回文件名、路径与字节数：**不返回任何像素数据**。屏幕内容不进日志、不进错误消息、
/// 不进任何遥测；它只落在用户可见的本地文件里。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Screenshot {
    /// 保存后的文件名（不含目录）。与请求名可能不同：同名时自动加序号。
    file_name: String,
    /// 本机完整路径，供界面展示与「在文件夹中显示」。
    path: String,
    /// 文件字节数，用于向用户确认「确实存下来了」。
    bytes: usize,
}

fn media_name_invalid_error() -> AppError {
    AppError::new(
        "media_name_invalid",
        "文件名不可用。",
        "请使用字母、数字、短横线和下划线组成的文件名。",
    )
}

fn screenshot_dir_unavailable_error() -> AppError {
    AppError::new(
        "screenshot_dir_unavailable",
        "无法确定截图的保存位置。",
        "请检查本机文件权限，或重新安装 MirrorDock。",
    )
}

/// 校验一个由外部给出的文件名，并要求它属于给定扩展名。
///
/// 这是本机写入路径的一部分，必须按不可信输入处理：只放行 ASCII 白名单，因此
/// `/`、`\`、`..`、控制字符以及隐藏文件前缀都无法通过，也就不存在路径穿越。
fn validate_media_name(name: &str, extension: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_MEDIA_NAME_LEN {
        return Err(media_name_invalid_error());
    }
    let allowed = name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !allowed || name.starts_with('.') || name.contains("..") {
        return Err(media_name_invalid_error());
    }
    if !name.to_ascii_lowercase().ends_with(&format!(".{extension}")) {
        return Err(media_name_invalid_error());
    }
    Ok(name.to_owned())
}

fn validate_screenshot_name(name: &str) -> Result<String, AppError> {
    validate_media_name(name, "png")
}

/// 确认 adb 返回的字节流真的是 PNG。
///
/// 只看退出码并不够：字节流被文本模式改写、或中间层插入诊断信息，都会出现
/// 「命令成功但内容不是图片」。宁可如实报错，也不写出一个打不开的文件。
fn ensure_png(bytes: &[u8]) -> Result<(), AppError> {
    if bytes.len() > PNG_MAGIC.len() && bytes.starts_with(&PNG_MAGIC) {
        Ok(())
    } else {
        Err(AppError::new(
            "screenshot_not_image",
            "手机返回的内容不是有效的图片。",
            "请重试一次；若反复失败，请改用 USB 数据线连接后再试。",
        ))
    }
}

fn screenshot_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .picture_dir()
        .or_else(|_| app.path().app_data_dir())
        .map(|directory| directory.join("MirrorDock"))
        .map_err(|_| screenshot_dir_unavailable_error())
}

/// 在目标目录里为 `name` 找一个尚未被占用的文件名。
///
/// 前端按秒生成文件名，连续操作会撞名；撞名时顺延加序号，而不是覆盖用户已有的文件。
fn unique_file_path(directory: &Path, name: &str) -> PathBuf {
    let first = directory.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) => (stem, format!(".{extension}")),
        None => (name, String::new()),
    };
    for index in 1..=MAX_MEDIA_NAME_ATTEMPTS {
        let candidate = directory.join(format!("{stem}-{index}{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

/// 把截图字节写进目标目录，返回用户可见的结果。
fn save_screenshot_bytes(
    directory: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<Screenshot, AppError> {
    fs::create_dir_all(directory).map_err(|_| screenshot_dir_unavailable_error())?;
    let path = unique_file_path(directory, name);
    fs::write(&path, bytes).map_err(|_| {
        AppError::new(
            "screenshot_write_failed",
            "无法把截图保存到本机。",
            "请检查保存目录是否可写、磁盘是否已满，然后重试。",
        )
    })?;
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_owned());
    Ok(Screenshot {
        file_name,
        path: path.to_string_lossy().into_owned(),
        bytes: bytes.len(),
    })
}

/// 删除一张由本应用保存的截图。
///
/// 文件名先经同一套白名单校验，因此只可能落在截图目录内部；这是「截图可撤销」的落点。
fn remove_screenshot_file(directory: &Path, name: &str) -> Result<(), AppError> {
    let path = directory.join(name);
    if !path.is_file() {
        return Err(AppError::new(
            "screenshot_missing",
            "这张截图已经不在了。",
            "它可能已被移动或删除，请刷新后重试。",
        ));
    }
    fs::remove_file(&path).map_err(|_| {
        AppError::new(
            "screenshot_delete_failed",
            "无法删除这张截图。",
            "请在本机的文件管理器中手动删除它。",
        )
    })
}

/// 截图编排：校验 → 确认设备就绪 → 取画面 → 确认真的是图片 → 落盘。
fn capture_screenshot_into(
    runtimes: &AppRuntimes,
    directory: &Path,
    serial: String,
    file_name: String,
) -> Result<Screenshot, AppError> {
    let file_name = validate_screenshot_name(&file_name)?;
    let serial = validate_serial(&serial)?;

    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }

    let bytes = runtimes.adb.screenshot_png(&serial).map_err(|error| {
        adb_command_error(
            error,
            "screenshot_failed",
            "无法从手机读取当前画面。",
            "请确认连接仍然有效；若手机正在重启或刚断开，请稍后重试。",
        )
    })?;
    ensure_png(&bytes)?;

    save_screenshot_bytes(directory, &file_name, &bytes)
}

// ---------------------------------------------------------------------------
// 文件传输：本机 <-> 设备 /sdcard/Download/MirrorDock
// ---------------------------------------------------------------------------

/// 设备端文件传输目录。收发都限制在这个目录内，不碰设备的其它位置。
const DEVICE_TRANSFER_DIR: &str = "/sdcard/Download/MirrorDock";

/// 传输文件名的长度上限（字节数）。文件名可能来自本机文件系统或设备目录清单，
/// 两者都必须当作不可信输入。
const MAX_TRANSFER_NAME_LEN: usize = 128;

/// 一次收发的结果回执。只含文件名、路径与字节数，不含任何文件内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TransferReceipt {
    /// 实际生效的文件名（同名时可能已加序号）。
    file_name: String,
    /// 发送时为设备上的完整路径；接收时为本机完整路径。
    path: String,
    bytes: u64,
}

fn transfer_name_invalid_error() -> AppError {
    AppError::new(
        "transfer_name_invalid",
        "文件名不可用。",
        "请使用常规的文件名（不含路径分隔符或特殊控制字符）后重试。",
    )
}

fn transfer_local_invalid_error() -> AppError {
    AppError::new(
        "transfer_local_missing",
        "选择的文件不存在或不是一个文件。",
        "请重新选择一个本地文件。",
    )
}

fn transfer_dir_unavailable_error() -> AppError {
    AppError::new(
        "transfer_dir_unavailable",
        "无法确定文件的保存位置。",
        "请检查本机文件权限，或重新安装 MirrorDock。",
    )
}

/// 校验一个传输文件名。
///
/// 与截图/录像不同：这里的文件名可能来自设备上的真实文件，常常包含中文等非
/// ASCII 字符，因此不能用 ASCII 白名单。改用黑名单：拒绝路径分隔符（`/`、`\`）、
/// `..`、控制字符、隐藏文件前缀与超长名——枚举出的每一项都是唯一的逃逸途径，
/// 列尽它们之后，名字只可能落在传输目录内部。
fn validate_transfer_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_TRANSFER_NAME_LEN {
        return Err(transfer_name_invalid_error());
    }
    if name.starts_with('.')
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || name.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(transfer_name_invalid_error());
    }
    Ok(name.to_owned())
}

/// 设备上某个文件名的完整路径。名字先经 `validate_transfer_name` 校验，
/// 因此拼接结果不可能逃出传输目录。
fn device_transfer_path(name: &str) -> String {
    format!("{DEVICE_TRANSFER_DIR}/{name}")
}

/// 设备目录清单解析：每行一个条目，剔除空行；`\r` 来自部分平台的行尾。
fn parse_device_listing(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|line| line.trim_end_matches('\r').trim())
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// 把一个本机文件发送到设备的传输目录。
fn send_file_to_device_with(
    runtimes: &AppRuntimes,
    serial: String,
    local_path: String,
) -> Result<TransferReceipt, AppError> {
    let serial = validate_serial(&serial)?;

    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }

    // 本机路径由用户通过系统文件选择器给出，先确认它真实存在且是文件。
    let local = PathBuf::from(&local_path);
    let metadata = fs::metadata(&local).map_err(|_| transfer_local_invalid_error())?;
    if !metadata.is_file() {
        return Err(transfer_local_invalid_error());
    }
    let bytes = metadata.len();
    let file_name = local
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(transfer_local_invalid_error)?;
    let file_name = validate_transfer_name(&file_name)?;

    runtimes
        .adb
        .make_directory(&serial, DEVICE_TRANSFER_DIR)
        .map_err(|_| {
            AppError::new(
                "transfer_push_failed",
                "无法在手机上准备接收目录。",
                "请确认手机存储可用、连接仍然有效，然后重试。",
            )
        })?;
    runtimes
        .adb
        .push_file(&serial, &local, DEVICE_TRANSFER_DIR)
        .map_err(|_| {
            AppError::new(
                "transfer_push_failed",
                "文件没有传到手机上。",
                "请确认连接仍然有效、手机存储空间充足，然后重试。",
            )
        })?;

    Ok(TransferReceipt {
        path: device_transfer_path(&file_name),
        file_name,
        bytes,
    })
}

/// 列出设备传输目录里的文件名。
fn list_device_files_with(
    runtimes: &AppRuntimes,
    serial: String,
) -> Result<Vec<String>, AppError> {
    let serial = validate_serial(&serial)?;

    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }

    // 先确保目录存在（幂等），避免「还没发送过文件」时把目录缺失误报成故障。
    runtimes
        .adb
        .make_directory(&serial, DEVICE_TRANSFER_DIR)
        .map_err(|_| {
            AppError::new(
                "transfer_list_failed",
                "无法读取手机上的文件列表。",
                "请确认连接仍然有效，然后重试。",
            )
        })?;
    let raw = runtimes
        .adb
        .list_directory(&serial, DEVICE_TRANSFER_DIR)
        .map_err(|_| {
            AppError::new(
                "transfer_list_failed",
                "无法读取手机上的文件列表。",
                "请确认连接仍然有效，然后重试。",
            )
        })?;
    Ok(parse_device_listing(&raw))
}

/// 从设备传输目录取回一个文件，保存到本机「下载 / MirrorDock」。
fn fetch_file_from_device_into(
    runtimes: &AppRuntimes,
    directory: &Path,
    serial: String,
    file_name: String,
) -> Result<TransferReceipt, AppError> {
    let serial = validate_serial(&serial)?;
    let file_name = validate_transfer_name(&file_name)?;

    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }

    fs::create_dir_all(directory).map_err(|_| transfer_dir_unavailable_error())?;
    let local = unique_file_path(directory, &file_name);
    runtimes
        .adb
        .pull_file(&serial, &device_transfer_path(&file_name), &local)
        .map_err(|_| {
            AppError::new(
                "transfer_pull_failed",
                "文件没有从手机取回来。",
                "请确认手机上这个文件还在，然后重试。",
            )
        })?;
    let bytes = fs::metadata(&local)
        .map(|metadata| metadata.len())
        .map_err(|_| transfer_dir_unavailable_error())?;

    Ok(TransferReceipt {
        file_name: local
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_name.clone()),
        path: local.to_string_lossy().into_owned(),
        bytes,
    })
}

/// 删除设备传输目录里的一个文件（发送区清理，X10-60）。
///
/// 只允许删 `DEVICE_TRANSFER_DIR` 内经 `validate_transfer_name` 校验的文件名；
/// 名单外的一切（路径分隔符、`..`、超长名）在拼路径前就被拒绝。
/// 把任意字符串安全地包成**设备端 shell** 的单个词。
///
/// 存在的理由（2026-10-03 真机实证）：`adb shell ARGV...` 不是把 argv 直接
/// exec 到设备，而是先用空格拼成一条命令串，再交给设备端的 shell 解析。
/// 因此「作为单独 argv 传入」**不能**保护空格 —— 传
/// `/sdcard/Dir/has space.txt` 会被拆成两个词。
///
/// 用单引号包裹，并按 POSIX 惯例把内嵌的单引号写成 `'"'"'`：
/// - `abc`      → `'abc'`
/// - `has space` → `'has space'`
/// - `it's`     → `'it'"'"'s'`
///
/// 单引号内一切字符都是字面量（含 `$`、反引号、`\`、`*`），所以这同时
/// 消除了注入面。**只用于经过远端 shell 的调用**；`adb push` / `adb pull`
/// 不经 shell，路径要原样传（见 `push_file` / `pull_file`）。
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// 构造 `adb rm` 的 argv。抽成函数是为了让测试能覆盖**接线**，
/// 而不只是测 `shell_quote` 这个纯函数（2026-10-03 教训：只测实现的测试
/// 无法发现"忘了调用它"）。
///
/// `rm -f`：文件已不存在视为清理成功，避免并发刷新时的竞态误报。
///
/// 路径**必须做 shell 引号**（2026-10-03 真机实证，见 X10-77）：
/// `adb shell` 会把 argv 用空格拼成一条命令串，再交给**设备端的 shell**
/// 重新解析。所以「作为单个 argv 传入」并不能保护空格 —— 路径
/// `/sdcard/Download/MirrorDock/has space.txt` 会被远端拆成
/// `/sdcard/.../has` 与 `space.txt` 两个词，`rm -f` 对两个都不存在的路径
/// 静默返回 0，于是**文件没删掉、后端却报告成功**。
/// `--` 也救不了：它是给本地 shell 用的，远端 shell 已在 argv 拼接之后
/// 才看到这条命令。
fn remove_device_file_argv(serial: &str, remote_path: &str) -> Vec<String> {
    vec![
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
        "rm".to_string(),
        "-f".to_string(),
        "--".to_string(),
        shell_quote(remote_path),
    ]
}

fn delete_device_file_with(
    runtimes: &AppRuntimes,
    serial: String,
    file_name: String,
) -> Result<(), AppError> {
    let serial = validate_serial(&serial)?;
    let file_name = validate_transfer_name(&file_name)?;

    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }

    runtimes
        .adb
        .remove_device_file(&serial, &device_transfer_path(&file_name))
        .map_err(|_| {
            AppError::new(
                "transfer_delete_failed",
                "文件没有从手机上删除。",
                "请确认连接仍然有效，然后重试。",
            )
        })
}

// ---------------------------------------------------------------------------
// 安装 APK：把用户选中的安装包直接装到手机上
// ---------------------------------------------------------------------------

fn apk_path_invalid_error() -> AppError {
    AppError::new(
        "apk_path_invalid",
        "选择的文件不是可用的安装包。",
        "请选择扩展名为 .apk 的安装包文件；如果它是压缩包或已损坏，请重新下载。",
    )
}

/// 安装回执：文件名、字节数与 adb 结论的可读摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ApkInstallReceipt {
    file_name: String,
    bytes: u64,
    /// 面向用户的结论（如「安装完成。」）；失败时不会走到这里，而是返回错误。
    summary: String,
}

/// 校验用户选中的 APK 路径，返回（路径, 文件名, 字节数）。
///
/// 只接受「绝对路径 + 已存在的普通文件 + `.apk` 后缀」。**不做目录白名单**：安装包
/// 可能在下载、桌面或 U 盘里，路径由系统文件选择器给出，这里是二次确认（防止把
/// 任意文件塞给 adb）。后缀严格限制是刻意的——`adb install` 对非 APK 只会回一句
/// 难懂的解析错误，提前拒绝能给出可照做的提示。
fn validate_apk_path(path: &str) -> Result<(PathBuf, String, u64), AppError> {
    let local = PathBuf::from(path.trim());
    if local.as_os_str().is_empty() || !local.is_absolute() {
        return Err(apk_path_invalid_error());
    }
    let metadata = fs::metadata(&local).map_err(|_| apk_path_invalid_error())?;
    if !metadata.is_file() {
        return Err(apk_path_invalid_error());
    }
    let file_name = local
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(apk_path_invalid_error)?;
    let lowered = file_name.to_ascii_lowercase();
    if !lowered.ends_with(".apk") || lowered.len() <= ".apk".len() {
        return Err(apk_path_invalid_error());
    }
    Ok((local, file_name, metadata.len()))
}

/// 解析 `adb install` 的输出，给出「是否成功 + 可读结论」。
///
/// 成功判定：输出里出现 `Success`。失败时尽量取出 `Failure [REASON]` 的 REASON，
/// 翻译成用户能照做的动作；遇到未知 REASON 就把原始错误码一起带上，便于用户复述
/// 与检索——而不是把具体原因塌缩成一句笼统的「安装失败」。
fn describe_apk_install_output(raw: &str) -> (bool, String) {
    if raw.contains("Success") {
        return (true, "安装完成。".to_owned());
    }
    let reason = raw.lines().find_map(|line| {
        let line = line.trim();
        let open = line.find('[')?;
        let closing = line[open..].find(']')?;
        let value = line[open + 1..open + closing].trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_owned())
        }
    });
    let advice = match reason.as_deref() {
        Some("INSTALL_FAILED_ALREADY_EXISTS") => {
            "手机里已有同包名的应用，且无法覆盖安装。请先在手机上卸载旧版本再试。"
        }
        Some("INSTALL_FAILED_VERSION_DOWNGRADE") => {
            "手机里已安装的版本比这个安装包更新。请先卸载手机上的版本，或改用版本号更高的安装包。"
        }
        Some("INSTALL_FAILED_UPDATE_INCOMPATIBLE") => {
            "与手机里已安装的同名应用签名不一致，无法覆盖。请先在手机上卸载旧应用再安装。"
        }
        Some("INSTALL_FAILED_DUPLICATE_PACKAGE") => "手机里已有同名的系统应用，无法覆盖。",
        Some("INSTALL_FAILED_INSUFFICIENT_STORAGE") => "手机存储空间不足。请清理空间后重试。",
        Some("INSTALL_FAILED_INVALID_APK") => "这个文件不是有效的安装包，或已损坏。",
        Some("INSTALL_FAILED_INVALID_URI") => {
            "安装包在传输过程中出错（文件不完整）。请重新选择安装包再试。"
        }
        Some("INSTALL_PARSE_FAILED_NO_CERTIFICATES") => "安装包没有签名，系统拒绝安装。",
        Some("INSTALL_FAILED_TEST_ONLY") => "这是仅供测试的安装包，手机拒绝了它。",
        Some("INSTALL_FAILED_USER_RESTRICTED") => {
            "手机当前限制了安装（如「安装未知应用」开关、儿童模式或工作资料限制）。请在手机上允许后重试。"
        }
        Some("INSTALL_FAILED_OLDER_SDK") => "这个安装包要求更高的 Android 版本，当前手机不支持。",
        Some("INSTALL_FAILED_DEPRECATED_SDK_VERSION") => {
            "这个安装包面向的 Android 版本过旧，当前系统拒绝安装。"
        }
        Some("INSTALL_FAILED_NO_MATCHING_ABIS") => "安装包不支持这台手机的处理器架构。",
        Some("INSTALL_FAILED_ABORTED") => "安装被手机上取消。请重新操作，并在手机上确认安装。",
        Some("INSTALL_FAILED_VERIFICATION_FAILURE") => "手机的应用校验没有通过这个安装包。",
        Some("INSTALL_FAILED_CONFLICTING_PROVIDER") => {
            "与手机里已安装的应用存在冲突。请先卸载相关旧应用再试。"
        }
        Some("INSTALL_FAILED_MEDIA_UNAVAILABLE") => "手机存储当前不可用。请检查存储状态后重试。",
        _ => "安装没有完成。请查看手机屏幕上的提示，处理后重试。",
    };
    match reason {
        Some(code) => (false, format!("{advice}（{code}）")),
        None => (false, advice.to_owned()),
    }
}

/// 在指定设备上安装一个 APK。
fn install_apk_with(
    runtimes: &AppRuntimes,
    serial: String,
    apk_path: String,
) -> Result<ApkInstallReceipt, AppError> {
    let serial = validate_serial(&serial)?;

    // 先确认连接可用：未授权/离线时直接给出对应的恢复动作，不去跑一次注定失败的安装。
    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }
    let (apk, file_name, bytes) = validate_apk_path(&apk_path)?;

    let raw = runtimes.adb.install_apk(&serial, &apk).map_err(|_| {
        AppError::new(
            "apk_install_failed",
            "无法在这台手机上执行安装。",
            "请确认数据线或无线连接仍然有效、手机保持解锁，然后重试。",
        )
    })?;
    let (installed, summary) = describe_apk_install_output(&raw);
    if !installed {
        return Err(AppError::new(
            "apk_install_failed",
            &summary,
            "如果手机屏幕上有提示，请按提示处理后再试一次。",
        ));
    }
    Ok(ApkInstallReceipt {
        file_name,
        bytes,
        summary,
    })
}

// ---------------------------------------------------------------------------
// 录制：把本会话的画面录成 MP4 保存在本机
// ---------------------------------------------------------------------------

/// 最近一次会话的录制文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Recording {
    /// 文件名（不含目录）。
    file_name: String,
    /// 本机完整路径，供界面展示与「在文件夹中显示」。
    path: String,
    /// 是否仍在录制：只有镜像进程还在运行时才为 true。
    ///
    /// 录像由镜像进程本身写盘，进程一退出文件就定型。因此「磁盘上有一个 mp4」与
    /// 「此刻正在录」是两件事，界面必须把它们区分开。
    active: bool,
}

fn recording_dir_unavailable_error() -> AppError {
    AppError::new(
        "recording_dir_unavailable",
        "无法确定录像的保存位置。",
        "请检查本机文件权限，或重新安装 MirrorDock。",
    )
}

fn recording_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    // X10-95：用户自定义目录优先；未设置或目录失效回退系统默认（视频目录/MirrorDock）。
    // 自定义目录必须是绝对路径——相对路径会被前端误传成不可预期的位置，直接拒用。
    if let Ok(settings) = app_settings_path(app).map(|p| load_app_settings(&p)) {
        if let Some(custom) = settings.recording_dir.as_deref() {
            let candidate = PathBuf::from(custom);
            if candidate.is_absolute() {
                return Ok(candidate);
            }
        }
    }
    app.path()
        .video_dir()
        .or_else(|_| app.path().app_data_dir())
        .map(|directory| directory.join("MirrorDock"))
        .map_err(|_| recording_dir_unavailable_error())
}

/// 为本次会话准备录制文件路径；`enabled` 为 false 时返回 `None`。
///
/// 文件名由前端按本机时间生成（后端不猜时区），随后按不可信输入严格校验；**目录由
/// 后端决定**——前端只能给名字，给不了路径。
fn prepare_recording_path(
    app: &AppHandle,
    enabled: bool,
    file_name: Option<&str>,
) -> Result<Option<PathBuf>, AppError> {
    if !enabled {
        return Ok(None);
    }
    let name = validate_media_name(file_name.unwrap_or("MirrorDock-recording.mp4"), "mp4")?;
    let directory = recording_dir(app)?;
    fs::create_dir_all(&directory).map_err(|_| recording_dir_unavailable_error())?;
    Ok(Some(unique_file_path(&directory, &name)))
}

/// 读取主会话最近一次的录制信息（若有）。`active` 表示此刻进程是否仍在写这个文件。
/// X10-27 后多台设备可能同时录制；界面「撤销」入口按主会话展示，删除校验则扫描全部会话。
#[tauri::command]
fn current_recording(sessions: State<SessionStore>) -> Result<Option<Recording>, AppError> {
    current_recording_with(&sessions)
}

/// X10-92：开始录制（双通道）。在显示会话之外另起一条 `--no-playback --no-window` 的
/// scrcpy 录制进程，画面窗口全程不重启。要求该设备已有运行中的显示会话。
#[tauri::command]
fn start_recording(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    serial: Option<String>,
    file_name: Option<String>,
) -> Result<Recording, AppError> {
    let result = start_recording_with(&app, &runtimes, &sessions, serial, file_name)?;
    // X10-95：录制状态跃迁后刷新托盘菜单（开始→「结束屏幕录制」）。
    refresh_tray_menu(&app);
    Ok(result)
}

/// X10-103：从显示通道的 scrcpy 输出解析虚拟屏 id，供录制通道 `--display-id` 捕获。
///
/// scrcpy 建虚拟屏后会在输出里打印 `New display: <WxH>/<dpi> (id=N)`，但**不是启动瞬间**
/// 就有——所以要短重试（最多 ~1.5s）。超时仍拿不到就返回 None，由 record_arguments
/// 报「虚拟画面还没就绪，请稍候重试」，而不是静默录一块空屏（X10-101 的白屏教训）。
fn resolve_desktop_display_id(store: &SessionStore, target: &str) -> Option<u32> {
    for _ in 0..15 {
        let output = {
            let Ok(map) = store.0.lock() else { return None };
            let state = map.get(target)?;
            let process = state.process.as_ref()?;
            process.output_tail()
        };
        if let Some(id) = parse_desktop_display_id(&output) {
            return Some(id);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

fn start_recording_with(
    app: &AppHandle,
    runtimes: &AppRuntimes,
    store: &SessionStore,
    serial: Option<String>,
    file_name: Option<String>,
) -> Result<Recording, AppError> {
    // 目标设备：显式指定优先，否则主会话（必须已有运行中的显示会话）。
    let (target, options) = running_session_options(store, serial.clone())?;
    // X10-103：桌面模式下，录制通道要 `--display-id` 捕获显示通道那块虚拟屏——
    // 从显示进程的 scrcpy 输出实时解析（含重试）。非桌面模式不需要，置 None。
    let desktop_display_id = if options.desktop_mode {
        resolve_desktop_display_id(store, &target)
    } else {
        None
    };
    // 录制是 Pro 功能（与既有 ensure_edition_allows 同一闸门）。
    let record_opts = SessionOptions { record: true, ..options.clone() };
    ensure_edition_allows(app, &record_opts)?;
    let record_path = prepare_recording_path(app, true, file_name.as_deref())?
        .ok_or_else(|| AppError::new("recording_path", "无法准备录制文件路径。", "请重试。"))?;

    let process = runtimes
        .mirror
        .start_recorder(&target, &options, &record_path, desktop_display_id)
        .map_err(|error| {
            // 动态文案不能用 AppError::new（要 &'static str）：先记日志，用静态文案。
            eprintln!("[mirrordock] start_recorder failed: {error}");
            AppError::new(
                "recording_start_failed",
                "无法开始屏幕录制。",
                "手机性能不足以同时编码两路画面时会出现此错误；请降低镜像画质后重试，或改用数据线连接。",
            )
        })?;

    let path_string = record_path.to_string_lossy().into_owned();
    let file = Path::new(&path_string)
        .file_name()
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_else(|| path_string.clone());
    {
        let mut map = store.lock()?;
        let state = session_entry_mut(&mut map, &target);
        // 若残留旧录制进程（异常路径），先停掉再换新的，避免泄漏。
        if let Some(mut old) = state.record_process.take() {
            let _ = old.stop();
        }
        state.record_path = Some(path_string.clone());
        state.record_process = Some(process);
    }
    Ok(Recording { file_name: file, path: path_string, active: true })
}

/// X10-92：结束录制（双通道）。优雅停掉独立录制进程（写 moov 索引定型 MP4），
/// 显示会话不受影响。返回定型的录制信息（active=false）。
#[tauri::command]
fn stop_recording(
    app: AppHandle,
    sessions: State<SessionStore>,
    serial: Option<String>,
) -> Result<Option<Recording>, AppError> {
    let result = stop_recording_with(&sessions, serial.as_deref())?;
    if result.is_some() {
        // X10-95：录制状态跃迁后刷新托盘菜单（结束→「开始屏幕录制」）。
        refresh_tray_menu(&app);
        // 录制状态跃迁：推给活跃伴侣会话（M4-3）。
        notify_companion_recording(&app, false);
    }
    Ok(result)
}

fn stop_recording_with(
    store: &SessionStore,
    serial: Option<&str>,
) -> Result<Option<Recording>, AppError> {
    let mut map = store.lock()?;
    // 定位要停止的会话：显式 serial 优先，否则任意仍在录制的设备，否则主会话。
    let target = serial
        .map(|s| s.to_owned())
        .or_else(|| {
            map.iter()
                .find(|(_, state)| state.record_process.is_some())
                .map(|(serial, _)| serial.clone())
        })
        .or_else(|| primary_session_serial(&map));
    let Some(target) = target else {
        return Ok(None);
    };
    let Some(state) = map.get_mut(&target) else {
        return Ok(None);
    };
    let Some(mut process) = state.record_process.take() else {
        return Ok(None); // 没在录制：幂等返回，不报错。
    };
    // 优雅退出写 moov；失败也只记录，不阻断（文件可能已坏，但状态必须清）。
    let _ = process.stop();
    let path = state.record_path.clone().unwrap_or_default();
    // X10-99：结束后**核验产物真实存在且非空**再报「已保存」。录制进程可能根本没
    // 写出文件（例如桌面模式下 --start-app 与 --no-control 互斥导致 scrcpy 立即退出、
    // 一字节未写），若不核验，界面会弹出「已结束录制…保存在文件夹」的假成功——这正是
    // 「提示都有，但文件夹没有视频」的直接原因。文件缺失/为空时如实报错，不谎报。
    let produced = Path::new(&path)
        .metadata()
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false);
    if !produced {
        // 状态必须与磁盘一致：没产出文件就清掉 record_path，别让「当前录像」还指向
        // 一个根本不存在的文件。
        state.record_path = None;
        return Err(AppError::new(
            "recording_file_missing",
            "录制没有产出视频文件。",
            "桌面（虚拟屏）模式下请确认镜像窗口正常；可改录手机屏幕或降低画质后重试。",
        ));
    }
    let file = Path::new(&path)
        .file_name()
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    Ok(Some(Recording { file_name: file, path, active: false }))
}

fn current_recording_with(store: &SessionStore) -> Result<Option<Recording>, AppError> {
    let map = store.lock()?;
    let primary = primary_session_serial(&map);
    // X10-92：「正在录制」由独立录制进程 record_process 判定（双通道），与显示进程无关。
    // 主会话持有录制文件时用它；否则取任意一台仍在录制的设备（多会话并存时
    // 「当前录像」展示最先按序列号排序的进行中录制）。
    let state = primary
        .as_deref()
        .and_then(|serial| map.get(serial))
        .filter(|state| state.record_path.is_some())
        .or_else(|| {
            map.values()
                .find(|state| state.record_path.is_some() && state.record_process.is_some())
        })
        .or_else(|| map.values().find(|state| state.record_path.is_some()));
    let Some(state) = state else {
        return Ok(None);
    };
    let Some(path) = state.record_path.clone() else {
        return Ok(None);
    };
    let file_name = Path::new(&path)
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    Ok(Some(Recording {
        file_name,
        path,
        active: state.record_process.is_some(),
    }))
}

/// 是否存在「真正正在录制」的会话：录制进程仍在运行（X10-92 双通道）。
///
/// 与 [`current_recording_with`] 的展示语义不同：那条路径会回退到已结束的录制条目
/// （界面要能展示「最近一次录像」）；伴侣通知的状态判定必须精确——录制进程没了，录制
/// 就已经结束，绝不能把历史录制条目当成「仍在录制」（M4-3 通知跃迁判定）。
fn any_session_recording(store: &SessionStore) -> bool {
    store
        .lock()
        .map(|map| map.values().any(|state| state.record_process.is_some()))
        .unwrap_or(false)
}

/// 删除一个录像文件，对应界面上的「撤销」。
///
/// **正在录制的文件会被拒绝删除**：删掉它既会让用户以为已经清理干净、实际却还在写，
/// 也可能破坏正在进行中的文件。这里如实报错并指出下一步，而不是静默失败。
/// X10-27 后扫描**所有**设备的会话——任意一台在录这个文件都拒绝。
fn remove_recording_file(
    directory: &Path,
    name: &str,
    store: &SessionStore,
) -> Result<(), AppError> {
    let path = directory.join(name);
    {
        let map = store.lock()?;
        let in_progress = map.values().any(|state| {
            state.record_process.is_some()
                && state.record_path.as_deref() == Some(path.to_string_lossy().as_ref())
        });
        if in_progress {
            return Err(AppError::new(
                "recording_in_progress",
                "这段录像仍在录制中，无法删除。",
                "请先结束镜像会话，录像结束后即可删除。",
            ));
        }
    }
    if !path.is_file() {
        return Err(AppError::new(
            "recording_missing",
            "这个录像文件已经不在了。",
            "它可能已被移动或删除，请刷新后重试。",
        ));
    }
    fs::remove_file(&path).map_err(|_| {
        AppError::new(
            "recording_delete_failed",
            "无法删除这个录像文件。",
            "请在本机的文件管理器中手动删除它。",
        )
    })
}

// ---------------------------------------------------------------------------
// scrcpy 运行时定位与解析
// ---------------------------------------------------------------------------

fn select_scrcpy_binary(
    explicit_path: Option<PathBuf>,
    development_path: &Path,
    exe_dir: Option<&Path>,
) -> PathBuf {
    if let Some(path) = explicit_path.filter(|path| path.is_file()) {
        return path;
    }

    if cfg!(debug_assertions) && development_path.is_file() {
        return development_path.to_path_buf();
    }

    // 发行模式优先使用随包分发的 scrcpy（A1-08），找不到再退回 PATH。
    if let Some(exe_dir) = exe_dir {
        let binary = if cfg!(windows) { "scrcpy.exe" } else { "scrcpy" };
        if let Some(found) = bundled_binary(&bundled_runtime_dir_candidates(exe_dir), "scrcpy", binary)
        {
            return found;
        }
    }

    PathBuf::from("scrcpy")
}

fn scrcpy_binary() -> PathBuf {
    let explicit_path = std::env::var_os("MIRRORDOCK_SCRCPY_PATH").map(PathBuf::from);
    let development_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tools/scrcpy/macos-x86_64/scrcpy");
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    select_scrcpy_binary(explicit_path, &development_path, exe_dir.as_deref())
}

/// Tauri 把 `bundle.resources` 按相对路径结构放进资源目录，位置因平台而异：
/// Windows / Linux 就在可执行文件旁，macOS 在 `Contents/Resources` 下，
/// deb 在 `/usr/lib/<identifier>`。这里列出所有候选资源根，按顺序探测。
fn bundled_runtime_dir_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![exe_dir.join("resources")];
    if let Some(parent) = exe_dir.parent() {
        candidates.push(parent.join("Resources").join("resources"));
        candidates.push(parent.join("Resources").join("_up_").join("resources"));
        candidates.push(parent.join("lib").join("com.mirrordock.desktop").join("resources"));
    }
    candidates
}

/// 在候选资源根下查找 `<sub>/<binary>`，返回第一个真实存在的文件。
fn bundled_binary(resource_roots: &[PathBuf], sub: &str, binary: &str) -> Option<PathBuf> {
    resource_roots
        .iter()
        .map(|root| root.join(sub).join(binary))
        .find(|path| path.is_file())
}

/// 镜像窗口的 Dock 图标包装（仅 macOS 生效）。
///
/// 直接运行 scrcpy 裸二进制时，SDL 会把 scrcpy 自带的绿色安卓机器人图标挂到
/// Dock 上（用户反馈：与主客户端风格割裂）。把 scrcpy 包进一个带
/// `CFBundleIconFile` 的最小 `.app` bundle 再启动，LaunchServices 会按 bundle
/// 注册，Dock 显示「MirrorDock 镜像」与主客户端同款图标。
///
/// bundle 放在 scrcpy 同目录下（与真实文件同卷，内容直接复制）；任何一步失败
/// 都返回 `None`，调用方回退为直接启动 scrcpy 原始二进制——包装只是外观增强，
/// 绝不能挡住镜像本身。
fn macos_mirror_bundle_exec(scrcpy: &Path, adb: &Path) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let dir = scrcpy.parent()?;
    let bundle = dir.join("MirrorDock Mirror.app");
    let contents = bundle.join("Contents");
    let macos_dir = contents.join("MacOS");
    let resources_dir = contents.join("Resources");
    std::fs::create_dir_all(&macos_dir).ok()?;
    std::fs::create_dir_all(&resources_dir).ok()?;
    mirror_copy(scrcpy, &macos_dir.join("scrcpy"))?;
    if let Some(server) = Some(dir.join("scrcpy-server")).filter(|path| path.is_file()) {
        let _ = mirror_copy(&server, &macos_dir.join("scrcpy-server"));
    }
    // adb 也放进 bundle：scrcpy 启动时会在自己的目录里找 adb。
    let _ = mirror_copy(adb, &macos_dir.join("adb"));
    // X10-79：图标资源也必须跟进去。scrcpy 启动时会在**自身所在目录**找
    // `scrcpy.png` 设置窗口/Dock 图标，缺了它就在 stderr 打
    // 「ERROR: Could not open icon image / Could not load icon」。
    // 此前每次重建 bundle 都没复制这两个文件，于是每次启动镜像都刷这两行错误，
    // 混在真正的故障信息里干扰排查（真机实测确认）。
    for image in ["scrcpy.png", "disconnected.png"] {
        if let Some(source) = Some(dir.join(image)).filter(|path| path.is_file()) {
            let _ = mirror_copy(&source, &macos_dir.join(image));
        }
    }
    if let Some(icon) = app_icon_icns() {
        let _ = mirror_copy(&icon, &resources_dir.join("AppIcon.icns"));
    }
    std::fs::write(contents.join("Info.plist"), mirror_bundle_plist()).ok()?;
    #[cfg(target_os = "macos")]
    {
        // 临时签名（ad-hoc）：保证 LaunchServices 干净地接受这个 bundle；失败不致命。
        let _ = std::process::Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(&bundle)
            .output();
    }
    let _ = bundle;
    Some(macos_dir.join("scrcpy"))
}

/// Info.plist 内容：名字显示为「MirrorDock 镜像」，图标取 Resources/AppIcon.icns。
fn mirror_bundle_plist() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>scrcpy</string>
  <key>CFBundleIdentifier</key><string>com.mirrordock.mirror</string>
  <key>CFBundleName</key><string>MirrorDock 镜像</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleShortVersionString</key><string>0.2.1</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
    .into()
}

/// 复制单个文件到 bundle（内容始终以源为准）；失败返回 None 让上层回退。
fn mirror_copy(src: &Path, dst: &Path) -> Option<()> {
    if !src.is_file() {
        return None;
    }
    let _ = std::fs::remove_file(dst);
    std::fs::copy(src, dst).ok().map(|_| ())
}

/// 主客户端的应用图标（icns）：发行包里在 Contents/Resources/icon.icns，
/// 开发模式退回仓库内 src-tauri/icons/icon.icns。
fn app_icon_icns() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        // <...>/MirrorDock.app/Contents/MacOS/mirrordock → Contents/Resources
        if let Some(contents) = exe.parent().and_then(|macos| macos.parent()) {
            let icon = contents.join("Resources").join("icon.icns");
            if icon.is_file() {
                return Some(icon);
            }
        }
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons").join("icon.icns");
    dev.is_file().then_some(dev)
}

fn adb_binary() -> PathBuf {
    let explicit_path = std::env::var_os("MIRRORDOCK_ADB").map(PathBuf::from);
    let development_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tools/scrcpy/adb");
    if let Some(path) = explicit_path.filter(|path| path.is_file()) {
        return path;
    }
    if cfg!(debug_assertions) && development_path.is_file() {
        return development_path.to_path_buf();
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let roots = bundled_runtime_dir_candidates(exe_dir);
            // Windows 的 adb.exe 随 scrcpy 官方包分发；macOS / Linux 用 platform-tools。
            let relative: &[&str] = if cfg!(windows) {
                &["scrcpy/adb.exe", "platform-tools/adb.exe"]
            } else {
                &["platform-tools/adb", "scrcpy/adb"]
            };
            for name in relative {
                if let Some(found) = roots
                    .iter()
                    .map(|root| root.join(name))
                    .find(|path| path.is_file())
                {
                    return found;
                }
            }
        }
    }
    PathBuf::from("adb")
}

fn is_scrcpy_available() -> bool {
    quiet_command(scrcpy_binary())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// 无线设备的 adb 序列号有两类：
/// 1. `IP:端口`（含冒号）；
/// 2. 无线调试的 mDNS 发现条目，形如 `adb-<id>-<name>._adb-tls-connect._tcp`——
///    不含冒号但也不是 USB，漏判会把纯 Wi-Fi 设备误标成「USB + 无线」。
///
/// USB 序列号是纯硬件号（如 `79j7kn9tkjt8rwss`），不含冒号、点号与 `_tcp`。
/// 与前端 `looksLikeWirelessEndpoint` 保持同一判定，仅用于通道归类与界面提示。
fn is_wireless_endpoint(serial: &str) -> bool {
    serial.contains(':') || serial.contains("._adb-tls") || serial.contains("._tcp")
}

fn parse_adb_devices(output: &str) -> Vec<AdbDevice> {
    output
        .lines()
        .skip_while(|line| !line.starts_with("List of devices attached"))
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let raw_state = fields.next()?;
            let state = match raw_state {
                "device" => DeviceState::Ready,
                "unauthorized" => DeviceState::Unauthorized,
                "offline" => DeviceState::Offline,
                _ => DeviceState::Unknown,
            };
            let model = fields
                .find_map(|field| field.strip_prefix("model:"))
                .map(|model| model.replace('_', " "));
            let kind = if is_wireless_endpoint(serial) {
                ConnectionKind::Wireless
            } else {
                ConnectionKind::Usb
            };
            Some(AdbDevice {
                serial: serial.to_owned(),
                label: model.unwrap_or_else(|| "Android 设备".to_owned()),
                state,
                // 硬件序列号需对已授权设备额外查询，解析阶段尚不可知。
                physical_serial: None,
                connections: vec![ConnectionEndpoint {
                    serial: serial.to_owned(),
                    kind,
                    state,
                }],
            })
        })
        .collect()
}

/// 通道/状态优先级：状态越「可用」排名越高；并列时 USB 优先于无线。
fn state_rank(state: DeviceState) -> u8 {
    match state {
        DeviceState::Ready => 3,
        DeviceState::Unauthorized => 2,
        DeviceState::Offline => 1,
        DeviceState::Unknown => 0,
    }
}

fn device_has_usb(device: &AdbDevice) -> bool {
    device
        .connections
        .iter()
        .any(|connection| connection.kind == ConnectionKind::Usb)
}

/// 合并组内选首选通道的优先级：USB > 无线 `IP:端口` > mDNS 发现名。
/// mDNS 条目（`adb-…._adb-tls-connect._tcp`）虽是有效 transport，但名字不稳定、
/// 不宜作为发起镜像的端点；`IP:端口` 是无线调试的常规稳定端点。
fn endpoint_preference(device: &AdbDevice) -> u8 {
    if device_has_usb(device) {
        return 2;
    }
    if device.serial.contains(':') {
        return 1;
    }
    0
}

/// 将同一物理设备（相同 `physical_serial`）的多条 adb 通道合并为一条设备记录。
///
/// 关键约束：**只有「已就绪（`Ready`）且已知硬件序列号」的通道才参与合并**。
/// 原因有二：
/// 1. 未授权/离线/读不到序列号的设备无法稳定查询 `ro.serialno`，强行合并会把
///    同型号的不同设备误并成一台；
/// 2. adb 拔除 USB 后可能残留陈旧条目（ghost），这类条目若仍被当成有效通道，
///    会让「只用 Wi-Fi 连接」的设备错误显示「USB + 无线」。陈旧/offline 条目
///    各自独立成行（界面标为「离线」），绝不污染已就绪设备的通道徽标。
///
/// 合并后：
/// - `serial` 取自首选通道（USB 优先），即发起镜像/唤醒所用的端点；
/// - `state` 取所有通道里最好的状态（任一通道就绪即可镜像）；
/// - `connections` 仅含已就绪通道，供界面提示「USB + 无线」。
fn dedup_devices(devices: Vec<AdbDevice>) -> Vec<AdbDevice> {
    let mut merged_groups: Vec<(String, Vec<AdbDevice>)> = Vec::new();
    let mut passthrough: Vec<AdbDevice> = Vec::new();

    for device in devices {
        // 仅已就绪且已知硬件序列号的通道参与合并；其余保持独立成行。
        if device.state == DeviceState::Ready {
            if let Some(psn) = device
                .physical_serial
                .clone()
                .filter(|value| !value.is_empty())
            {
                match merged_groups
                    .iter_mut()
                    .find(|(key, _)| *key == psn)
                {
                    Some((_, group)) => group.push(device),
                    None => merged_groups.push((psn, vec![device])),
                }
                continue;
            }
        }
        passthrough.push(device);
    }

    // 影子端点归并（X10-32）：非就绪/未知归属的条目若能判定属于某个就绪组
    // （同一台物理手机），不再单独成卡——用户看到的每台手机只有一张卡。
    // 判定依据（按可靠度）：① 硬件序列号相同；② mDNS 实例名内含该组 USB 序列号
    // （实例名构成为 `adb-<序列号>-<随机串>`）；③ 无线端点 IP 相同（换端口重连的
    // 残留）。三者都判不了才保留独立卡片——离线状态必须如实可见，不能瞎猜合并。
    let group_keys: Vec<(String, Vec<String>, Vec<String>)> = merged_groups
        .iter()
        .map(|(psn, group)| (psn.clone(), group_usb_serials(group), group_wireless_ips(group)))
        .collect();
    let shadows: Vec<AdbDevice> = passthrough
        .into_iter()
        .filter(|device| {
            !group_keys
                .iter()
                .any(|(psn, usb_serials, wireless_ips)| device_shadows_group(device, psn, usb_serials, wireless_ips))
        })
        .collect();
    // 整机离线时的残影之间也归并：USB 残影 + 它的 mDNS 残影 → 一张卡。
    let mut result: Vec<AdbDevice> = merge_offline_shadows(shadows);
    for (_, group) in merged_groups {
        if group.len() == 1 {
            result.push(group.into_iter().next().unwrap());
            continue;
        }
        // 选首选通道：USB 优先，其次无线 `IP:端口`，最后 mDNS 发现名（均为 Ready）。
        let primary = group
            .iter()
            .max_by(|a, b| endpoint_preference(a).cmp(&endpoint_preference(b)))
            .unwrap()
            .clone();
        let state = group
            .iter()
            .map(|device| device.state)
            .max_by_key(|state| state_rank(*state))
            .unwrap();
        let connections = group
            .iter()
            .flat_map(|device| device.connections.clone())
            .collect();
        result.push(AdbDevice {
            serial: primary.serial,
            label: primary.label,
            state,
            physical_serial: primary.physical_serial.clone(),
            connections,
        });
    }
    result
}

/// 从无线端点提取 IP 部分（`192.168.2.90:41901` → `192.168.2.90`）。
/// 仅接受 `IPv4:端口` 形式；mDNS 发现名（不含冒号）与其它形式返回 None。
/// IPv6 暂不参与按 IP 归并——adb 无线调试常规广播是 IPv4，够用且不误伤。
fn wireless_ip_endpoint(serial: &str) -> Option<&str> {
    let (ip, port) = serial.rsplit_once(':')?;
    if ip.is_empty()
        || ip.contains(':')
        || !ip.contains('.')
        || port.is_empty()
        || !port.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some(ip)
}

/// mDNS 实例名（`adb-<USB序列号>-<随机串>._adb-tls-connect._tcp`）是否携带
/// 某台设备的 USB 序列号。直接子串匹配：序列号设长度下限，避免误配。
fn mdns_serial_contains_usb(mdns_serial: &str, usb_serial: &str) -> bool {
    usb_serial.len() >= 6 && mdns_serial.contains(usb_serial)
}

/// 一个就绪组（同一物理手机）的全部 USB 序列号，用于和 mDNS 实例名互认。
fn group_usb_serials(group: &[AdbDevice]) -> Vec<String> {
    let mut out = Vec::new();
    for device in group {
        let serials = std::iter::once(&device.serial)
            .chain(device.connections.iter().map(|c| &c.serial));
        for serial in serials {
            if !is_wireless_endpoint(serial) && !out.contains(serial) {
                out.push(serial.clone());
            }
        }
    }
    out
}

/// 一个就绪组的全部无线端点 IP（去重）。同一 IP 换端口 = 同一台手机重连残留。
fn group_wireless_ips(group: &[AdbDevice]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for device in group {
        let serials = std::iter::once(&device.serial)
            .chain(device.connections.iter().map(|c| &c.serial));
        for serial in serials {
            if let Some(ip) = wireless_ip_endpoint(serial) {
                if !out.iter().any(|known| known == ip) {
                    out.push(ip.to_owned());
                }
            }
        }
    }
    out
}

/// 判断一个未参与合并的条目是否属于某个就绪组（同一台物理手机）。
fn device_shadows_group(
    device: &AdbDevice,
    psn: &str,
    usb_serials: &[String],
    wireless_ips: &[String],
) -> bool {
    if device.physical_serial.as_deref() == Some(psn) {
        return true;
    }
    let serials = std::iter::once(&device.serial)
        .chain(device.connections.iter().map(|c| &c.serial));
    for serial in serials {
        // mDNS 发现名（无线、不含冒号）内含组内 USB 序列号 → 同一台手机。
        if is_wireless_endpoint(serial)
            && !serial.contains(':')
            && usb_serials
                .iter()
                .any(|usb| mdns_serial_contains_usb(serial, usb))
        {
            return true;
        }
        if let Some(ip) = wireless_ip_endpoint(serial) {
            if wireless_ips.iter().any(|known| known == ip) {
                return true;
            }
        }
    }
    false
}

/// 都进不了就绪组的残影之间互认：mDNS 残影内含某张残影卡的 USB 序列号时并入它
/// （整台手机离线时通常剩 USB 残影 + mDNS 残影两条，合并后只出一张离线卡）。
fn merge_offline_shadows(shadows: Vec<AdbDevice>) -> Vec<AdbDevice> {
    let mut result: Vec<AdbDevice> = Vec::new();
    for device in shadows {
        let is_mdns = is_wireless_endpoint(&device.serial) && !device.serial.contains(':');
        let target = if is_mdns {
            result.iter_mut().find(|candidate| {
                !is_wireless_endpoint(&candidate.serial)
                    && mdns_serial_contains_usb(&device.serial, &candidate.serial)
            })
        } else {
            None
        };
        match target {
            Some(candidate) => candidate.connections.extend(device.connections),
            None => result.push(device),
        }
    }
    result
}

fn endpoint_is_ready(devices: &[AdbDevice], endpoint: &str) -> bool {
    devices.iter().any(|device| {
        // 合并后的设备其 `serial` 是首选通道；仍要匹配任一 `connections` 里的原始端点，
        // 否则用无线端点调用时会被判为未就绪。
        (device.serial == endpoint
            || device
                .connections
                .iter()
                .any(|connection| connection.serial == endpoint))
            && device.state == DeviceState::Ready
    })
}

// ---------------------------------------------------------------------------
// 设备能力探测
//
// 目的：在启动会话**之前**就把"这台手机能做什么、不能做什么"讲清楚。
// 受保护内容黑屏、系统音频不可捕获、OEM 差异都是正确的平台行为，必须向
// 非技术用户解释，而不是等到失败后再猜。
// ---------------------------------------------------------------------------

/// 需要从设备读取的属性白名单。不在此列表中的属性既不进内存，也不进界面。
const WANTED_PROPERTIES: [&str; 5] = [
    "ro.build.version.release",
    "ro.build.version.sdk",
    "ro.product.manufacturer",
    "ro.product.brand",
    "ro.product.model",
];

/// 受限能力说明的等级。`Info` 表示"能力可用"，`Limitation` 表示"存在边界"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum NoticeLevel {
    Info,
    Limitation,
}

/// 一条面向用户的受限能力说明：标题 + 原因/影响/应对。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CapabilityNotice {
    /// 稳定标识，前端据此决定图标与分组；修改文案时不要改动它。
    code: &'static str,
    level: NoticeLevel,
    title: String,
    detail: String,
}

impl CapabilityNotice {
    fn limitation(code: &'static str, title: &str, detail: String) -> Self {
        Self {
            code,
            level: NoticeLevel::Limitation,
            title: title.to_owned(),
            detail,
        }
    }

    fn info(code: &'static str, title: &str, detail: String) -> Self {
        Self {
            code,
            level: NoticeLevel::Info,
            title: title.to_owned(),
            detail,
        }
    }
}

/// 设备在启动会话前被探测到的能力。
///
/// `mirroring_supported` 与 `audio_forwarding_supported` 使用 `Option<bool>`：
/// 读不到系统版本时保持 `None`（未知），**不得**默认成"支持"或"不支持"。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DeviceCapabilities {
    serial: String,
    label: String,
    android_release: Option<String>,
    sdk: Option<u32>,
    mirroring_supported: Option<bool>,
    audio_forwarding_supported: Option<bool>,
    notices: Vec<CapabilityNotice>,
}

/// 清洗来自设备的属性文本：去首尾空白、剔除控制字符、限制长度。
fn sanitize_property(value: &str) -> Option<String> {
    let cleaned: String = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_PROPERTY_LEN)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_owned())
    }
}

/// 解析 `adb shell getprop` 的输出。每行形如 `[key]: [value]`。
fn parse_device_properties(output: &str) -> BTreeMap<&'static str, String> {
    let mut properties = BTreeMap::new();
    for line in output.lines().take(MAX_PROPERTY_LINES) {
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((key, rest)) = rest.split_once("]: [") else {
            continue;
        };
        let Some(value) = rest.strip_suffix(']') else {
            continue;
        };
        let Some(wanted) = WANTED_PROPERTIES.iter().find(|wanted| **wanted == key) else {
            continue;
        };
        if let Some(cleaned) = sanitize_property(value) {
            properties.entry(*wanted).or_insert(cleaned);
        }
    }
    properties
}

/// 由设备属性推导能力判定与受限说明。
fn capabilities_from_properties(
    serial: String,
    properties: &BTreeMap<&'static str, String>,
) -> DeviceCapabilities {
    let sdk = properties
        .get("ro.build.version.sdk")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|sdk| (1..=1000).contains(sdk));

    let android_release = properties.get("ro.build.version.release").cloned();
    let label = match (
        properties.get("ro.product.manufacturer"),
        properties.get("ro.product.model"),
    ) {
        (Some(manufacturer), Some(model)) => format!("{manufacturer} {model}"),
        (None, Some(model)) => model.clone(),
        (Some(manufacturer), None) => manufacturer.clone(),
        (None, None) => properties
            .get("ro.product.brand")
            .cloned()
            .unwrap_or_else(|| "Android 设备".to_owned()),
    };

    DeviceCapabilities {
        serial,
        label,
        android_release,
        sdk,
        mirroring_supported: sdk.map(|sdk| sdk >= MIN_SDK_FOR_MIRRORING),
        audio_forwarding_supported: sdk.map(|sdk| sdk >= MIN_SDK_FOR_AUDIO),
        notices: capability_notices(sdk),
    }
}

/// 生成受限能力说明。结论只依赖系统版本，不依赖设备提供的其它字段。
fn capability_notices(sdk: Option<u32>) -> Vec<CapabilityNotice> {
    let mut notices = Vec::new();

    match sdk {
        Some(sdk) if sdk < MIN_SDK_FOR_MIRRORING => notices.push(CapabilityNotice::limitation(
            "android_too_old",
            "系统版本可能过低",
            format!(
                "这台手机的系统等级为 API {sdk}，低于 MirrorDock 支持的 Android 8（API 26）。镜像可能无法启动或运行不稳定。"
            ),
        )),
        Some(sdk) if sdk < MIN_SDK_FOR_AUDIO => notices.push(CapabilityNotice::limitation(
            "audio_forwarding_unavailable",
            "不支持把手机声音传到电脑",
            "Android 11 以下无法转发系统音频。画面与鼠标键盘控制不受影响，只是电脑上不会有手机的声音。".to_owned(),
        )),
        Some(_) => notices.push(CapabilityNotice::info(
            "audio_forwarding_available",
            "可以把手机声音传到电脑",
            "这台手机运行 Android 11 及以上，系统声音会一起在电脑上播放；被应用单独禁止捕获的声音除外。"
                .to_owned(),
        )),
        None => notices.push(CapabilityNotice::limitation(
            "android_version_unknown",
            "无法确认系统版本",
            "这台手机没有返回系统版本信息，声音等能力无法提前判断。可以直接尝试开始镜像，若缺少声音再检查手机的开发者选项。"
                .to_owned(),
        )),
    }

    notices.push(CapabilityNotice::limitation(
        "protected_content",
        "部分页面会显示黑屏",
        "银行、支付和部分视频应用会主动禁止被镜像。这是 Android 的安全策略，MirrorDock 不会也无法绕过。"
            .to_owned(),
    ));
    notices.push(CapabilityNotice::limitation(
        "input_restricted_by_apps",
        "部分应用会屏蔽电脑的点击",
        "少数应用会忽略由电脑发来的点击与按键。这属于应用自身的安全限制，改用手机会恢复正常。"
            .to_owned(),
    ));
    notices.push(CapabilityNotice::limitation(
        "oem_differences",
        "不同品牌的开发者选项位置不同",
        "各品牌的开发者选项名称与入口略有差异。如果找不到“无线调试”，请先开启“USB 调试”，或用数据线完成第一次连接。"
            .to_owned(),
    ));

    notices
}

/// 探测指定设备的能力。只读取系统属性，不启动镜像、不改变会话状态。
fn probe_device_capabilities_with(
    runtimes: &AppRuntimes,
    serial: String,
) -> Result<DeviceCapabilities, AppError> {
    let serial = validate_serial(&serial)?;

    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }

    let output = runtimes.adb.device_properties(&serial).map_err(|error| {
        adb_command_error(
            error,
            "probe_failed",
            "无法读取这台手机的能力信息。",
            "请重新检查连接后重试；如果仍然失败，可以直接尝试开始镜像。",
        )
    })?;

    Ok(capabilities_from_properties(
        serial,
        &parse_device_properties(&output),
    ))
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

#[tauri::command]
async fn check_adb_devices(
    runtimes: State<'_, AppRuntimes>,
    sessions: State<'_, SessionStore>,
    log: State<'_, DiagnosticsLog>,
    // 轮询调用传 Some(true)：只回传设备状态，不写诊断日志，避免每几秒刷爆诊断缓冲。
    silent: Option<bool>,
) -> Result<AdbCheck, AppError> {
    // X10-89：改 async 后命令体跑在 tauri 工作线程，主线程/IPC 立即释放。
    // 内部 adb 调用（已带 ADB_CALL_TIMEOUT 子线程超时）即使慢，也只占工作线程，
    // 不再冻结 UI（0.4.15 把同步忙等放主线程导致 Mac 启动卡死的回归修复）。
    Ok(check_adb_devices_sync(&runtimes, &sessions, &log, silent))
}

/// `check_adb_devices` 的同步实现，抽出来供 async 命令与内部复用。
fn check_adb_devices_sync(
    runtimes: &AppRuntimes,
    sessions: &SessionStore,
    log: &DiagnosticsLog,
    silent: Option<bool>,
) -> AdbCheck {
    let scrcpy_available = runtimes.mirror.is_available();
    let mut devices = match runtimes.adb.list_devices() {
        Ok(devices) => devices,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return AdbCheck {
                adb_available: false,
                scrcpy_available,
                devices: Vec::new(),
                diagnostic: Some("未找到 Android 平台工具。请重新安装 MirrorDock 或联系支持人员。".into()),
            };
        }
        Err(_) => {
            return AdbCheck {
                adb_available: true,
                scrcpy_available,
                devices: Vec::new(),
                diagnostic: Some(
                    "Android 调试服务暂时不可用。请拔下数据线后重新连接，再试一次。".to_owned(),
                ),
            };
        }
    };

    // 为已授权设备补全硬件序列号（用于跨 USB/无线去重）。命中缓存则跳过一次 shell 调用。
    for device in &mut devices {
        if device.state == DeviceState::Ready && device.physical_serial.is_none() {
            let cached = runtimes
                .serial_cache
                .lock()
                .unwrap()
                .get(&device.serial)
                .cloned();
            let physical = match cached {
                Some(value) => Some(value),
                None => runtimes.adb.physical_serial(&device.serial).ok().flatten(),
            };
            if let Some(value) = physical.filter(|value| !value.is_empty()) {
                runtimes
                    .serial_cache
                    .lock()
                    .unwrap()
                    .insert(device.serial.clone(), value.clone());
                device.physical_serial = Some(value);
            }
        }
    }
    // 清理缓存中已消失的序列号，避免陈旧映射把新设备误判成旧设备。
    let present: HashSet<String> = devices.iter().map(|device| device.serial.clone()).collect();
    runtimes
        .serial_cache
        .lock()
        .unwrap()
        .retain(|serial, _| present.contains(serial));
    // 会话表清理（X10-27）：设备已从 adb 消失、又没有进行中的会话时，移除其残条目
    // （Failed/Unauthorized/Offline 状态会随设备重插/重连重新建立）。注意 dedup 后的
    // serial 可能是组内首选通道，这里用原始 present + 各条目 serial 双重判断更稳妥。
    if let Ok(mut map) = sessions.lock() {
        let mut stale: Vec<String> = Vec::new();
        for (serial, state) in map.iter() {
            if !is_session_active(state) && !present.contains(serial) {
                stale.push(serial.clone());
            }
        }
        for serial in stale {
            map.remove(&serial);
        }
    }

    let devices = dedup_devices(devices);
    let check = AdbCheck {
        adb_available: true,
        scrcpy_available,
        devices,
        diagnostic: None,
    };
    // 诊断事件只记设备状态计数，不记序列号与型号。
    let mut by_state: BTreeMap<String, usize> = BTreeMap::new();
    for device in &check.devices {
        *by_state.entry(format!("{:?}", device.state)).or_insert(0) += 1;
    }
    let summary: Vec<String> = by_state
        .iter()
        .map(|(state, count)| format!("{}×{}", state, count))
        .collect();
    let detail = if check.adb_available {
        if summary.is_empty() {
            "未发现设备".to_owned()
        } else {
            format!("连接设备状态：{}", summary.join("、"))
        }
    } else {
        check.diagnostic.clone().unwrap_or_default()
    };
    // 轮询（silent=true）只刷新界面，不污染诊断日志；手动「重新检查」才记录。
    if silent != Some(true) {
        log.record(
            "device_check",
            if check.adb_available { "ok" } else { "adb_missing" },
            &detail,
            &[],
        );
    }
    check
}

// -- 产品内诊断包（B2-01）：显式同意、导出前可预览、默认脱敏 --

/// 诊断事件容量上限：只保留最近的事件，避免无限增长。
const DIAGNOSTICS_CAP: usize = 200;

/// 单条事件详情的长度上限。
const DIAGNOSTICS_DETAIL_CAP: usize = 400;

/// 一条脱敏后的诊断事件。`detail` 只允许来自用户可见文案或固定字符串；
/// 序列号、配对码、本地路径等敏感值由调用方声明进 `secrets`，入库前统一擦除。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DiagnosticsEvent {
    /// Unix 毫秒时间戳。
    timestamp_ms: u128,
    kind: String,
    code: String,
    detail: String,
}

/// 内存中的诊断事件环形日志。默认不落盘——只有用户预览并显式导出时才写成文件。
#[derive(Default)]
struct DiagnosticsLog(Mutex<VecDeque<DiagnosticsEvent>>);

impl DiagnosticsLog {
    fn record(&self, kind: &str, code: &str, detail: &str, secrets: &[&str]) {
        let mut detail = detail.to_owned();
        for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
            detail = detail.replace(secret, "[已脱敏]");
        }
        if detail.chars().count() > DIAGNOSTICS_DETAIL_CAP {
            detail = detail.chars().take(DIAGNOSTICS_DETAIL_CAP).collect();
        }
        let mut events = match self.0.lock() {
            Ok(events) => events,
            Err(poisoned) => poisoned.into_inner(),
        };
        if events.len() >= DIAGNOSTICS_CAP {
            events.pop_front();
        }
        events.push_back(DiagnosticsEvent {
            timestamp_ms: now_unix_ms(),
            kind: kind.to_owned(),
            code: code.to_owned(),
            detail,
        });
    }

    /// 按命令结果记录：成功记 `ok`，失败记录错误码与用户可见文案（不引入新信息源）。
    fn record_outcome(&self, kind: &str, error: Option<&AppError>, secrets: &[&str]) {
        match error {
            None => self.record(kind, "ok", "成功", secrets),
            Some(error) => self.record(
                kind,
                error.code,
                &format!("{}（修复建议：{}）", error.message, error.recovery),
                secrets,
            ),
        }
    }

    fn snapshot(&self) -> Vec<DiagnosticsEvent> {
        let events = match self.0.lock() {
            Ok(events) => events,
            Err(poisoned) => poisoned.into_inner(),
        };
        events.iter().cloned().collect()
    }
}

fn now_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// 诊断包预览：用户在导出前看到的全部内容，也就是导出文件的全部内容——不多不少。
#[derive(Debug, Serialize)]
struct DiagnosticsPreview {
    generated_at_ms: u128,
    app_version: String,
    system: String,
    scrcpy_available: bool,
    events: Vec<DiagnosticsEvent>,
}

/// 用给定事实组装预览。独立成函数便于测试断言「预览里没有机密字段」。
fn build_diagnostics_preview(
    app_version: String,
    system: String,
    scrcpy_available: bool,
    events: Vec<DiagnosticsEvent>,
) -> DiagnosticsPreview {
    DiagnosticsPreview {
        generated_at_ms: now_unix_ms(),
        app_version,
        system,
        scrcpy_available,
        events,
    }
}

/// 导出回执：写了哪个文件、多少条事件、多少字节。
#[derive(Debug, Serialize)]
struct DiagnosticsReceipt {
    path: String,
    events: usize,
    bytes: u64,
}

#[tauri::command]
fn probe_device_capabilities(
    runtimes: State<AppRuntimes>,
    serial: String,
) -> Result<DeviceCapabilities, AppError> {
    probe_device_capabilities_with(&runtimes, serial)
}

#[tauri::command]
fn start_mirroring(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    log: State<DiagnosticsLog>,
    serial: String,
    options: Option<SessionOptions>,
    // X10-92（方案甲）：开始镜像不再自动录，此参数保留仅为兼容旧前端调用，一律忽略。
    _record_file_name: Option<String>,
) -> Result<(), AppError> {
    let mut options = options.unwrap_or_default();
    // Pro 门控先于一切副作用：免费版请求录制时，在触碰设备之前就给出明确引导。
    ensure_edition_allows(&app, &options)?;
    // X10-92（方案甲）：设置项「启用屏幕录制功能」只作能力开关，**开始镜像不再自动录**。
    // 何时录、录多久完全由用户在工具页/托盘/快捷键手动触发（走独立录制通道，不重启
    // 显示窗口）。因此这里一律忽略 record 请求，显示进程永不带 `--record`。
    if options.record {
        options.record = false;
    }
    let result = start_mirroring_with(&runtimes, &sessions, serial.clone(), options, None);
    log.record_outcome("mirror_start", result.as_ref().err(), &[&serial]);
    result?;

    // 会话已启动：若宿主输入源不是 ABC 布局，临时切到 ABC，保证镜像窗口
    // 能正常打字（X10-38/X10-41）。幂等；失败静默，不阻断镜像。
    maybe_switch_host_input_source(&app);

    // 只有成功启动才记入最近设备；读取人类可读的名称失败时退回序列号。
    let serial = serial.trim().to_owned();
    let label = runtimes
        .adb
        .list_devices()
        .ok()
        .and_then(|devices| {
            devices
                .into_iter()
                .find(|device| device.serial == serial)
                .map(|device| device.label)
        })
        .unwrap_or_else(|| serial.clone());
    remember_recent_device(&app, &serial, &label);
    refresh_tray_menu(&app);
    Ok(())
}

fn start_mirroring_with(
    runtimes: &AppRuntimes,
    sessions: &SessionStore,
    serial: String,
    options: SessionOptions,
    record_path: Option<PathBuf>,
) -> Result<(), AppError> {
    options.arguments()?;

    let serial = validate_serial(&serial)?;

    reserve_session(sessions, MirrorSession::connecting(serial.clone()))?;

    launch_into_reserved_session(runtimes, sessions, serial, options, record_path)
}

/// 在**已被本次请求占用**的会话槽位（`Connecting`）上真正拉起镜像进程。
///
/// 调用前提：`reserve_session` 或等价的占位动作已经成功。这里不再做互斥检查，
/// 因此「首次启动」与「会话中应用新设置后的重启」可以共用同一条启动路径，
/// 也就共用同一套设备校验与错误码。
fn launch_into_reserved_session(
    runtimes: &AppRuntimes,
    sessions: &SessionStore,
    serial: String,
    options: SessionOptions,
    record_path: Option<PathBuf>,
) -> Result<(), AppError> {
    if !runtimes.mirror.is_available() {
        return Err(fail_session(
            sessions,
            Some(serial),
            AppError::new(
                "mirror_runtime_missing",
                "镜像引擎尚未安装完成。",
                "请重新安装 MirrorDock 后再试。",
            ),
        ));
    }

    let lookup = device_lookup(runtimes, &serial);
    if let Some(error) = device_readiness_error(lookup) {
        match lookup {
            DeviceLookup::Unauthorized => {
                mark_session(
                    sessions,
                    MirrorSession::unauthorized(serial.clone(), error.clone()),
                );
            }
            DeviceLookup::Offline => {
                mark_session(
                    sessions,
                    MirrorSession::offline(serial.clone(), error.clone()),
                );
            }
            _ => {
                mark_session(
                    sessions,
                    MirrorSession::failed(Some(serial.clone()), error.clone()),
                );
            }
        }
        return Err(error);
    }

    let process = match runtimes.mirror.start(&serial, &options, record_path.as_deref()) {
        Ok(process) => process,
        Err(_) => {
            return Err(fail_session(
                sessions,
                Some(serial),
                AppError::new(
                    "mirror_start_failed",
                    "无法启动镜像窗口。",
                    "请重新检查连接后再试。",
                ),
            ));
        }
    };

    let keep_awake = options.keep_awake;
    // X10-80：桌面模式 + 指定应用时，会话起来后核验一次「应用是否真的落到虚拟屏」。
    // scrcpy 只发启动意图，应用拒不渲染它管不着（真机实证：MIUI 上网易系游戏的
    // SDK 跳板启动后即被移出虚拟屏，窗口全白）——必须由我们补上这层核验。
    let desktop_app = options
        .desktop_mode
        .then(|| options.desktop_app.clone())
        .flatten();
    let epoch = attach_process(
        sessions,
        process,
        serial.clone(),
        options,
        record_path.map(|path| path.to_string_lossy().into_owned()),
    )?;
    spawn_session_monitor(sessions.clone(), epoch, serial.clone());
    if let Some(package) = desktop_app {
        spawn_desktop_app_landing_check(sessions.clone(), epoch, serial.clone(), package);
    }
    // 无线连接下 `--stay-awake` 无效（见亮屏补偿注释）：会话真正跑起来后才补偿，
    // 启动失败路径不会留下被延长却无人还原的熄屏时间。重启会话时备份已存在，
    // 这里幂等地重写延长值；关闭保持唤醒或换回 USB 时则顺势还原。
    if keep_awake && is_wireless_serial(&serial) {
        enable_wireless_keep_awake(runtimes.adb.as_ref(), sessions, &serial);
    } else {
        // 换回 USB 或关闭保持唤醒时，只还原**这台设备**的补偿；其它设备的会话
        // 各有各的账本，互不牵连。
        disable_wireless_keep_awake(runtimes.adb.as_ref(), sessions, &serial);
    }
    // 变暗（DIM）守护：stay_on 类保活拦得住熄屏、拦不住熄屏前的变暗阶段
    // （背光 5%、镜像黑帧），锁屏静置约 3 分钟必然进入。USB 与无线都会遇到
    // （真机实测），所以守护不限连接方式，只跟随 keep_awake 开关。
    if keep_awake {
        spawn_keep_awake_guard(sessions.clone(), epoch, serial);
    }
    Ok(())
}

/// 会话中应用新设置的结果。
///
/// 镜像窗口的形态（全屏、置顶、旋转、画质）由 scrcpy 进程在启动时确定，运行中无法
/// 改写。因此这里的语义是**「应用新设置 = 结束旧窗口 + 按新设置重新打开」**，而不是
/// 悄悄把界面上的选项标记为已生效。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SessionUpdate {
    /// 请求的设置是否已经真正生效（即镜像窗口是否已按新设置重启）。
    applied: bool,
    /// 未重启时的原因说明；已重启时为 `None`。
    note: Option<String>,
    /// 操作后的会话状态，供界面立即刷新而无需等待下一次轮询。
    session: MirrorSession,
}

/// 会话没有运行中的进程时统一的错误：说明「设置何时生效」，而不是只说“没有会话”。
fn session_not_running_error() -> AppError {
    AppError::new(
        "session_not_running",
        "当前没有正在运行的镜像会话。",
        "设置会在下次开始镜像时生效；如需立即应用，请先开始镜像。",
    )
}

/// 读取某台正在运行的会话所使用的设备与启动参数。
///
/// `serial` 为 `None` 时取主会话（真正持有镜像进程的设备优先）。只有真正持有
/// 运行中的进程时才算“会话进行中”：处于 `Connecting` 但尚未拿到进程、以及
/// `Paired` / `Failed` 等阶段都会返回可恢复的错误，而不是去 kill 一个不存在的进程。
fn running_session_options(
    store: &SessionStore,
    serial: Option<String>,
) -> Result<(String, SessionOptions), AppError> {
    let map = store.lock()?;
    let target = match serial {
        Some(serial) => {
            if map.get(&serial).is_none_or(|state| state.process.is_none()) {
                return Err(session_not_running_error());
            }
            serial
        }
        None => {
            let Some(serial) = primary_session_serial(&map) else {
                return Err(session_not_running_error());
            };
            let state = &map[&serial];
            if state.process.is_none() {
                return Err(session_not_running_error());
            }
            serial
        }
    };
    let state = &map[&target];
    Ok((target, state.options.clone()))
}

/// 某台设备的会话快照；表中没有该设备时返回 `Idle`。
fn device_session_snapshot(store: &SessionStore, serial: &str) -> Result<MirrorSession, AppError> {
    Ok(store
        .lock()?
        .get(serial)
        .map(|state| state.session.clone())
        .unwrap_or_else(MirrorSession::idle))
}

/// 主会话快照（向后兼容的单会话入口用）。
fn session_snapshot(store: &SessionStore) -> Result<MirrorSession, AppError> {
    let map = store.lock()?;
    match primary_session_serial(&map) {
        Some(serial) => Ok(map[&serial].session.clone()),
        None => Ok(MirrorSession::idle()),
    }
}

/// 全部设备的会话快照（X10-27：多设备并发），按序列号稳定排序。
fn all_session_snapshots(store: &SessionStore) -> Result<Vec<MirrorSession>, AppError> {
    Ok(store
        .lock()?
        .values()
        .map(|state| state.session.clone())
        .collect())
}

/// 为重启过程中的失败补充上下文：用户必须知道「新设置没生效，而且镜像已经关了」。
///
/// 这里刻意保留原始错误码与原因（可能是 `device_unauthorized`、`device_offline`、
/// `mirror_runtime_missing` 等具体状态），只改写恢复建议，避免把具体状态塌缩成一个
/// 笼统的“重启失败”。
fn with_restart_context(error: &AppError) -> AppError {
    AppError {
        code: error.code,
        message: error.message.clone(),
        recovery: format!("{} 本次修改未生效，镜像窗口已关闭。", error.recovery),
    }
}

/// 把已写入会话的错误替换为带重启上下文的版本，且**不改变会话阶段**。
fn annotate_session_error(store: &SessionStore, serial: &str, error: &AppError) {
    if let Ok(mut map) = store.0.lock() {
        if let Some(state) = map.get_mut(serial) {
            if state.session.error.is_some() {
                state.session.error = Some(error.clone());
            }
        }
    }
}

/// 会话进行中应用新设置：先结束旧窗口，再按新设置重新打开。
///
/// `serial` 指定目标设备；`None` 时作用于主会话（真正持有进程的设备优先）。
fn apply_session_options_with(
    runtimes: &AppRuntimes,
    sessions: &SessionStore,
    options: SessionOptions,
    record_path: Option<PathBuf>,
    serial: Option<String>,
) -> Result<SessionUpdate, AppError> {
    // 先校验参数，再触碰正在运行的会话：一个非法请求绝不能打断一次正常的镜像。
    options.arguments()?;

    let (serial, current) = running_session_options(sessions, serial)?;
    if current == options {
        return Ok(SessionUpdate {
            applied: false,
            note: Some("设置与当前会话一致，无需重启镜像窗口。".to_owned()),
            session: device_session_snapshot(sessions, &serial)?,
        });
    }

    let Some(mut previous) = begin_session_restart(sessions, serial.clone())? else {
        return Err(session_not_running_error());
    };

    // 优雅结束旧窗口：录制中的 MP4 需要 scrcpy 写出索引后才能播放。
    if previous.stop().is_err() {
        return Err(fail_session(
            sessions,
            Some(serial),
            AppError::new(
                "session_restart_failed",
                "无法结束旧的镜像窗口，新设置尚未应用。",
                "请手动关闭镜像窗口后重新开始镜像。",
            ),
        ));
    }

    if let Err(error) =
        launch_into_reserved_session(runtimes, sessions, serial.clone(), options, record_path)
    {
        let error = with_restart_context(&error);
        annotate_session_error(sessions, &serial, &error);
        return Err(error);
    }

    Ok(SessionUpdate {
        applied: true,
        note: None,
        session: device_session_snapshot(sessions, &serial)?,
    })
}

/// 结束某台设备的镜像会话（X10-27）。`serial` 为 `None` 时结束主会话。
#[tauri::command]
fn stop_mirroring(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    log: State<DiagnosticsLog>,
    serial: Option<String>,
) -> Result<(), AppError> {
    // 停止前快照录制状态：录制中的会话被结束 ⇒ 伴侣端要收到「录制结束」（M4-3）。
    // 用精确判定：历史录制条目（进程已没）不算「正在录制」。
    let was_recording = any_session_recording(&sessions);
    let result = stop_mirroring_with(&sessions, serial.clone());
    log.record_outcome("mirror_stop", result.as_ref().err(), &[]);
    // 会话已结束：还原该设备的无线亮屏补偿（若有）。失败时备份保留，等待重试。
    if result.is_ok() {
        if was_recording {
            notify_companion_recording(&app, false);
        }
        let target = serial.or_else(|| {
            sessions
                .lock()
                .ok()
                .and_then(|map| primary_session_serial(&map))
        });
        // take_running_process 已把该设备会话置回 Idle，此时表里可能只剩账本；
        // 主会话路径下 serial 需从停止前的上下文拿——这里兜底遍历所有仍有账本的设备。
        match target {
            Some(serial) => {
                disable_wireless_keep_awake(runtimes.adb.as_ref(), &sessions, &serial);
                prune_idle_entry(&sessions, &serial);
            }
            None => disable_all_wireless_keep_awake(runtimes.adb.as_ref(), &sessions),
        }
        // 输入源恢复（X10-39）：全部会话都结束后，把宿主输入法还给用户。
        let running = sessions
            .lock()
            .map(|map| count_running_processes(&map))
            .unwrap_or(usize::MAX); // 锁不可用时保守处理：视为仍在运行，不恢复。
        maybe_restore_host_input_source(&app, running);
    }
    refresh_tray_menu(&app);
    result
}

fn stop_mirroring_with(
    store: &SessionStore,
    serial: Option<String>,
) -> Result<(), AppError> {
    let target = match serial {
        Some(serial) => serial,
        None => {
            let map = store.lock()?;
            match primary_session_serial(&map) {
                Some(serial) => serial,
                None => {
                    return Err(AppError::new(
                        "session_not_running",
                        "当前没有正在运行的镜像会话。",
                        "请先选择设备并开始镜像。",
                    ))
                }
            }
        }
    };
    match take_running_process(store, &target)? {
        // 优雅结束：给 scrcpy 时间收尾（录制文件写索引），超时才强杀。
        Some(mut process) => process.stop().map_err(|_| {
            AppError::new(
                "mirror_stop_failed",
                "无法结束镜像窗口。",
                "请手动关闭镜像窗口后重试。",
            )
        }),
        None => Err(AppError::new(
            "session_not_running",
            "当前没有正在运行的镜像会话。",
            "请先选择设备并开始镜像。",
        )),
    }
}

/// 主会话快照：向后兼容的单会话界面入口（优先真正持有镜像进程的设备）。
#[tauri::command]
fn mirror_session(sessions: State<SessionStore>) -> Result<MirrorSession, AppError> {
    session_snapshot(&sessions)
}

/// 全部设备的会话快照（X10-27 并发多设备），按序列号稳定排序。
#[tauri::command]
fn mirror_sessions(sessions: State<SessionStore>) -> Result<Vec<MirrorSession>, AppError> {
    all_session_snapshots(&sessions)
}

/// 会话进行中应用新的窗口设置（`serial` 指定目标设备，`None` 时作用于主会话）。
///
/// 语义是明确的「结束旧窗口 + 按新设置重新打开」：镜像画面会短暂中断，界面必须如实
/// 告知用户，不能假装设置已经热更新。设置与当前会话一致时不做任何动作，避免无谓地
/// 打断一次正常的镜像。
#[tauri::command]
fn update_session_options(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    log: State<DiagnosticsLog>,
    options: SessionOptions,
    // X10-92（方案甲）：录制与设置解耦，此参数保留仅为兼容旧前端调用，一律忽略。
    _record_file_name: Option<String>,
    serial: Option<String>,
) -> Result<SessionUpdate, AppError> {
    // 同样是「先校验、再触碰运行中的会话」：路径不可用时不打断正在进行的镜像。
    // Pro 门控同样前置：免费版把录制重新打开时直接拒绝，不打断当前会话。
    ensure_edition_allows(&app, &options)?;
    // X10-92（方案甲）：录制启停走专用命令 start_recording/stop_recording（独立通道、
    // 不重启显示）。应用窗口设置时忽略 record 字段——它与显示参数无关，绝不应触发
    // 「为开/关录制而重启镜像」。若其它显示参数变化导致窗口重启，运行中的独立录制
    // 通道不受影响（它挂在 record_process 上，不随显示进程重启）。
    let options = SessionOptions { record: false, ..options };
    let result = apply_session_options_with(&runtimes, &sessions, options, None, serial);
    log.record_outcome("session_update", result.as_ref().err(), &[]);
    if result.is_ok() {
        // 重启会话期间用户可能刚换了第三方输入法；幂等补一次托管（X10-39）。
        maybe_switch_host_input_source(&app);
    }
    refresh_tray_menu(&app);
    result
}

/// 删除一个录像文件，对应界面上的「撤销」。
#[tauri::command]
fn delete_recording(
    app: AppHandle,
    sessions: State<SessionStore>,
    file_name: String,
) -> Result<(), AppError> {
    let directory = recording_dir(&app)?;
    let file_name = validate_media_name(&file_name, "mp4")?;
    remove_recording_file(&directory, &file_name, &sessions)
}

/// `adb mdns services` 的一条服务记录。
#[derive(Debug, Clone, PartialEq, Eq)]
struct MdnsEntry {
    /// 服务实例名，例如 `adb-79j7kn9tkjt8rwss-rF7qH8`；旧格式输出没有实例名时为空串。
    instance: String,
    /// 服务类型，统一去掉结尾的点，例如 `_adb-tls-pairing._tcp`。
    service: String,
    /// `ip:port` 端点。
    endpoint: String,
}

/// 解析 `adb mdns services` 输出。
///
/// 真实 adb（platform-tools 31+）每行三列，制表符分隔：
/// ```text
/// adb-79j7kn9tkjt8rwss-rF7qH8  _adb-tls-connect._tcp  192.168.1.9:33739
/// ```
/// 兼容只写「类型 + 端点」的两列旧格式。类型识别靠已知服务类型白名单；
/// 端点必须形如 ip:port，非法行直接跳过，不猜。
fn parse_mdns_entries(raw: &str) -> Vec<MdnsEntry> {
    const KNOWN: [(&str, &str); 2] = [
        ("_adb-tls-pairing._tcp.", "_adb-tls-pairing._tcp"),
        ("_adb-tls-connect._tcp.", "_adb-tls-connect._tcp"),
    ];
    let mut entries = Vec::new();
    for line in raw.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        for (index, token) in tokens.iter().enumerate() {
            // 兼容带点与不带点两种服务类型写法，命中后统一用去点的规范名。
            let Some((_, service)) = KNOWN
                .iter()
                .find(|(dotted, plain)| *dotted == *token || *plain == *token)
            else {
                continue;
            };
            let endpoint = tokens.get(index + 1).copied().unwrap_or("");
            let valid = endpoint.rsplit_once(':').is_some_and(|(_, port)| {
                !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
            });
            if !valid {
                continue;
            }
            let instance = if index > 0 { tokens[index - 1] } else { "" };
            entries.push(MdnsEntry {
                instance: instance.to_owned(),
                service: (*service).to_owned(),
                endpoint: endpoint.to_owned(),
            });
        }
    }
    entries
}

/// 旧接口：按服务类型挑出 (配对地址, 连接地址) 两组去重端点。
fn parse_mdns_services(raw: &str) -> (Vec<String>, Vec<String>) {
    let mut pairing = Vec::new();
    let mut connect = Vec::new();
    for entry in parse_mdns_entries(raw) {
        if entry.service == "_adb-tls-pairing._tcp" && !pairing.contains(&entry.endpoint) {
            pairing.push(entry.endpoint);
        } else if entry.service == "_adb-tls-connect._tcp" && !connect.contains(&entry.endpoint) {
            connect.push(entry.endpoint);
        }
    }
    (pairing, connect)
}

/// 自动发现局域网内手机的无线调试地址（mDNS）。手机端要求：
/// - 配对地址：停在「使用配对码配对设备」页面才会广播；
/// - 连接地址：无线调试主页面广播（Android 12+ 部分机型需开启「无线调试」里的
///   mDNS 后端开关）。
/// 端点是局域网地址，不写诊断日志。
#[tauri::command]
fn discover_pairing_services(
    runtimes: State<AppRuntimes>,
    log: State<DiagnosticsLog>,
) -> Result<WirelessServices, AppError> {
    let result = runtimes
        .adb
        .mdns_services()
        .map(|raw| {
            let (pairing, connect) = parse_mdns_services(&raw);
            WirelessServices { pairing, connect }
        })
        .map_err(|error| {
            adb_command_error(
                error,
                "mdns_failed",
                "自动发现不可用。",
                "请手动填写配对地址；旧版本 adb 可能不支持 mDNS，可运行 adb version 确认。",
            )
        });
    log.record_outcome("mdns_discover", result.as_ref().err(), &[]);
    result
}

#[derive(Debug, Clone, serde::Serialize)]
struct WirelessServices {
    pairing: Vec<String>,
    connect: Vec<String>,
}

// -- 二维码配对（Android 11+ 无线调试「使用二维码配对设备」）----------------------
//
// 流程与 Android Studio 的「Pair Using QR Code」一致：
// 1. 桌面生成随机服务名与 6 位配对码，渲染二维码 `WIFI:T:ADB;S:<服务名>;P:<配对码>;;`；
// 2. 手机扫码后广播 `_adb-tls-pairing._tcp`，实例名即二维码里的服务名；
// 3. 桌面轮询 adb mdns services 命中该实例名后执行 adb pair；
// 4. 配对成功后手机改广播 `_adb-tls-connect._tcp`，桌面自动 adb connect。
// 配对码等价于一次性凭据：不写入日志，接口返回后只在前端内存中存在。

#[derive(Default)]
struct QrPairingStore(std::sync::Mutex<std::collections::HashMap<String, QrPairingState>>);

/// X10-91：paired 阶段需要记住「配对成功时 adb pair 命中的手机 IP」，否则配对
/// 成功、手机改广播 `_adb-tls-connect._tcp`（随机实例名）后，无法区分局域网里
/// 多台手机，可能连错设备。pairing 与 connect 是同一台手机先后广播的两个
/// mDNS 服务，源 IP 一致，可安全关联。
#[derive(Clone, Default)]
struct QrPairingState {
    paired: bool,
    /// 配对阶段 adb pair 命中条目的手机 IP（`ip:port` 的 host 部分）。
    paired_ip: Option<String>,
}

impl QrPairingStore {
    fn insert(&self, service_name: &str) {
        self.0.lock().unwrap().insert(service_name.to_owned(), QrPairingState::default());
    }
    fn mark_paired(&self, service_name: &str, paired_ip: Option<String>) {
        if let Some(state) = self.0.lock().unwrap().get_mut(service_name) {
            state.paired = true;
            state.paired_ip = paired_ip;
        }
    }
    fn is_paired(&self, service_name: &str) -> bool {
        self.0.lock().unwrap().get(service_name).map(|s| s.paired).unwrap_or(false)
    }
    fn paired_ip(&self, service_name: &str) -> Option<String> {
        self.0.lock().unwrap().get(service_name).and_then(|s| s.paired_ip.clone())
    }
    fn remove(&self, service_name: &str) {
        self.0.lock().unwrap().remove(service_name);
    }
}

fn random_service_name() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut bytes = [0u8; 6];
    getrandom::getrandom(&mut bytes).expect("系统熵源不可用");
    let suffix: String = bytes.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
    format!("mirrordock-{suffix}")
}

fn random_pairing_code() -> String {
    let mut bytes = [0u8; 6];
    getrandom::getrandom(&mut bytes).expect("系统熵源不可用");
    bytes.iter().map(|b| char::from(b'0' + (b % 10))).collect()
}

#[derive(Debug, Clone, serde::Serialize)]
struct QrPairingOffer {
    /// 发给手机扫的二维码内容。
    payload: String,
    service_name: String,
    pairing_code: String,
}

/// 开始一次扫码配对：生成二维码载荷并登记跟踪状态。前端持有返回值并轮询进度。
#[tauri::command]
fn begin_qr_pairing(store: State<QrPairingStore>) -> QrPairingOffer {
    let service_name = random_service_name();
    let pairing_code = random_pairing_code();
    store.insert(&service_name);
    QrPairingOffer {
        payload: format!("WIFI:T:ADB;S:{service_name};P:{pairing_code};;"),
        service_name,
        pairing_code,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct QrPairingProgress {
    /// waiting=等手机扫码；pairing=已发现手机，正在配对；paired=配对成功，等连接；
    /// done=配对且连接完成；ended=该次配对已被取消或不存在。
    stage: String,
    detail: Option<String>,
}

/// 扫码配对进度轮询：无进展返回 waiting；发现手机实例名后执行配对与连接。
/// 每次调用都是幂等的——已配对的会话不会重复配对，已完成的不会重复连接。
#[tauri::command]
fn qr_pairing_progress(
    store: State<QrPairingStore>,
    runtimes: State<AppRuntimes>,
    service_name: String,
    pairing_code: String,
) -> Result<QrPairingProgress, AppError> {
    if service_name.is_empty()
        || !service_name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || validate_pairing_code(&pairing_code).is_err()
    {
        return Err(AppError::new(
            "qr_pairing_invalid",
            "扫码配对参数不合法。",
            "请重新生成二维码后再试。",
        ));
    }
    if !store.0.lock().unwrap().contains_key(&service_name) {
        return Ok(QrPairingProgress { stage: "ended".into(), detail: None });
    }
    let raw = runtimes.adb.mdns_services().map_err(|error| {
        adb_command_error(
            error,
            "mdns_failed",
            "无法搜索局域网内的无线调试服务。",
            "请确认 adb 可用后重试；也可以改用「配对码配对」手动填写。",
        )
    })?;
    let entries = parse_mdns_entries(&raw);
    if !store.is_paired(&service_name) {
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.service == "_adb-tls-pairing._tcp" && entry.instance == service_name)
        else {
            return Ok(QrPairingProgress { stage: "waiting".into(), detail: None });
        };
        runtimes
            .adb
            .pair(&entry.endpoint, &pairing_code)
            .map_err(|error| {
                adb_command_error(
                    error,
                    "pairing_failed",
                    "发现手机但配对未完成。",
                    "请确认手机停在「使用二维码配对设备」页面后重试；失败持续时可改用配对码配对。",
                )
            })?;
        store.mark_paired(
            &service_name,
            entry.endpoint.rsplit_once(':').map(|(host, _)| host.to_owned()),
        );
        return Ok(QrPairingProgress { stage: "paired".into(), detail: None });
    }
    // 配对已完成：只连接「源 IP 与 pairing 阶段 adb pair 命中的那台手机相同」的
    // _adb-tls-connect 条目（X10-91）。pairing 与 connect 是同一台手机先后广播的
    // 两个 mDNS 服务，源 IP 一致；而实例名在配对成功后被手机换成随机串，无法直接
    // 用名字关联。用 IP 关联后，多台手机共存也不会连错。
    //
    // pairing 阶段成功时已把命中条目的端点存进 store（键 <服务名> → ip），这里取出比对。
    let paired_ip = store.paired_ip(&service_name);
    let connect_entry = entries.iter().find(|entry| {
        entry.service == "_adb-tls-connect._tcp"
            && paired_ip
                .as_deref()
                .is_some_and(|ip| entry.endpoint.rsplit_once(':').map(|(host, _)| host) == Some(ip))
    });
    if let Some(entry) = connect_entry {
        match runtimes.adb.connect(&entry.endpoint) {
            Ok(()) => {
                store.remove(&service_name);
                return Ok(QrPairingProgress { stage: "done".into(), detail: Some(entry.endpoint.clone()) });
            }
            Err(_) => {
                return Ok(QrPairingProgress {
                    stage: "paired".into(),
                    detail: Some("手机已配对，但自动连接还没有完成。".into()),
                });
            }
        }
    }
    Ok(QrPairingProgress {
        stage: "paired".into(),
        detail: Some("配对成功，等待手机回到无线调试主页面后自动连接。".into()),
    })
}

/// 结束/取消扫码配对：清除跟踪状态；前端的二维码也随之作废。
#[tauri::command]
fn end_qr_pairing(store: State<QrPairingStore>, service_name: String) {
    store.remove(&service_name);
}

#[tauri::command]
fn pair_wireless_device(
    runtimes: State<AppRuntimes>,
    log: State<DiagnosticsLog>,
    endpoint: String,
    pairing_code: String,
) -> Result<(), AppError> {
    let result = (|| {
        let endpoint = validate_endpoint(&endpoint)?;
        let pairing_code = validate_pairing_code(&pairing_code)?;
        runtimes.adb.pair(&endpoint, &pairing_code).map_err(|error| {
            adb_command_error(
                error,
                "pairing_failed",
                "配对未完成。",
                "请确认手机与电脑在同一 Wi-Fi，且配对地址、端口和 6 位配对码仍在有效期内。",
            )
        })
    })();
    // 配对码与端点都不进诊断日志；详情只来自用户可见文案。
    log.record_outcome("wireless_pair", result.as_ref().err(), &[&pairing_code, &endpoint]);
    result
}

#[tauri::command]
fn connect_wireless_device(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    log: State<DiagnosticsLog>,
    endpoint: String,
) -> Result<(), AppError> {
    let result = (|| {
        let endpoint = validate_endpoint(&endpoint)?;
        runtimes.adb.connect(&endpoint).map_err(|error| {
            adb_command_error(
                error,
                "connect_failed",
                "无法连接手机。",
                "请确认手机无线调试仍开启、电脑和手机在同一 Wi-Fi，然后重试。",
            )
        })?;

        let devices = runtimes.adb.list_devices().map_err(|error| {
            adb_command_error(
                error,
                "adb_unavailable",
                "无法确认手机的连接状态。",
                "请重新检查连接后再试。",
            )
        })?;
        if !endpoint_is_ready(&devices, &endpoint) {
            return Err(AppError::new(
                "connect_not_ready",
                "尚未确认已授权连接。",
                "请使用无线调试主页面的连接端口，保持同一 Wi-Fi 后重试。",
            ));
        }

        let path = trusted_devices_path(&app)?;
        let mut trusted = load_trusted_devices(&path)?;
        if !trusted.iter().any(|device| device.endpoint == endpoint) {
            trusted.push(TrustedWirelessDevice {
                endpoint: endpoint.clone(),
            });
            save_trusted_devices(&path, &trusted)?;
        }
        mark_paired_if_idle(&sessions, endpoint);
        Ok(())
    })();
    // 端点（局域网地址）不进诊断日志。
    log.record_outcome("wireless_connect", result.as_ref().err(), &[&endpoint]);
    result
}

#[tauri::command]
fn list_trusted_wireless_devices(
    app: AppHandle,
) -> Result<Vec<TrustedWirelessDevice>, AppError> {
    load_trusted_devices(&trusted_devices_path(&app)?)
}

#[tauri::command]
fn forget_trusted_wireless_device(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    endpoint: String,
) -> Result<(), AppError> {
    let endpoint = validate_endpoint(&endpoint)?;
    let path = trusted_devices_path(&app)?;
    let mut trusted = load_trusted_devices(&path)?;
    trusted.retain(|device| device.endpoint != endpoint);
    save_trusted_devices(&path, &trusted)?;

    // 断开是本机尽力而为：手机端是否撤销配对由用户在系统设置中控制。
    let _ = runtimes.adb.disconnect(&endpoint);
    clear_paired_if_matches(&sessions, &endpoint);
    Ok(())
}

// -- 客户端侧取消授权（X10-32）----------------------------------------------
//
// 边界如实声明：Android 的授权记录保存在手机上（/data/misc/adb，root 才能直接
// 清除），「撤销 USB 调试授权」按钮只能由本人在手机上点。客户端能做的是：
// ① 把手机的开发者选项页打开（引导本人完成最后一步）；
// ② 关闭手机上的「无线调试」与「USB 调试」开关（等效收回所有电脑的访问权，
//    这两部开关由 adb shell 的系统设置权限写入，参数固定、可审计）；
// ③ 断开本机全部无线连接并清理本机的受信/最近记录。

/// 取消授权的执行回执：每一步做了什么（或为什么没做成），按执行顺序。
#[derive(Debug, Clone, serde::Serialize)]
struct RevokeReceipt {
    steps: Vec<String>,
}

/// 依次尝试用每个端点执行同一设备侧操作，返回第一个成功的结果；
/// 全部失败时把各端点的原因汇总——无线通道死了还有 USB 通道兜底。
fn first_endpoint_ok<T>(
    endpoints: &[String],
    mut operation: impl FnMut(&str) -> Result<T, std::io::Error>,
) -> Result<T, String> {
    let mut failures = Vec::new();
    for endpoint in endpoints {
        match operation(endpoint) {
            Ok(value) => return Ok(value),
            Err(error) => failures.push(format!("{endpoint}: {error}")),
        }
    }
    Err(failures.join("；"))
}

/// 找到一台设备（按 serial 或其任一连接端点匹配）并收集全部端点，USB 优先——
/// USB 通道最稳定，设备侧设置写入优先走它。
fn device_endpoints_for(devices: &[AdbDevice], serial: &str) -> Option<Vec<String>> {
    let device = devices.iter().find(|device| {
        device.serial == serial || device.connections.iter().any(|c| c.serial == serial)
    })?;
    let mut endpoints: Vec<String> = device
        .connections
        .iter()
        .map(|c| c.serial.clone())
        .collect();
    if !endpoints.iter().any(|value| value == &device.serial) {
        endpoints.push(device.serial.clone());
    }
    endpoints.sort_by_key(|endpoint| is_wireless_endpoint(endpoint));
    Some(endpoints)
}

fn revoke_device_access_with(
    runtimes: &AppRuntimes,
    endpoints: &[String],
) -> RevokeReceipt {
    let mut steps = Vec::new();
    // 回执面向非技术用户：失败只说「设备当前无响应或未授权」，
    // 不透出 adb 原始英文报错（X10-35）；细节仍留在诊断日志。
    // ① 先打开手机的开发者选项（趁通道还活着）。
    match first_endpoint_ok(endpoints, |serial| {
        runtimes.adb.open_developer_settings(serial)
    }) {
        Ok(()) => steps.push("已在手机上打开「开发者选项」页面。".into()),
        Err(_) => steps.push(
            "这台手机当前无响应（离线或未授权），未能自动打开开发者选项；请在手机上手动进入 设置 → 开发者选项。".into(),
        ),
    }
    // ② 关闭「无线调试」：所有电脑的无线访问立即失效。
    match first_endpoint_ok(endpoints, |serial| {
        runtimes.adb.disable_wireless_debugging(serial)
    }) {
        Ok(()) => steps.push("已关闭手机上的「无线调试」开关。".into()),
        Err(_) => steps.push("关闭「无线调试」未成功：设备当前无响应，可在手机上手动关闭。".into()),
    }
    // ③ 关闭「USB 调试」：所有电脑的调试访问（含 USB）立即失效，连接随之断开。
    match first_endpoint_ok(endpoints, |serial| {
        runtimes.adb.disable_usb_debugging(serial)
    }) {
        Ok(()) => steps.push("已关闭手机上的「USB 调试」开关，连接即将断开。".into()),
        Err(_) => steps.push("关闭「USB 调试」未成功：设备当前无响应，可在手机上手动关闭。".into()),
    }
    // ④ 本机侧断开全部无线端点（尽力而为）。
    for endpoint in endpoints {
        if is_wireless_endpoint(endpoint) {
            let _ = runtimes.adb.disconnect(endpoint);
        }
    }
    steps.push("已断开本机与这台手机的全部无线连接，并已把它从设备列表移除。".into());
    steps.push(
        "最后一步要在手机上完成：在手机的「开发者选项」里点「撤销 USB 调试授权」，清除手机保存的授权记录。".into(),
    );
    RevokeReceipt { steps }
}

#[tauri::command]
fn revoke_device_access(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    log: State<DiagnosticsLog>,
    serial: String,
) -> Result<RevokeReceipt, AppError> {
    let serial = validate_serial(&serial)?;
    let devices = runtimes.adb.list_devices().map_err(|error| {
        adb_command_error(
            error,
            "adb_unavailable",
            "无法读取设备列表。",
            "请确认 adb 可用后重试。",
        )
    })?;
    let endpoints = device_endpoints_for(&devices, &serial).ok_or_else(|| {
        AppError::new(
            "device_missing",
            "这台手机已不在连接列表里。",
            "无需取消授权；如果手机仍显示已连接，请在手机上直接撤销。",
        )
    })?;
    // 镜像进行中不允许直接取消授权：会话会随开关关闭立即死亡，先结束再撤。
    if let Ok(map) = sessions.lock() {
        if map.iter().any(|(session_serial, state)| {
            is_session_active(state)
                && (endpoints.iter().any(|endpoint| endpoint == session_serial)
                    || device_endpoints_for(&devices, session_serial)
                        .is_some_and(|own| {
                            own.iter().any(|endpoint| endpoints.contains(endpoint))
                        }))
        }) {
            return Err(AppError::new(
                "session_active",
                "这台手机正在镜像中。",
                "请先结束镜像，再取消授权。",
            ));
        }
    }

    let receipt = revoke_device_access_with(runtimes.inner(), &endpoints);

    // 本机记录清理：受信无线设备与最近设备里属于这台手机的端点。读写失败
    // 不影响取消授权本身——记录只是便利功能。
    if let Ok(path) = trusted_devices_path(&app) {
        if let Ok(mut trusted) = load_trusted_devices(&path) {
            let before = trusted.len();
            trusted.retain(|device| !endpoints.contains(&device.endpoint));
            if trusted.len() != before {
                let _ = save_trusted_devices(&path, &trusted);
            }
        }
    }
    if let Ok(path) = recent_devices_path(&app) {
        if let Ok(mut recents) = load_recent_devices(&path) {
            let before = recents.len();
            recents.retain(|device| !endpoints.contains(&device.serial));
            if recents.len() != before {
                let _ = save_recent_devices(&path, &recents);
            }
        }
    }
    // 诊断日志只记事件，不记序列号与端点。
    log.record(
        "revoke_device",
        "ok",
        "已执行客户端侧取消授权流程",
        &[],
    );
    Ok(receipt)
}


#[tauri::command]
fn wake_device(runtimes: State<AppRuntimes>, serial: String) -> Result<(), AppError> {
    let serial = validate_serial(&serial)?;
    if let Some(error) = device_readiness_error(device_lookup(&runtimes, &serial)) {
        return Err(error);
    }
    wake_screen_for_serial(&runtimes, &serial)
}

/// 安装完更新后重启应用（X10-47）。`restart()` 不返回（进程被替换）。
#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

/// 在手机上打开「实体键盘」设置页（X10-46）。
///
/// UHID 键盘打字依赖手机端为 scrcpy 键盘启用的布局；用户反馈「打字没反应/字符
/// 全错」时，把这个页面带到用户面前（确认「英语（美国）」已启用），并提醒镜像
/// 窗口里 MOD+k 也能打开同一页面（scrcpy 内置快捷键）。
#[tauri::command]
fn open_keyboard_settings(runtimes: State<AppRuntimes>, serial: String) -> Result<(), AppError> {
    let serial = validate_serial(&serial)?;
    if let Some(error) = device_readiness_error(device_lookup(&runtimes, &serial)) {
        return Err(error);
    }
    runtimes
        .adb
        .open_keyboard_layout_settings(&serial)
        .map_err(|_| {
            AppError::new(
                "keyboard_settings_open_failed",
                "无法在手机上打开「实体键盘」设置。",
                "请解锁手机后重试；也可以在手机上手动进入 设置 → 系统与更新 → 实体键盘。",
            )
        })
}

#[tauri::command]
async fn device_lock_report(
    runtimes: State<'_, AppRuntimes>,
    serial: String,
) -> Result<DeviceLockReport, AppError> {
    // X10-89：async 使命令体跑在 tauri 工作线程，adb 调用（dumpsys power）不再阻塞主线程。
    lock_report_with(&runtimes, serial)
}

/// 探测「密码输入页（安全表面）」是否正在屏上。
///
/// 判据来自真机双路取证：锁屏中 + 屏幕点亮 + screencap 输出异常小（实测 0 字节）。
/// 只读状态与字节数，不保存、不回传任何屏幕像素。前端据此在锁屏面板显示
/// 「请在手机上输入密码解锁」的提示，密码页退出后自动消失。
#[tauri::command]
async fn probe_pin_pad_state(
    runtimes: State<'_, AppRuntimes>,
    serial: String,
) -> Result<PinPadProbe, AppError> {
    // X10-89：async 使命令体跑在 tauri 工作线程，adb 调用（screencap/dumpsys）不再阻塞主线程。
    pin_pad_probe_with(&runtimes, serial)
}

#[tauri::command]
fn list_recent_devices(app: AppHandle) -> Result<Vec<RecentDevice>, AppError> {
    load_recent_devices(&recent_devices_path(&app)?)
}

/// 把一台设备从本机的最近使用记录中移除。
///
/// 它只删除“最近使用”这条**本地便利记录**：不会断开当前连接、不会忘记无线配对，也不会
/// 撤销手机上的调试授权。本地优先的产品里，用户必须能撤销自己被记下的痕迹。
/// 返回移除后的完整列表，省去前端再取一次。
#[tauri::command]
fn forget_recent_device(app: AppHandle, serial: String) -> Result<Vec<RecentDevice>, AppError> {
    let serial = validate_serial(&serial)?;
    forget_recent_device_at(&recent_devices_path(&app)?, &serial)
}

/// 一键清空本机的全部最近使用记录（物理删除：直接把记录文件写成空列表）。
///
/// 与逐条「移除记录」同一语义边界：只删本地便利记录，不断开连接、不撤销手机授权、
/// 不清除无线配对。之后再次启动镜像时，用过的设备会按既有行为重新记入。
#[tauri::command]
fn clear_recent_devices(app: AppHandle) -> Result<(), AppError> {
    save_recent_devices(&recent_devices_path(&app)?, &[])
}

/// 读取全部桌面模式偏好（X10-90）。供前端启动/设备变化时拉取；键为稳定
/// physical_serial，与后端解析口径一致。解析失败返回空表（前端容错兜底）。
#[tauri::command]
fn get_desktop_prefs(
    app: AppHandle,
) -> std::collections::HashMap<String, DesktopPrefEntry> {
    let Ok(path) = desktop_prefs_path(&app) else {
        return std::collections::HashMap::new();
    };
    load_desktop_prefs(&path)
}

/// 覆盖式写入全部桌面模式偏好（X10-90）。前端每次增删改后传整表；返回错误时
/// 前端如实提示，不假装保存成功。键在写前归一化（mDNS 端点→physical_serial），
/// 与读取/解析口径对齐。
#[tauri::command]
fn set_desktop_prefs(
    app: AppHandle,
    prefs: std::collections::HashMap<String, DesktopPrefEntry>,
) -> Result<(), AppError> {
    let normalized: std::collections::HashMap<String, DesktopPrefEntry> = prefs
        .into_iter()
        .map(|(key, entry)| (normalize_device_identity_key(&key), entry))
        .collect();
    save_desktop_prefs(&desktop_prefs_path(&app)?, &normalized)
}

/// 把手机当前画面保存为本机的一张 PNG。
///
/// 文件名由前端按**本地时间**生成（后端不猜时区），随后按不可信输入严格校验。
/// 截图内容是屏幕像素：只写入用户可见的本地文件，不写日志、不进错误消息。
#[tauri::command]
fn capture_screenshot(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    log: State<DiagnosticsLog>,
    serial: String,
    file_name: String,
) -> Result<Screenshot, AppError> {
    let directory = screenshot_dir(&app)?;
    let result = capture_screenshot_into(&runtimes, &directory, serial.clone(), file_name.clone());
    // 文件名与序列号都不入日志；失败详情只来自用户可见文案。
    log.record_outcome("screenshot", result.as_ref().err(), &[&serial, &file_name]);
    result
}

/// 删除一张由本应用保存的截图，对应界面上的「撤销」。
#[tauri::command]
fn delete_screenshot(app: AppHandle, file_name: String) -> Result<(), AppError> {
    let directory = screenshot_dir(&app)?;
    let file_name = validate_screenshot_name(&file_name)?;
    remove_screenshot_file(&directory, &file_name)
}

/// 从手机取回文件的本机保存目录：「下载 / MirrorDock」（不可用时退回应用数据目录）。
fn transfer_download_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir())
        .map(|directory| directory.join("MirrorDock"))
        .map_err(|_| transfer_dir_unavailable_error())
}

/// 把一个本机文件发送到手机的「下载 / MirrorDock」目录。
///
/// 只在用户明确选择文件后调用；文件内容不经过 MirrorDock 进程，不入日志。
#[tauri::command]
fn send_file_to_device(
    runtimes: State<AppRuntimes>,
    log: State<DiagnosticsLog>,
    serial: String,
    local_path: String,
) -> Result<TransferReceipt, AppError> {
    let result = send_file_to_device_with(&runtimes, serial.clone(), local_path.clone());
    // 本地路径可能包含用户名等隐私，一并作为机密擦除。
    log.record_outcome("file_send", result.as_ref().err(), &[&serial, &local_path]);
    result
}

/// 列出手机上适合在虚拟屏启动的应用（X10-53；X10-71 升级为「应用名 + 包名」）。
///
/// 首选 `scrcpy --list-apps`：只列**可启动**的应用（有启动入口，与「虚拟屏启动
/// 应用」的场景一致），且带应用名——小白用户看得懂「浏览器」而看不懂
/// `com.android.browser`。scrcpy 不可用或失败时回退 `pm list packages -3`
/// （应用名退化为包名）。读操作，不改变设备状态。
#[tauri::command]
fn list_device_apps(
    runtimes: State<AppRuntimes>,
    serial: String,
) -> Result<Vec<DeviceApp>, AppError> {
    list_device_apps_with(&runtimes, serial)
}

fn list_device_apps_with(runtimes: &AppRuntimes, serial: String) -> Result<Vec<DeviceApp>, AppError> {
    list_device_apps_scoped(runtimes, serial, run_scrcpy_list_apps)
}

/// `scrcpy_apps` 是注入点：单测传 `|_| None`（跳过 scrcpy 路径）或样例输出，
/// 避免测试真正拉起 scrcpy 进程。
fn list_device_apps_scoped<F>(
    runtimes: &AppRuntimes,
    serial: String,
    scrcpy_apps: F,
) -> Result<Vec<DeviceApp>, AppError>
where
    F: FnOnce(&str) -> Option<String>,
{
    let serial = validate_serial(&serial)?;
    if let Some(error) = device_readiness_error(device_lookup(runtimes, &serial)) {
        return Err(error);
    }
    if let Some(output) = scrcpy_apps(&serial) {
        let apps = parse_scrcpy_app_list(&output);
        if !apps.is_empty() {
            return Ok(apps);
        }
    }
    let raw = runtimes.adb.list_device_apps(&serial).map_err(|_| {
        AppError::new(
            "app_list_failed",
            "无法读取手机上的应用列表。",
            "请确认连接仍然有效，然后重试。",
        )
    })?;
    let mut apps: Vec<DeviceApp> = raw
        .lines()
        .filter_map(|line| line.strip_prefix("package:"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|package| DeviceApp {
            package: package.to_string(),
            name: package.to_string(),
        })
        .collect();
    apps.sort_by(|a, b| a.package.cmp(&b.package));
    apps.dedup_by(|a, b| a.package == b.package);
    Ok(apps)
}

/// 「虚拟屏启动的应用」候选条目（X10-71）：下拉里展示应用名，提交包名。
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
struct DeviceApp {
    package: String,
    name: String,
}

/// 运行 `scrcpy --list-apps -s <serial>`，返回 stdout；失败或超时返回 None。
///
/// 固定参数直接调用（序列号已过 `validate_serial` 白名单校验），不做任何 shell
/// 拼接。scrcpy 把 server 日志与应用列表写到 stdout、推送进度写到 stderr，
/// 这里只取 stdout。stdout 读取会阻塞到进程退出，因此放到线程里，主线程轮询
/// 等待并在超时后强杀，避免设备假死把命令挂住。
fn run_scrcpy_list_apps(serial: &str) -> Option<String> {
    let mut child = quiet_command(scrcpy_binary())
        .args(["--list-apps", "-s", serial])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        match std::io::Read::read_to_string(&mut stdout, &mut text) {
            Ok(_) => text,
            Err(_) => String::new(),
        }
    });
    let deadline = std::time::Instant::now() + LIST_APPS_TIMEOUT;
    let exited = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if std::time::Instant::now() >= deadline => break false,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break false,
        }
    };
    if !exited {
        // 超时：强杀后不必等读取线程返回（进程退出即 EOF），直接按失败处理。
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let text = reader.join().unwrap_or_default();
    if text.is_empty() { None } else { Some(text) }
}

/// 解析 `scrcpy --list-apps` 输出里的应用条目（X10-71）。
///
/// 应用行形如 ` * 应用名<空白>包名`：应用名可以包含空格，包名恒为行内最后一个
/// 空白分隔的字段。server 日志行（以 `[` 开头）与无法识别的杂行一律忽略；包名
/// 必须匹配 Android 包名字符集且至少含一个点。设备返回的内容属于不可信输入，
/// 应用名里的控制字符会被剔除。结果按应用名排序、按包名去重。
fn parse_scrcpy_app_list(output: &str) -> Vec<DeviceApp> {
    let mut apps: Vec<DeviceApp> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        // 行首的 `*` 是 scrcpy 的标记位，不是名字的一部分。
        let line = line.strip_prefix('*').map(str::trim).unwrap_or(line);
        let Some((name, package)) = line.rsplit_once(char::is_whitespace) else {
            continue;
        };
        let package = package.trim();
        if !is_package_like(package) {
            continue;
        }
        let name: String = name.chars().filter(|c| !c.is_control()).collect();
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        apps.push(DeviceApp {
            package: package.to_string(),
            name: name.to_string(),
        });
    }
    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.package.cmp(&b.package))
    });
    apps.dedup_by(|a, b| a.package == b.package);
    apps
}

/// Android 包名的宽松白名单：字母、数字、点、下划线，至少一个点，长度受限。
/// 与 `SessionOptions::arguments` 里 `--start-app` 的校验口径一致。
fn is_package_like(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 120
        && token.contains('.')
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

/// 列出手机传输目录里的文件名，供用户挑选要取回的文件。
#[tauri::command]
fn list_device_files(
    runtimes: State<AppRuntimes>,
    serial: String,
) -> Result<Vec<String>, AppError> {
    list_device_files_with(&runtimes, serial)
}

/// 把用户选中的 APK 安装到手机上（「一键安装」）。
///
/// 只在用户明确选择安装包后调用；安装包内容不经过 MirrorDock 进程，也不写日志。
/// 本地路径可能含用户名等隐私，因此与文件名一起作为机密擦除。
#[tauri::command]
fn install_apk_to_device(
    runtimes: State<AppRuntimes>,
    log: State<DiagnosticsLog>,
    serial: String,
    apk_path: String,
) -> Result<ApkInstallReceipt, AppError> {
    let result = install_apk_with(&runtimes, serial.clone(), apk_path.clone());
    log.record_outcome("apk_install", result.as_ref().err(), &[&serial, &apk_path]);
    result
}

/// 从手机取回一个文件，保存到本机「下载 / MirrorDock」。
#[tauri::command]
fn fetch_file_from_device(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    log: State<DiagnosticsLog>,
    serial: String,
    file_name: String,
) -> Result<TransferReceipt, AppError> {
    let directory = transfer_download_dir(&app)?;
    let result = fetch_file_from_device_into(&runtimes, &directory, serial.clone(), file_name.clone());
    log.record_outcome("file_fetch", result.as_ref().err(), &[&serial, &file_name]);
    result
}

/// 删除手机发送区（下载 / MirrorDock）里的一个文件（X10-60 发送区清理）。
#[tauri::command]
fn delete_device_file(
    runtimes: State<AppRuntimes>,
    log: State<DiagnosticsLog>,
    serial: String,
    file_name: String,
) -> Result<(), AppError> {
    let result = delete_device_file_with(&runtimes, serial.clone(), file_name.clone());
    log.record_outcome("file_delete", result.as_ref().err(), &[&serial, &file_name]);
    result
}

/// 组装诊断预览。adb 的可用性不单独作为字段——它已经体现在 device_check 事件里。
fn diagnostics_preview_with(
    app: &AppHandle,
    runtimes: &AppRuntimes,
    log: &DiagnosticsLog,
) -> DiagnosticsPreview {
    build_diagnostics_preview(
        app.package_info().version.to_string(),
        format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
        runtimes.mirror.is_available(),
        log.snapshot(),
    )
}

/// 诊断包预览：用户先看到将要导出的全部内容，再决定是否导出。
#[tauri::command]
fn diagnostics_preview(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    log: State<DiagnosticsLog>,
) -> DiagnosticsPreview {
    diagnostics_preview_with(&app, &runtimes, &log)
}

/// 把预览内容原样写成 JSON 文件。路径来自系统保存对话框，不猜测、不改写。
#[tauri::command]
fn export_diagnostics(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    log: State<DiagnosticsLog>,
    path: String,
) -> Result<DiagnosticsReceipt, AppError> {
    let preview = diagnostics_preview_with(&app, &runtimes, &log);
    export_diagnostics_into(&path, &preview)
}

fn export_diagnostics_into(
    path: &str,
    preview: &DiagnosticsPreview,
) -> Result<DiagnosticsReceipt, AppError> {
    let json = serde_json::to_vec_pretty(preview).map_err(|_| {
        AppError::new(
            "diagnostics_write_failed",
            "诊断包序列化失败。",
            "请重试一次；若仍失败请联系支持人员。",
        )
    })?;
    fs::write(path, &json).map_err(|_| {
        AppError::new(
            "diagnostics_write_failed",
            "诊断包写入失败。",
            "请换一个保存位置（例如桌面或下载文件夹）再试。",
        )
    })?;
    Ok(DiagnosticsReceipt {
        path: path.to_owned(),
        events: preview.events.len(),
        bytes: json.len() as u64,
    })
}

// ===========================================================================
// R3-01 免费/Pro 授权（本地权益，无账户、无激活服务器）
//
// 设计决策（2026-09-28 夜间轮次，自主决策并记录）：
// - 免费版保留全部核心连接体验（USB/无线镜像、截图、文件传输、音频转发、
//   会话设置、诊断）；Pro 门控仅覆盖 MP4 录制——付费点清晰，且不削弱安全与
//   基础可用性。特别地，音频转发**不**做付费门控：SessionOptions.audio 默认
//   开启，若纳入门控会让免费版用户的默认启动直接报错，违背「开箱即用」。
// - 许可证 = ed25519 签名的 JSON 载荷（product/key_id/edition/expires_at），
//   公钥编译进二进制；私钥存于仓库外的内部文档目录，绝不入 Git、绝不在
//   日志/错误/诊断中回显（激活命令把原始密钥串声明进诊断脱敏列表）。
// - 权益状态 = 应用数据目录下的 entitlement.json（存许可证原文，加载时重新
//   验签；任何损坏/过期/验签失败一律回退免费版，绝不因授权问题阻断镜像基础功能）。
// ===========================================================================

/// 许可证验证公钥（ed25519）。对应私钥见内部文档目录的签发说明。
/// 由 examples/license_keygen 生成（/dev/urandom，含签名自检）。
const LICENSE_VERIFYING_KEY: [u8; 32] = [
    0x0a, 0x56, 0xf0, 0x4d, 0xbd, 0x86, 0x11, 0xbf, 0xfe, 0xae, 0x49, 0x91, 0xc4, 0x0b, 0xd6, 0x2f,
    0x4e, 0x99, 0x86, 0x23, 0x4a, 0x8e, 0x81, 0x6e, 0x17, 0x6a, 0xdc, 0x0b, 0xcd, 0xa2, 0x73, 0x17,
];

/// Beta 阶段全量内置的测试许可证（key-id `beta`，永久、Pro 权益）。
/// 存在意义：测试期所有安装默认解锁完整功能，用户无需手动激活。
/// 生命周期：Beta 结束的正式版必须移除本常量；届时已激活的 beta 许可证
/// 因「永久」仍然有效——如需回收，正式版改用带 expires_at 的正式许可证
/// 并通过更新提示用户换发（v1 无远程吊销，见 docs/license-operations.md）。
const BETA_LICENSE_KEY: &str = "MD1-AA4HWI-TQOJXW-I5LDOQ-RDUITN-NFZHE3-3SMRXW-G2ZCFQ-RGWZLZ-L5UWII-R2EJRG-K5DBEI-WCEZLE-NF2GS3-3OEI5C-E4DSN4-RH35EQ-Q775OH-EYH43H-A7A22Q-GJPIRS-S3BFU4-UJ5GAA-VP3VLI-KWFEIZ-JGOGA7-FP4HXZ-OF4WWP-MS5PB4-PJIMMR-FHPCRW-UEJ5CH-4GAL3G-ZIFA";

pub mod licensing {
    use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
    use serde::{Deserialize, Serialize};

    pub const LICENSE_PREFIX: &str = "MD1";
    pub const ENTITLEMENT_FILE: &str = "entitlement.json";

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum Edition {
        Free,
        Pro,
    }

    impl Edition {
        pub fn as_str(self) -> &'static str {
            match self {
                Edition::Free => "free",
                Edition::Pro => "pro",
            }
        }
    }

    /// 许可证载荷。字段顺序即序列化顺序，是验签输入的一部分，不可调整。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct LicensePayload {
        pub product: String,
        pub key_id: String,
        pub edition: String,
        /// Unix 秒；None 表示永久。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub expires_at: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LicenseError {
        Malformed,
        BadSignature,
        WrongProduct,
        Expired,
    }

    // ---- Base32（RFC 4648，无填充）。自实现以避免引入额外依赖。 ----

    const B32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    pub fn base32_encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
        for chunk in data.chunks(5) {
            let mut buf = [0u8; 5];
            buf[..chunk.len()].copy_from_slice(chunk);
            let bits = u64::from_be_bytes([
                0, buf[0], buf[1], buf[2], buf[3], buf[4], 0, 0,
            ]) >> 16;
            let chars = match chunk.len() {
                5 => 8,
                4 => 7,
                3 => 5,
                2 => 4,
                _ => 2,
            };
            for i in 0..chars {
                let idx = ((bits >> (35 - i * 5)) & 0x1f) as usize;
                out.push(B32_ALPHABET[idx] as char);
            }
        }
        out
    }

    pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut bits: u32 = 0;
        let mut acc: u32 = 0;
        for ch in text.chars() {
            let v = B32_ALPHABET
                .iter()
                .position(|&c| c as char == ch.to_ascii_uppercase())? as u32;
            acc = (acc << 5) | v;
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                bytes.push(((acc >> bits) & 0xff) as u8);
            }
        }
        Some(bytes)
    }

    /// 生成许可证字符串：`MD1` + base32(2字节载荷长度 + 载荷 + 64字节签名)，
    /// 以 6 字符分组、`-` 连接便于人工抄写；解析时忽略大小写与分隔符。
    pub fn encode_license(payload: &LicensePayload, signing: &ed25519_dalek::SigningKey) -> String {
        let payload_json =
            serde_json::to_vec(payload).expect("license payload serializes unconditionally");
        let sig = signing.sign(&payload_json).to_bytes();
        assert!(payload_json.len() <= u16::MAX as usize, "payload too long");
        let mut raw = Vec::with_capacity(2 + payload_json.len() + 64);
        raw.extend_from_slice(&(payload_json.len() as u16).to_be_bytes());
        raw.extend_from_slice(&payload_json);
        raw.extend_from_slice(&sig);
        let body = base32_encode(&raw);
        // 前缀后每 6 个字符插一个 '-'（尾部不足 6 个则原样）。
        let mut grouped = String::with_capacity(body.len() + body.len() / 6);
        for (i, ch) in body.chars().enumerate() {
            if i > 0 && i % 6 == 0 {
                grouped.push('-');
            }
            grouped.push(ch);
        }
        format!("{LICENSE_PREFIX}-{grouped}")
    }

    /// 解析并验签。`now` 为 Unix 秒（由调用方注入以便测试过期逻辑）。
    pub fn verify_license(
        license: &str,
        verifying: &VerifyingKey,
        now: u64,
    ) -> Result<LicensePayload, LicenseError> {
        let compact: String = license
            .chars()
            .filter(|c| *c != '-' && *c != ' ')
            .collect::<String>()
            .to_ascii_uppercase();
        let body = compact
            .strip_prefix(LICENSE_PREFIX)
            .ok_or(LicenseError::Malformed)?;
        let raw = base32_decode(body).ok_or(LicenseError::Malformed)?;
        if raw.len() < 2 + 64 {
            return Err(LicenseError::Malformed);
        }
        let payload_len = u16::from_be_bytes([raw[0], raw[1]]) as usize;
        if raw.len() != 2 + payload_len + 64 {
            return Err(LicenseError::Malformed);
        }
        let payload_json = &raw[2..2 + payload_len];
        let sig = Signature::from_slice(&raw[2 + payload_len..])
            .map_err(|_| LicenseError::Malformed)?;
        verifying
            .verify(payload_json, &sig)
            .map_err(|_| LicenseError::BadSignature)?;
        let payload: LicensePayload =
            serde_json::from_slice(payload_json).map_err(|_| LicenseError::Malformed)?;
        if payload.product != "mirrordock" {
            return Err(LicenseError::WrongProduct);
        }
        if payload.expires_at.is_some_and(|at| at <= now) {
            return Err(LicenseError::Expired);
        }
        Ok(payload)
    }

    /// 当前版本判定：无许可证文件 = 免费版；文件存在则重新验签，
    /// 任何失败（损坏/篡改/过期/产品不符）都回退免费版且**不报错**。
    pub fn current_edition(dir: &std::path::Path, now: u64) -> Edition {
        let verifying = match VerifyingKey::from_bytes(&super::LICENSE_VERIFYING_KEY) {
            Ok(key) => key,
            Err(_) => return Edition::Free,
        };
        match std::fs::read_to_string(dir.join(ENTITLEMENT_FILE)) {
            Ok(text) => match serde_json::from_str::<StoredLicense>(&text) {
                Ok(stored) => match verify_license(&stored.license, &verifying, now) {
                    Ok(payload) if payload.edition == "pro" => Edition::Pro,
                    _ => Edition::Free,
                },
                Err(_) => Edition::Free,
            },
            Err(_) => Edition::Free,
        }
    }

    #[derive(Serialize, Deserialize)]
    struct StoredLicense {
        schema: u8,
        license: String,
        activated_at: u64,
    }

    /// 激活：验签通过才落盘。落盘失败是真实错误（不静默），已验签的许可仍在手。
    pub fn activate_into(
        dir: &std::path::Path,
        license: &str,
        now: u64,
    ) -> Result<LicensePayload, LicenseError> {
        let verifying = VerifyingKey::from_bytes(&super::LICENSE_VERIFYING_KEY)
            .map_err(|_| LicenseError::Malformed)?;
        let payload = verify_license(license, &verifying, now)?;
        if payload.edition != "pro" {
            return Err(LicenseError::WrongProduct);
        }
        let stored = StoredLicense {
            schema: 1,
            license: license.trim().to_owned(),
            activated_at: now,
        };
        std::fs::create_dir_all(dir)
            .and_then(|_| {
                std::fs::write(
                    dir.join(ENTITLEMENT_FILE),
                    serde_json::to_vec(&stored).expect("stored serializes"),
                )
            })
            .map_err(|_| LicenseError::Malformed)?;
        Ok(payload)
    }

    pub fn deactivate_in(dir: &std::path::Path) -> Result<(), std::io::Error> {
        match std::fs::remove_file(dir.join(ENTITLEMENT_FILE)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Pro 功能门控：MP4 录制需要 Pro 版。免费版其余功能（含音频转发）
    /// 全部放行，保证默认会话开箱即用。
    pub(crate) fn ensure_pro_features(
        edition: Edition,
        options: &super::SessionOptions,
    ) -> Result<(), super::AppError> {
        if edition == Edition::Pro || !options.record {
            return Ok(());
        }
        Err(super::AppError::new(
            "pro_required",
            "MP4 录制是专业版功能，当前为免费版。",
            "在「版本与授权」中激活专业版许可证，或关闭录制后重试。",
        ))
    }
}

use licensing::Edition;
mod companion_pairing;
mod input_source;

fn entitlement_dir(app: &AppHandle) -> Result<std::path::PathBuf, AppError> {
    app.path().app_data_dir().map_err(|_| {
        AppError::new(
            "entitlement_io_failed",
            "无法定位应用数据目录，授权状态不可用。",
            "请重启应用再试；若仍失败请联系支持人员。",
        )
    })
}

fn entitlement_view(
    edition: Edition,
    payload: Option<&licensing::LicensePayload>,
) -> EntitlementView {
    EntitlementView {
        edition: edition.as_str().to_owned(),
        key_id: payload.map(|p| p.key_id.clone()),
        expires_at: payload.and_then(|p| p.expires_at),
    }
}

#[derive(Serialize)]
struct EntitlementView {
    edition: String,
    key_id: Option<String>,
    expires_at: Option<u64>,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn embedded_verifying_key() -> Result<ed25519_dalek::VerifyingKey, AppError> {
    ed25519_dalek::VerifyingKey::from_bytes(&LICENSE_VERIFYING_KEY).map_err(|_| {
        AppError::new("license_malformed", "许可证公钥异常。", "请联系支持人员。")
    })
}

fn stored_license_payload(dir: &std::path::Path) -> Result<Option<licensing::LicensePayload>, AppError> {
    let text = std::fs::read_to_string(dir.join(licensing::ENTITLEMENT_FILE)).map_err(|_| {
        AppError::new(
            "entitlement_io_failed",
            "授权状态文件不可读。",
            "重新激活许可证即可修复。",
        )
    })?;
    let stored: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
        AppError::new(
            "entitlement_io_failed",
            "授权状态文件损坏。",
            "重新激活许可证即可修复。",
        )
    })?;
    let license = stored["license"].as_str().unwrap_or_default();
    let verifying = embedded_verifying_key()?;
    Ok(licensing::verify_license(license, &verifying, unix_now()).ok())
}

#[tauri::command]
fn entitlement_status(app: AppHandle) -> Result<EntitlementView, AppError> {
    let dir = entitlement_dir(&app)?;
    let edition = licensing::current_edition(&dir, unix_now());
    if edition != Edition::Pro && !beta_opt_out(&dir) {
        // Beta 阶段全量内置测试许可证：未激活时静默激活，让所有用户体验完整功能。
        // 激活失败（不可能发生：内置码与公钥同源编译）按原样返回免费版，不阻断。
        if let Ok(view) = entitlement_activate_impl(&app, BETA_LICENSE_KEY) {
            return Ok(view);
        }
    }
    let payload = if edition == Edition::Pro {
        stored_license_payload(&dir)?
    } else {
        None
    };
    Ok(entitlement_view(edition, payload.as_ref()))
}

/// 用户在 Beta 期间显式撤销授权后，不再自动激活内置测试码（尊重用户选择）。
/// 标记文件只在本机应用数据目录，重装后随目录一起清除。
fn beta_opt_out_file(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("beta_opt_out")
}

fn beta_opt_out(dir: &std::path::Path) -> bool {
    beta_opt_out_file(dir).exists()
}

#[tauri::command]
fn entitlement_activate(
    app: AppHandle,
    log: State<DiagnosticsLog>,
    license_key: String,
) -> Result<EntitlementView, AppError> {
    let result = entitlement_activate_impl(&app, &license_key);
    // 原始密钥串声明进机密列表：激活失败的诊断记录绝不回显用户输入的密钥。
    log.record_outcome("license_activate", result.as_ref().err(), &[license_key.as_str()]);
    result
}

fn entitlement_activate_impl(
    app: &AppHandle,
    license_key: &str,
) -> Result<EntitlementView, AppError> {
    let dir = entitlement_dir(app)?;
    let trimmed = license_key.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            "license_malformed",
            "许可证为空。",
            "请输入完整的许可证，格式形如 MD1-XXXXXX-…。",
        ));
    }
    let now = unix_now();
    let payload = licensing::activate_into(&dir, trimmed, now).map_err(|err| match err {
        licensing::LicenseError::Malformed => AppError::new(
            "license_malformed",
            "许可证格式无法识别。",
            "请检查是否复制完整（以 MD1- 开头），不要混入多余空行。",
        ),
        licensing::LicenseError::BadSignature => AppError::new(
            "license_invalid",
            "许可证签名无效。",
            "请确认许可证来自官方渠道，必要时联系支持人员核对。",
        ),
        licensing::LicenseError::WrongProduct => AppError::new(
            "license_invalid",
            "这不是有效的 MirrorDock 专业版许可证。",
            "请核对许可证是否为 MirrorDock 专业版。",
        ),
        licensing::LicenseError::Expired => AppError::new(
            "license_expired",
            "许可证已过期。",
            "请续订专业版后重新激活，或联系支持人员。",
        ),
    })?;
    Ok(entitlement_view(Edition::Pro, Some(&payload)))
}

#[tauri::command]
fn entitlement_deactivate(app: AppHandle) -> Result<EntitlementView, AppError> {
    let dir = entitlement_dir(&app)?;
    licensing::deactivate_in(&dir).map_err(|_| {
        AppError::new(
            "entitlement_io_failed",
            "撤销授权失败。",
            "请检查应用数据目录权限后重试。",
        )
    })?;
    // 撤销即视为用户主动退出 Beta 全量解锁：不再自动激活内置测试码。
    let _ = std::fs::write(beta_opt_out_file(&dir), b"");
    Ok(entitlement_view(Edition::Free, None))
}

/// 启动镜像与会话更新共用的 Pro 门控：录制在免费版下拒绝，
/// 其余功能（含全部安全相关能力与默认体验）不受授权状态影响。
fn ensure_edition_allows(app: &AppHandle, options: &SessionOptions) -> Result<(), AppError> {
    let edition = licensing::current_edition(&entitlement_dir(app)?, unix_now());
    licensing::ensure_pro_features(edition, options)
}

// ---------------------------------------------------------------------------
// C4-01 伴侣 App 配对（POC）
// ---------------------------------------------------------------------------

/// 伴侣会话 → 镜像通道桥接：伴侣 App 扫码建立的是它自己的加密通道，
/// 不会自动把手机加进连接列表（这是设计边界，但用户自然预期「扫完就能连」）。
/// 这里用伴侣端的来源 IP 在 mDNS 里找这台手机的无线调试广播并自动
/// `adb connect`——手机开过无线调试且与电脑配对过即可一键回连；
/// 找不到时如实告知两条通道的差别与首次配对的正确入口。
/// 返回要追加进伴侣事件流的文案（不含配对码等敏感值）。
fn companion_bridge_events(adb: &dyn AdbRuntime, peer_ip: &str) -> Vec<String> {
    let raw = match adb.mdns_services() {
        Ok(raw) => raw,
        Err(error) => {
            return vec![format!(
                "自动检查镜像通道失败（{error}）。要镜像这台手机，请用数据线连接或在「无线」页完成配对。"
            )];
        }
    };
    let endpoints: Vec<String> = parse_mdns_entries(&raw)
        .iter()
        .filter(|entry| entry.service == "_adb-tls-connect._tcp")
        .map(|entry| entry.endpoint.clone())
        .filter(|endpoint| {
            endpoint
                .rsplit_once(':')
                .is_some_and(|(host, _)| host == peer_ip)
        })
        .collect();
    if endpoints.is_empty() {
        return vec![format!(
            "没有发现这台手机（{peer_ip}）的无线调试广播。伴侣扫码与镜像连接是两条独立通道：\
             首次使用请在连接页用数据线连接，或在「无线」页完成配对码配对。"
        )];
    }
    endpoints
        .iter()
        .map(|endpoint| match adb.connect(endpoint) {
            Ok(()) => format!("已自动连接镜像通道 {endpoint}，设备几秒内会出现在连接列表。"),
            Err(error) => format!("自动连接镜像通道 {endpoint} 失败：{error}。请在手机上确认无线调试已开启。"),
        })
        .collect()
}

/// 伴侣配对数据根目录（桌面身份 + 已配对设备台账同目录）。
fn companion_data_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("companion-identity"))
        .map_err(|_| {
            AppError::new(
                "settings_unavailable",
                "无法访问本机数据目录。",
                "请检查本机文件权限，或重新安装 MirrorDock。",
            )
        })
}

#[tauri::command]
fn companion_begin_pairing(
    state: State<'_, Arc<companion_pairing::PairingState>>,
    app: AppHandle,
) -> Result<companion_pairing::PairingOffer, AppError> {
    let dir = companion_data_dir(&app)?;
    companion_pairing::begin_pairing(&state, &dir, companion_pairing::PairedStore::new(&dir))
}

#[tauri::command]
fn companion_pairing_status(
    state: State<'_, Arc<companion_pairing::PairingState>>,
) -> companion_pairing::PairingStatus {
    state.status()
}

#[tauri::command]
fn companion_end_pairing(state: State<'_, Arc<companion_pairing::PairingState>>) {
    companion_pairing::end_pairing(&state);
}

/// 开启常驻通道（M4-2）：已配对伴侣设备可免扫码直连。返回监听端口。
#[tauri::command]
fn companion_begin_resident(
    state: State<'_, Arc<companion_pairing::PairingState>>,
    app: AppHandle,
) -> Result<u16, AppError> {
    let dir = companion_data_dir(&app)?;
    companion_pairing::begin_resident(&state, &dir, companion_pairing::PairedStore::new(&dir))
}

/// 关闭常驻通道（幂等；也用于结束扫码配对监听）。
#[tauri::command]
fn companion_end_resident(state: State<'_, Arc<companion_pairing::PairingState>>) {
    companion_pairing::end_pairing(&state);
}

/// 快捷回复内容校验（X10-69）：key 非空白且无控制字符（≤256 字节），text 非空、
/// 无控制字符、≤500 字符。通过后返回 trim 过的 (key, text)。
fn validate_notification_reply(key: &str, text: &str) -> Result<(String, String), AppError> {
    let key = key.trim();
    if key.is_empty() || key.len() > 256 || key.chars().any(|c| c.is_control()) {
        return Err(AppError::new(
            "endpoint_invalid",
            "通知标识无效，无法发送回复。",
            "请等待新的通知到达后再试。",
        ));
    }
    // 回复是一行输入：拒绝换行与控制字符；长度收紧到 500 字符（远低于行上限，
    // 也符合「快捷回复」的使用直觉）。
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::new(
            "endpoint_invalid",
            "回复内容为空。",
            "输入内容后再发送。",
        ));
    }
    if text.chars().count() > 500 || text.chars().any(|c| c.is_control()) {
        return Err(AppError::new(
            "endpoint_invalid",
            "回复内容过长或包含不允许的字符。",
            "请缩短内容（500 字以内）并去掉换行后重试。",
        ));
    }
    Ok((key.to_owned(), text.to_owned()))
}

/// 快捷回复（X10-69）：把用户输入的回复文本下发给手机伴侣端，由伴侣端填进
/// 对应通知的 RemoteInput 动作。只在本机点对点通道传输，不经过任何云端。
///
/// 返回 `sent=false` 表示当前没有在线的伴侣会话（手机不在线/常驻连接未建立），
/// 不是错误——前端据此提示用户稍后再试。
#[tauri::command]
fn companion_notification_reply(
    state: State<'_, Arc<companion_pairing::PairingState>>,
    log: State<DiagnosticsLog>,
    key: String,
    text: String,
) -> Result<bool, AppError> {
    let (key, text) = validate_notification_reply(&key, &text)?;
    // serde_json 负责转义，保证文本里的引号等不会破坏行协议。
    let line = serde_json::json!({ "type": "notification_reply", "key": key, "text": text });
    let sent = state.broadcast_line(line.to_string());
    log.record(
        "companion_notification_reply",
        if sent { "ok" } else { "no_active_session" },
        "回复已下发到伴侣端",
        &[],
    );
    Ok(sent)
}

/// 把录制状态变化推给当前活跃的伴侣会话（M4-3）。
/// 无活跃会话时静默忽略——伴侣通道是附加信息通道，不构成错误。
fn notify_companion_recording(app: &AppHandle, active: bool) {
    if let Some(state) = app.try_state::<Arc<companion_pairing::PairingState>>() {
        let line = format!("{{\"type\":\"recording\",\"active\":{active}}}");
        if state.broadcast_line(line) {
            if let Some(log) = app.try_state::<DiagnosticsLog>() {
                log.record(
                    "companion_notify_recording",
                    "ok",
                    if active { "已推送录制开始" } else { "已推送录制结束" },
                    &[],
                );
            }
        }
    }
}

/// 已配对的伴侣设备列表（M4-1 互信台账）。
#[tauri::command]
fn companion_paired_devices(app: AppHandle) -> Result<Vec<companion_pairing::PairedCompanion>, AppError> {
    let dir = companion_data_dir(&app)?;
    Ok(companion_pairing::PairedStore::new(&dir).load())
}

/// 移除一台已配对伴侣设备的互信（对端下次连接需重新扫码配对）。
#[tauri::command]
fn companion_unpair_device(
    app: AppHandle,
    pairing_id: String,
) -> Result<Vec<companion_pairing::PairedCompanion>, AppError> {
    let dir = companion_data_dir(&app)?;
    companion_pairing::PairedStore::new(&dir)
        .remove(&pairing_id)
        .map_err(|e| AppError {
            code: "companion_unpair_failed",
            message: format!("移除配对设备失败：{e}"),
            recovery: "检查本机文件权限后重试。".into(),
        })
}

/// 应用退出时回收子进程，避免残留 scrcpy 进程。优雅结束让录制文件有机会收尾。
/// X10-27：多会话并存时逐台回收。
fn reclaim_children(app: &AppHandle) {
    if let Some(store) = app.try_state::<SessionStore>() {
        if let Ok(processes) = take_all_running_processes(store.inner()) {
            for (_, mut process) in processes {
                let _ = process.stop();
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 运行时与伴侣配对状态先于 builder 创建：桥接回调要在 manage 之前装好，
    // 保证任何一次配对开始时「设备报到 → 自动连接镜像通道」都已生效。
    let runtimes = AppRuntimes::system();
    let pairing_state = Arc::new(companion_pairing::PairingState::default());
    let bridge_adb = Arc::clone(&runtimes.adb);
    pairing_state.set_device_bridge(Arc::new(move |peer_ip: &str| {
        companion_bridge_events(bridge_adb.as_ref(), peer_ip)
    }));

    tauri::Builder::default()
        .manage(SessionStore::default())
        .manage(runtimes)
        .manage(DiagnosticsLog::default())
        .manage(QrPairingStore::default())
        .manage(pairing_state)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // 应用内自动更新（X10-47）：更新包经 minisign 验签后才安装，
        // 公钥编译在 tauri.conf.json 里，端点指向本仓库 Release 的 latest.json。
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // 应用级设置（含 macOS 隐藏 Dock）要在任何窗口展示前生效。
            // setup 闭包的错误类型是 Box<dyn StdError>，而 AppError 没实现
            // StdError：这里失败只能说明 app_data_dir 不可用，转成字符串即可。
            let path = app_settings_path(app.handle())
                .map_err(|_| "无法访问本机设置目录".to_string())?;
            let settings = load_app_settings(&path);
            apply_dock_icon_policy(app.handle(), &settings);
            build_tray(app)?;
            // 监视线程需要能独立还原无线亮屏补偿；系统实现固定可用。
            let _ = MONITOR_ADB.set(Box::new(SystemAdbRuntime));
            // 上次会话若因应用被强杀而没来得及还原熄屏时间，这里补上。
            // 设备未连接时还原失败，备份文件保留，下次启动再试。
            if let Ok(backup_path) = keep_awake_backup_path(app.handle()) {
                restore_persisted_keep_awake(&SystemAdbRuntime, &backup_path);
            }
            // 上次运行若因崩溃残留了「临时切换的输入源」账本，这里恢复原输入法。
            if let Ok(path) = input_source_backup_path(app.handle()) {
                restore_persisted_input_source(app.handle(), &path);
            }
            // X10-84：后端设备状态 watcher——替代前端高频轮询 check_adb_devices。
            // 前端不再定时 invoke，而是订阅 devices-changed 事件；watcher 仅在设备
            // 集合变化时 emit，无变化只静默循环（adb 调用已带 ADB_CALL_TIMEOUT 超时，
            // 卡死兜底在上层，不再拖死 UI）。
            {
                let watch_handle = app.handle().clone();
                let runtimes = AppRuntimes::system();
                std::thread::spawn(move || {
                    let mut last_fingerprint = String::new();
                    loop {
                        let devices = match runtimes.adb.list_devices() {
                            Ok(devices) => devices,
                            Err(_) => {
                                std::thread::sleep(Duration::from_secs(3));
                                continue;
                            }
                        };
                        // 指纹只含序列号+状态：设备插拔/授权变化即触发，普通属性不扰动。
                        let mut fingerprint: Vec<String> = devices
                            .iter()
                            .map(|device| format!("{}:{:?}", device.serial, device.state))
                            .collect();
                        fingerprint.sort();
                        let fingerprint = fingerprint.join("|");
                        if fingerprint != last_fingerprint {
                            last_fingerprint = fingerprint;
                            let _ = watch_handle.emit("devices-changed", ());
                        }
                        std::thread::sleep(Duration::from_secs(2));
                    }
                });
            }
            // 伴侣端上行事件转发（X10-60/X10-66）：发送区新文件 / 崩溃堆栈 /
            // 手机通知 → 前端。通知内容只进前端内存（不落盘、不进日志）。
            let hook_handle = app.handle().clone();
            if let Some(state) = app.try_state::<Arc<companion_pairing::PairingState>>() {
                state.set_event_hook(Arc::new(move |value: &serde_json::Value| {
                    match value.get("type").and_then(|t| t.as_str()) {
                        Some("files_changed") => {
                            let _ = hook_handle.emit("companion-files-changed", ());
                        }
                        Some("last_crash") => {
                            let stack = value.get("stack").and_then(|s| s.as_str()).unwrap_or("");
                            let _ = hook_handle.emit(
                                "companion-crash-report",
                                serde_json::json!({ "stack": stack }),
                            );
                        }
                        Some("notification") => {
                            let str_field = |key: &str| {
                                value.get(key).and_then(|v| v.as_str()).unwrap_or("").to_owned()
                            };
                            let _ = hook_handle.emit(
                                "companion-notification",
                                serde_json::json!({
                                    "pkg": str_field("pkg"),
                                    "app": str_field("app"),
                                    "title": str_field("title"),
                                    "text": str_field("text"),
                                    "posted": value.get("posted").and_then(|v| v.as_u64()).unwrap_or(0),
                                    // X10-69 快捷回复：key 用于定位手机上的原通知；
                                    // replyable=false（旧版伴侣端没有这两个字段）时前端不显示回复框。
                                    "key": str_field("key"),
                                    "replyable": value.get("replyable").and_then(|v| v.as_bool()).unwrap_or(false),
                                }),
                            );
                        }
                        _ => {}
                    }
                }));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 点关闭 = 最小化到菜单栏/托盘，不退出：从菜单栏图标可以随时回到
            // 主窗口，退出走托盘菜单（明确的出口只有一个，避免「关了又在后台」
            // 与「想最小化却把会话杀了」两种误伤）。
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            check_adb_devices,
            probe_device_capabilities,
            start_mirroring,
            stop_mirroring,
            mirror_session,
            mirror_sessions,
            update_session_options,
            wake_device,
            open_keyboard_settings,
            restart_app,
            device_lock_report,
            probe_pin_pad_state,
            list_recent_devices,
            forget_recent_device,
            clear_recent_devices,
            get_desktop_prefs,
            set_desktop_prefs,
            revoke_device_access,
            capture_screenshot,
            delete_screenshot,
            send_file_to_device,
            list_device_files,
            list_device_apps,
            fetch_file_from_device,
            delete_device_file,
            install_apk_to_device,
            current_recording,
            start_recording,
            stop_recording,
            delete_recording,
            pair_wireless_device,
            connect_wireless_device,
            discover_pairing_services,
            begin_qr_pairing,
            qr_pairing_progress,
            end_qr_pairing,
            list_trusted_wireless_devices,
            forget_trusted_wireless_device,
            diagnostics_preview,
            export_diagnostics,
            entitlement_status,
            entitlement_activate,
            entitlement_deactivate,
            companion_begin_pairing,
            companion_pairing_status,
            companion_end_pairing,
            companion_begin_resident,
            companion_end_resident,
            companion_notification_reply,
            companion_paired_devices,
            companion_unpair_device,
            get_app_settings,
            set_app_settings
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                // `code == None` 是「最后一个窗口被关闭/销毁」触发的**隐式退出
                // 请求**：一律拒绝（2026-10-01 用户报「关闭镜像窗口后整个客户端
                // 被关掉」，此为系统性兜底）。应用的唯一出口是托盘菜单的退出项
                // ——它走 `app.exit(0)`，`code = Some(0)`，不命中本臂（落入后面的
                // 空臂，行为与原来的 if 判断完全一致）。
                // 副作用：macOS 的 Cmd+Q 也不再直接杀进程（与「明确出口只有一个」
                // 的设计一致：误触 Cmd+Q 本会连带杀掉镜像会话）。
                tauri::RunEvent::ExitRequested { code, api, .. } if code.is_none() => {
                    api.prevent_exit();
                }
                tauri::RunEvent::Exit => {
                    reclaim_children(app_handle);
                    // 应用退出：还原**所有**设备的无线亮屏补偿（X10-27 多会话）。
                    // 失败时备份文件还在，下次启动会再试——不因还原失败阻塞退出。
                    if let Some(sessions) = app_handle.try_state::<SessionStore>() {
                        disable_all_wireless_keep_awake(&SystemAdbRuntime, &sessions);
                    }
                }
                _ => {}
            }
        });
}


    // -- shell_quote（X10-77）------------------------------------------------
    //
    // 回归测试：2026-10-03 真机发现「删除含空格的文件」静默失败 ——
    // `adb shell rm -f -- <含空格路径>` 把路径拆成多个词，rm -f 对不存在的
    // 路径返回 0，于是文件没删掉、后端却报成功，UI 提示"已删除"但列表不变。
    #[test]
    fn shell_quote_protects_paths_from_adb_shell_resplitting() {
        // 基本情形：包起来就够。
        assert_eq!(shell_quote("/sdcard/Dir/a.txt"), "'/sdcard/Dir/a.txt'");
        // 关键回归：含空格。远端 shell 必须把它当**一个**词。
        assert_eq!(
            shell_quote("/sdcard/Download/MirrorDock/has space.txt"),
            "'/sdcard/Download/MirrorDock/has space.txt'"
        );
        // 中文文件名（真实数据里就有 `MirrorDock 传输 冒烟.txt`）。
        assert_eq!(
            shell_quote("/sdcard/Download/MirrorDock/MirrorDock 传输 冒烟.txt"),
            "'/sdcard/Download/MirrorDock/MirrorDock 传输 冒烟.txt'"
        );
        // 最恶劣情况：内嵌单引号。用 POSIX 的闭合-转义-重开，不能直接套。
        assert_eq!(shell_quote("it's.txt"), r#"'it'"'"'s.txt'"#);
        // shell 元字符在单引号内是字面量 —— 顺带消除注入面。
        assert_eq!(shell_quote("a;rm -rf /"), "'a;rm -rf /'");
        assert_eq!(shell_quote("$(whoami)"), "'$(whoami)'");
        assert_eq!(shell_quote("`id`"), "'`id`'");
    }

    // 接线测试：断言 `adb rm` 的 argv **真的带上了引号**。
    //
    // 为什么单独一条：只测 `shell_quote` 这个纯函数是不够的 —— 2026-10-03
    // 我先写了纯函数测试，**反向验证时把 `shell_quote(remote_path)` 改回
    // `remote_path`，测试依然全绿**，因为纯函数本身没变、只是没人调用它了。
    // 这类「只测实现、不测接线」的盲区与 v0.4.7 那次同源。
    #[test]
    fn remove_device_file_argv_actually_quotes_the_path() {
        let argv = remove_device_file_argv("phone", "/sdcard/D/MirrorDock 传输 冒烟.txt");
        // 前 6 段是固定前缀。
        assert_eq!(&argv[..6], &[
            "-s", "phone", "shell", "rm", "-f", "--"
        ].map(String::from));
        // 关键：最后一段必须是**带引号的完整路径**，不能是裸路径。
        let last = argv.last().expect("argv 非空");
        assert_eq!(last, "'/sdcard/D/MirrorDock 传输 冒烟.txt'");
        // 反向断言：裸路径形态（修复前的 bug）必须不等于引号形态。
        assert_ne!(last, &"/sdcard/D/MirrorDock 传输 冒烟.txt".to_string());
        // 引号内不含未转义的单引号 —— 否则远端 shell 会报 no closing quote。
        let inner = last.trim_matches('\'');
        assert!(!inner.contains('\''), "引号内出现未转义单引号: {inner}");
    }

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    // -- 可替换实现的假运行时：无需真机即可验证会话状态机与错误码 --

    #[derive(Default)]
    struct FakeAdb {
        devices: Vec<AdbDevice>,
        /// 允许测试在**运行中**改变设备列表（例如模拟「会话进行到一半手机被拔掉」）。
        live_devices: Option<Arc<Mutex<Vec<AdbDevice>>>>,
        unavailable: bool,
        /// `adb shell getprop` 的原始输出；`None` 表示读取失败。
        properties: Option<String>,
        /// `dumpsys window policy` 的原始输出；`None` 表示读取失败。
        window_policy_dump: Option<String>,
        /// `dumpsys power` 的原始输出；`None` 表示读取失败。
        power_dump: Option<String>,
        /// `dumpsys display` 的原始输出；`None` 表示读取失败。用互斥包一层，
        /// 让 `press_key` 能在测试中模拟「注入按键改变了显示策略」。
        display_dump: Arc<Mutex<Option<String>>>,
        /// 为真（Some）时，`press_key` 后显示策略被改写为该值（模拟真实设备响应）。
        policy_after_key: Option<DisplayPolicy>,
        /// `screencap -p` 返回的字节流；`None` 表示读取失败。
        screenshot: Option<Vec<u8>>,
        /// `screencap_probe_bytes` 返回的字节数；`None` 表示探测失败。
        probe_bytes: Option<u64>,
        /// 设备传输目录的 `ls -1` 输出；`None` 表示读取失败。
        device_listing: Option<String>,
        /// 为真时传输类调用（mkdir/push/pull）一律失败。
        transfer_fails: bool,
        /// `adb install` 的原始输出（stdout+stderr 合并）；`None` 表示进程本身失败。
        install_output: Option<String>,
        /// 设备当前的熄屏时间（毫秒）；`None` 表示读取失败或返回 `null`。
        screen_timeout: Arc<Mutex<Option<u64>>>,
        /// 设备当前的「充电时保持唤醒」位掩码；`None` 表示读取失败或返回 `null`。
        stay_on_bits: Arc<Mutex<Option<u64>>>,
        /// 为真时一切设备侧写入失败（模拟设备离线）。用互斥包一层，测试可在
        /// 会话中途翻转（先成功补偿、再模拟离线还原失败）。
        device_writes_fail: Arc<Mutex<bool>>,
        /// `adb mdns services` 的原始输出；`None` 表示空列表（无服务广播）。
        mdns_output: Option<String>,
        /// `list_device_apps` 的 canned 输出（X10-53 解析测试用）。
        apps_output: Option<String>,
        /// 为真时 `connect` 一律失败（模拟无线调试已关/未配对）。
        connect_fails: bool,
        /// `pull_file` 成功时写进本机文件的内容，用于验证回执字节数。
        pulled_contents: Vec<u8>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeAdb {
        fn with_devices(devices: Vec<AdbDevice>) -> Self {
            Self {
                devices,
                ..Self::default()
            }
        }

        /// 返回一个可在测试中持续改写的设备列表句柄。
        fn with_live_devices(devices: Vec<AdbDevice>) -> (Self, Arc<Mutex<Vec<AdbDevice>>>) {
            let live = Arc::new(Mutex::new(devices));
            (
                Self {
                    live_devices: Some(Arc::clone(&live)),
                    ..Self::default()
                },
                live,
            )
        }

        fn with_capabilities(devices: Vec<AdbDevice>, properties: &str) -> Self {
            Self {
                devices,
                properties: Some(properties.to_owned()),
                ..Self::default()
            }
        }

        fn with_lock_state(
            devices: Vec<AdbDevice>,
            window_policy: &str,
            power: &str,
        ) -> Self {
            Self {
                devices,
                window_policy_dump: Some(window_policy.to_owned()),
                power_dump: Some(power.to_owned()),
                ..Self::default()
            }
        }

        /// 一台能正常返回 PNG 快照的设备。
        fn with_screenshot(devices: Vec<AdbDevice>, screenshot: Vec<u8>) -> Self {
            Self {
                devices,
                screenshot: Some(screenshot),
                ..Self::default()
            }
        }

        /// 一台返回了非图片字节流的设备（模拟 `exec-out` 被文本模式改写）。
        fn returning_corrupt_screenshot(devices: Vec<AdbDevice>) -> Self {
            Self {
                devices,
                screenshot: Some(b"adb: not an image".to_vec()),
                ..Self::default()
            }
        }

        fn unavailable() -> Self {
            Self {
                unavailable: true,
                ..Self::default()
            }
        }

        /// connect 一律失败的设备（模拟无线调试关闭或未配对）。
        fn with_connect_failure() -> Self {
            Self {
                connect_fails: true,
                ..Self::default()
            }
        }

        /// 设备侧写入一律失败的设备（模拟离线：取消授权各步都会失败）。
        fn with_device_write_failure() -> Self {
            Self {
                device_writes_fail: Arc::new(Mutex::new(true)),
                ..Self::default()
            }
        }
    }

    impl AdbRuntime for FakeAdb {
        fn list_devices(&self) -> Result<Vec<AdbDevice>, std::io::Error> {
            if let Some(live) = &self.live_devices {
                return Ok(live.lock().unwrap().clone());
            }
            if self.unavailable {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            } else {
                Ok(self.devices.clone())
            }
        }

        fn device_properties(&self, serial: &str) -> Result<String, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("properties {serial}"));
            match &self.properties {
                Some(properties) => Ok(properties.clone()),
                None => Err(std::io::Error::other("properties unavailable")),
            }
        }

        fn physical_serial(&self, _serial: &str) -> Result<Option<String>, std::io::Error> {
            Ok(None)
        }

        fn wake_screen(&self, serial: &str) -> Result<(), std::io::Error> {
            self.calls.lock().unwrap().push(format!("wake {serial}"));
            Ok(())
        }

        fn screen_off_timeout(&self, serial: &str) -> Result<Option<u64>, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("get_timeout {serial}"));
            Ok(*self.screen_timeout.lock().unwrap())
        }

        fn set_screen_off_timeout(&self, serial: &str, millis: u64) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("set_timeout {serial} {millis}"));
            if *self.device_writes_fail.lock().unwrap() {
                Err(std::io::Error::other("device offline"))
            } else {
                *self.screen_timeout.lock().unwrap() = Some(millis);
                Ok(())
            }
        }

        fn stay_on_while_plugged_in(&self, serial: &str) -> Result<Option<u64>, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("get_stay_on {serial}"));
            Ok(*self.stay_on_bits.lock().unwrap())
        }

        fn set_stay_on_while_plugged_in(
            &self,
            serial: &str,
            bits: u64,
        ) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("set_stay_on {serial} {bits}"));
            if *self.device_writes_fail.lock().unwrap() {
                Err(std::io::Error::other("device offline"))
            } else {
                *self.stay_on_bits.lock().unwrap() = Some(bits);
                Ok(())
            }
        }

        fn window_policy(&self, serial: &str) -> Result<String, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("window_policy {serial}"));
            match &self.window_policy_dump {
                Some(dump) => Ok(dump.clone()),
                None => Err(std::io::Error::other("window policy unavailable")),
            }
        }

        fn power_state(&self, serial: &str) -> Result<String, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("power_state {serial}"));
            match &self.power_dump {
                Some(dump) => Ok(dump.clone()),
                None => Err(std::io::Error::other("power state unavailable")),
            }
        }

        fn display_state(&self, serial: &str) -> Result<String, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("display_state {serial}"));
            match self.display_dump.lock().unwrap().clone() {
                Some(dump) => Ok(dump),
                None => Err(std::io::Error::other("display state unavailable")),
            }
        }

        fn press_key(&self, serial: &str, keycode: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("key {keycode} {serial}"));
            // 模拟「注入按键改变了显示策略」：BACK 恢复到 policy_after_key，
            // SLEEP 进 Other（熄屏）——与真机行为一致的简化模型。
            if let Some(target) = &self.policy_after_key {
                let next = match keycode {
                    "KEYCODE_SLEEP" => DisplayPolicy::Other,
                    _ => *target,
                };
                let text = format!(
                    "  mPowerRequest=policy={}\n",
                    match next {
                        DisplayPolicy::Bright => "BRIGHT",
                        DisplayPolicy::Dim => "DIM",
                        DisplayPolicy::Other => "OFF",
                    }
                );
                let mut slot = self.display_dump.lock().unwrap();
                if let Some(dump) = slot.as_mut() {
                    *dump = text;
                } else {
                    *slot = Some(text);
                }
            }
            Ok(())
        }

        fn screenshot_png(&self, serial: &str) -> Result<Vec<u8>, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("screenshot {serial}"));
            match &self.screenshot {
                Some(bytes) => Ok(bytes.clone()),
                None => Err(std::io::Error::other("screenshot unavailable")),
            }
        }

        fn screencap_probe_bytes(&self, serial: &str) -> Result<u64, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("probe {serial}"));
            match self.probe_bytes {
                Some(bytes) => Ok(bytes),
                None => Err(std::io::Error::other("probe unavailable")),
            }
        }

        fn make_directory(&self, serial: &str, remote_dir: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("mkdir {serial} {remote_dir}"));
            if self.transfer_fails {
                Err(std::io::Error::other("mkdir failed"))
            } else {
                Ok(())
            }
        }

        fn list_directory(&self, serial: &str, remote_dir: &str) -> Result<String, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("ls {serial} {remote_dir}"));
            match &self.device_listing {
                Some(raw) => Ok(raw.clone()),
                None => Err(std::io::Error::other("listing unavailable")),
            }
        }

        fn push_file(
            &self,
            serial: &str,
            local: &Path,
            remote_dir: &str,
        ) -> Result<String, std::io::Error> {
            self.calls.lock().unwrap().push(format!(
                "push {serial} {} {remote_dir}",
                local.to_string_lossy()
            ));
            if self.transfer_fails {
                Err(std::io::Error::other("push failed"))
            } else {
                Ok("1 file pushed".to_owned())
            }
        }

        fn pull_file(
            &self,
            serial: &str,
            remote_path: &str,
            local: &Path,
        ) -> Result<String, std::io::Error> {
            self.calls.lock().unwrap().push(format!(
                "pull {serial} {remote_path} {}",
                local.to_string_lossy()
            ));
            if self.transfer_fails {
                return Err(std::io::Error::other("pull failed"));
            }
            // 模拟 adb pull 的落盘结果，让上层对回执字节数的校验可以走到真实路径。
            let contents: &[u8] = if self.pulled_contents.is_empty() {
                b"device-bytes"
            } else {
                &self.pulled_contents
            };
            fs::write(local, contents).map_err(|_| std::io::Error::other("cannot write pulled file"))?;
            Ok("1 file pulled".to_owned())
        }

        fn remove_device_file(&self, serial: &str, remote_path: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("rm {serial} {remote_path}"));
            if self.transfer_fails {
                Err(std::io::Error::other("rm failed"))
            } else {
                Ok(())
            }
        }

        fn install_apk(&self, serial: &str, apk: &Path) -> Result<String, std::io::Error> {
            self.calls.lock().unwrap().push(format!(
                "install {serial} {}",
                apk.to_string_lossy()
            ));
            match &self.install_output {
                Some(raw) => Ok(raw.clone()),
                // 进程本身没跑起来（如 adb 缺失）：与真实实现一致，返回 Err。
                None => Err(std::io::Error::other("adb is not available")),
            }
        }

        fn pair(&self, endpoint: &str, pairing_code: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("pair {endpoint} {pairing_code}"));
            Ok(())
        }
        fn connect(&self, endpoint: &str) -> Result<(), std::io::Error> {
            self.calls.lock().unwrap().push(format!("connect {endpoint}"));
            if self.connect_fails {
                Err(std::io::Error::other("failed to connect to endpoint"))
            } else {
                Ok(())
            }
        }

        fn disconnect(&self, endpoint: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("disconnect {endpoint}"));
            Ok(())
        }

        fn open_developer_settings(&self, serial: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("open_developer_settings {serial}"));
            if *self.device_writes_fail.lock().unwrap() {
                return Err(std::io::Error::other("device offline"));
            }
            Ok(())
        }

        fn open_keyboard_layout_settings(&self, serial: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("open_keyboard_layout_settings {serial}"));
            if *self.device_writes_fail.lock().unwrap() {
                return Err(std::io::Error::other("device offline"));
            }
            Ok(())
        }

        fn disable_wireless_debugging(&self, serial: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("disable_wireless_debugging {serial}"));
            if *self.device_writes_fail.lock().unwrap() {
                return Err(std::io::Error::other("device offline"));
            }
            Ok(())
        }

        fn disable_usb_debugging(&self, serial: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("disable_usb_debugging {serial}"));
            if *self.device_writes_fail.lock().unwrap() {
                return Err(std::io::Error::other("device offline"));
            }
            Ok(())
        }

        fn mdns_services(&self) -> Result<String, std::io::Error> {
            Ok(self.mdns_output.clone().unwrap_or_default())
        }

        fn list_device_apps(&self, serial: &str) -> Result<String, std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("list_device_apps {serial}"));
            Ok(self.apps_output.clone().unwrap_or_default())
        }
    }

    struct FakeProcess {
        /// `None` 表示进程持续运行；`Some(success)` 表示下一次轮询即报告退出。
        exit: Option<bool>,
        killed: Arc<Mutex<bool>>,
    }

    impl MirrorProcess for FakeProcess {
        fn try_wait(&mut self) -> Option<bool> {
            self.exit.take()
        }

        fn kill(&mut self) -> Result<(), std::io::Error> {
            *self.killed.lock().unwrap() = true;
            self.exit = None;
            Ok(())
        }
    }

    struct FakeMirror {
        available: bool,
        exit: Option<bool>,
        killed: Arc<Mutex<bool>>,
        /// 每次 `start` 实际收到的启动参数，用于验证「会话中应用设置」确实按新参数重启。
        starts: Arc<Mutex<Vec<SessionOptions>>>,
        /// 每次 `start` 收到的录制路径，用于验证录制确实被传给了镜像进程。
        records: Arc<Mutex<Vec<Option<String>>>>,
        /// 为真时 `start` 直接失败，用于验证重启失败不会塌缩会话状态。
        fail_start: bool,
        /// X10-92：录制通道（`start_recorder`）收到的参数与录制路径。
        recorder_starts: Arc<Mutex<Vec<SessionOptions>>>,
        recorder_records: Arc<Mutex<Vec<String>>>,
    }

    impl FakeMirror {
        /// 启动后持续运行的镜像进程。
        fn running() -> Self {
            Self {
                available: true,
                exit: None,
                killed: Arc::new(Mutex::new(false)),
                starts: Arc::new(Mutex::new(Vec::new())),
                records: Arc::new(Mutex::new(Vec::new())),
                fail_start: false,
                recorder_starts: Arc::new(Mutex::new(Vec::new())),
                recorder_records: Arc::new(Mutex::new(Vec::new())),
            }
        }

        /// 启动后立即以给定退出码结束的镜像进程。
        fn exiting(success: bool) -> Self {
            Self {
                available: true,
                exit: Some(success),
                killed: Arc::new(Mutex::new(false)),
                starts: Arc::new(Mutex::new(Vec::new())),
                records: Arc::new(Mutex::new(Vec::new())),
                fail_start: false,
                recorder_starts: Arc::new(Mutex::new(Vec::new())),
                recorder_records: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn unavailable() -> Self {
            Self {
                available: false,
                exit: None,
                killed: Arc::new(Mutex::new(false)),
                starts: Arc::new(Mutex::new(Vec::new())),
                records: Arc::new(Mutex::new(Vec::new())),
                fail_start: false,
                recorder_starts: Arc::new(Mutex::new(Vec::new())),
                recorder_records: Arc::new(Mutex::new(Vec::new())),
            }
        }

        /// 可用但一旦尝试启动就失败，用于验证「重启后启动失败」的路径。
        fn failing_start() -> Self {
            Self {
                fail_start: true,
                ..Self::running()
            }
        }
    }

    impl MirrorRuntime for FakeMirror {
        fn is_available(&self) -> bool {
            self.available
        }

        fn start(
            &self,
            _serial: &str,
            options: &SessionOptions,
            record_path: Option<&Path>,
        ) -> Result<Box<dyn MirrorProcess>, std::io::Error> {
            self.starts.lock().unwrap().push(options.clone());
            self.records
                .lock()
                .unwrap()
                .push(record_path.map(|path| path.to_string_lossy().into_owned()));
            if self.fail_start {
                return Err(std::io::Error::other("mirror start failed"));
            }
            Ok(Box::new(FakeProcess {
                exit: self.exit,
                killed: Arc::clone(&self.killed),
            }))
        }

        fn start_recorder(
            &self,
            _serial: &str,
            options: &SessionOptions,
            record_path: &Path,
            _desktop_display_id: Option<u32>,
        ) -> Result<Box<dyn MirrorProcess>, std::io::Error> {
            // 记录录制进程的参数与路径，供测试断言「双通道用 record_arguments 且独立」。
            self.recorder_starts.lock().unwrap().push(options.clone());
            self.recorder_records
                .lock()
                .unwrap()
                .push(record_path.to_string_lossy().into_owned());
            if self.fail_start {
                return Err(std::io::Error::other("recorder start failed"));
            }
            Ok(Box::new(FakeProcess {
                exit: self.exit,
                killed: Arc::clone(&self.killed),
            }))
        }
    }

    fn runtimes(adb: FakeAdb, mirror: FakeMirror) -> AppRuntimes {
        AppRuntimes {
            adb: Arc::new(adb),
            mirror: Box::new(mirror),
            serial_cache: Mutex::new(HashMap::new()),
        }
    }

    fn device(serial: &str, state: DeviceState) -> AdbDevice {
        let kind = if is_wireless_endpoint(serial) {
            ConnectionKind::Wireless
        } else {
            ConnectionKind::Usb
        };
        AdbDevice {
            serial: serial.to_owned(),
            label: "测试设备".to_owned(),
            state,
            physical_serial: None,
            connections: vec![ConnectionEndpoint {
                serial: serial.to_owned(),
                kind,
                state,
            }],
        }
    }

    fn snapshot(store: &SessionStore) -> MirrorSession {
        let map = store
            .lock()
            .expect("session state must be readable in tests");
        match primary_session_serial(&map) {
            Some(serial) => map[&serial].session.clone(),
            None => MirrorSession::idle(),
        }
    }

    /// 手工写入一个「进程正在运行」的会话，用于测试无法走完首次启动的路径。
    fn mark_running_for_test(store: &SessionStore, options: SessionOptions) {
        let mut map = store.lock().unwrap();
        let state = session_entry_mut(&mut map, "phone");
        state.epoch = state.epoch.wrapping_add(1);
        state.session = MirrorSession::streaming("phone".into());
        state.process = Some(Box::new(FakeProcess {
            exit: None,
            killed: Arc::new(Mutex::new(false)),
        }));
        state.options = options;
    }

    /// 读取某台设备的亮屏补偿账本（测试断言用）。
    fn keep_awake_backup_of(store: &SessionStore, serial: &str) -> Option<KeepAwakeBackup> {
        store
            .lock()
            .unwrap()
            .get(serial)
            .and_then(|state| state.keep_awake_backup.clone())
    }

    fn wait_until_idle_or_failed(store: &SessionStore) -> MirrorSession {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let session = snapshot(store);
            if matches!(session.phase, SessionPhase::Idle | SessionPhase::Failed) {
                return session;
            }
            assert!(
                Instant::now() < deadline,
                "监视线程未在时限内更新会话状态：{session:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    // -- 设备解析 --

    #[test]
    fn notification_reply_validation_accepts_normal_input() {
        let (key, text) =
            validate_notification_reply(" 0|com.example.chat|0|1234567|null|1234 ", " 你好，稍后回复 ").unwrap();
        assert_eq!(key, "0|com.example.chat|0|1234567|null|1234");
        assert_eq!(text, "你好，稍后回复");
    }

    #[test]
    fn notification_reply_validation_rejects_bad_key_and_text() {
        // key：空、超长、控制字符。
        assert!(validate_notification_reply("", "hi").is_err());
        assert!(validate_notification_reply(&"k".repeat(257), "hi").is_err());
        assert!(validate_notification_reply("bad\nkey", "hi").is_err());
        // text：空、超长（按字符数算，中文同样受限）、控制字符。
        assert!(validate_notification_reply("ok-key", "   ").is_err());
        assert!(validate_notification_reply("ok-key", &"长".repeat(501)).is_err());
        assert!(validate_notification_reply("ok-key", "line1\nline2").is_err());
    }

    #[test]
    fn notification_reply_validation_allows_500_chars() {
        let text = "字".repeat(500);
        assert!(validate_notification_reply("ok-key", &text).is_ok());
    }

    #[test]
    fn parses_ready_and_unauthorized_devices() {
        let devices = parse_adb_devices(
            "List of devices attached\nR5CT1 device product:foo model:Pixel_8 device:shiba transport_id:1\nABC unauthorized usb:1-1\n",
        );

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].label, "Pixel 8");
        assert_eq!(devices[0].state, DeviceState::Ready);
        assert_eq!(devices[1].state, DeviceState::Unauthorized);
    }

    #[test]
    fn ignores_adb_headers_and_blank_lines() {
        let devices =
            parse_adb_devices("* daemon started successfully\nList of devices attached\n\n");

        assert!(devices.is_empty());
    }

    #[test]
    fn recognizes_only_ready_device_for_a_requested_serial() {
        let devices = parse_adb_devices(
            "List of devices attached\nready device model:Pixel_8\nwaiting unauthorized model:Pixel_7\n",
        );

        assert!(devices
            .iter()
            .any(|device| device.serial == "ready" && device.state == DeviceState::Ready));
        assert!(!devices
            .iter()
            .any(|device| device.serial == "waiting" && device.state == DeviceState::Ready));
    }

    #[test]
    fn wireless_success_requires_exact_ready_endpoint() {
        let devices = parse_adb_devices(
            "List of devices attached\n192.168.1.2:1234 offline\n192.168.1.2:4321 device\n",
        );
        assert!(!endpoint_is_ready(&devices, "192.168.1.2:1234"));
        assert!(!endpoint_is_ready(&devices, "192.168.1.3:4321"));
        assert!(endpoint_is_ready(&devices, "192.168.1.2:4321"));
        assert!(validate_endpoint("192.168.1.2:0").is_err());
    }

    // 真机回归（X10-25）：无线调试会在 adb devices 里多出一条 mDNS 发现条目
    // `adb-…._adb-tls-connect._tcp`（不含冒号），此前被误判为 USB 通道，
    // 导致纯 Wi-Fi 设备被标成「USB + 无线」。锁定：mDNS 条目归无线、
    // 合并后首选端点取 `IP:端口` 而非 mDNS 名、且不含 USB 通道。
    #[test]
    fn mdns_tls_entry_is_wireless_and_never_usb() {
        let devices = parse_adb_devices(
            "List of devices attached\n\
             adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp device product:chopin model:M2104K10AC\n\
             192.168.2.224:46289 device product:chopin model:M2104K10AC\n",
        );
        for device in &devices {
            assert_eq!(
                device.connections[0].kind,
                ConnectionKind::Wireless,
                "serial {} 应归为无线通道",
                device.serial
            );
        }
        // 同一台物理设备（相同硬件序列号）的两条无线通道应合并，且首选 IP:端口。
        let merged = vec![
            AdbDevice {
                serial: "adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp".into(),
                label: "M2104K10AC".into(),
                state: DeviceState::Ready,
                physical_serial: Some("PHYS-1".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Ready,
                }],
            },
            AdbDevice {
                serial: "192.168.2.224:46289".into(),
                label: "M2104K10AC".into(),
                state: DeviceState::Ready,
                physical_serial: Some("PHYS-1".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "192.168.2.224:46289".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Ready,
                }],
            },
        ];
        let mut result = dedup_devices(merged);
        assert_eq!(result.len(), 1);
        let device = result.pop().unwrap();
        assert_eq!(device.serial, "192.168.2.224:46289");
        assert!(!device_has_usb(&device));
        assert_eq!(device.connections.len(), 2);
    }

    // 伴侣会话 → 镜像通道桥接（X10-31）：设备报到后按来源 IP 自动回连。

    #[test]
    fn companion_bridge_connects_wireless_service_matching_peer_ip() {
        let adb = FakeAdb {
            mdns_output: Some(
                "adb-abc-rQ._adb-tls-connect._tcp.\t_adb-tls-connect._tcp.\t192.168.2.224:46289\n\
                 adb-other-yZ._adb-tls-connect._tcp.\t_adb-tls-connect._tcp.\t192.168.2.9:40001\n"
                    .into(),
            ),
            ..FakeAdb::default()
        };
        let events = companion_bridge_events(&adb, "192.168.2.224");
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("已自动连接镜像通道 192.168.2.224:46289"), "{events:?}");
        // 只对匹配来源 IP 的端点发起 connect，别的设备不受影响。
        let calls = adb.calls.lock().unwrap().clone();
        assert_eq!(calls, vec!["connect 192.168.2.224:46289".to_string()]);
    }

    #[test]
    fn companion_bridge_reports_missing_service_instead_of_connecting() {
        let adb = FakeAdb::default();
        let events = companion_bridge_events(&adb, "192.168.2.224");
        assert!(events[0].contains("没有发现"), "{events:?}");
        assert!(events[0].contains("两条独立通道"), "{events:?}");
        assert!(adb.calls.lock().unwrap().is_empty(), "未发现服务时不得发起 connect");
    }

    #[test]
    fn companion_bridge_does_not_match_other_devices_ip() {
        let adb = FakeAdb {
            mdns_output: Some(
                "adb-abc-rQ._adb-tls-connect._tcp.\t_adb-tls-connect._tcp.\t192.168.2.9:40001\n".into(),
            ),
            ..FakeAdb::default()
        };
        let events = companion_bridge_events(&adb, "192.168.2.224");
        assert!(events[0].contains("没有发现"), "{events:?}");
        assert!(adb.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn companion_bridge_reports_connect_failure_honestly() {
        let adb = FakeAdb {
            mdns_output: Some(
                "adb-abc-rQ._adb-tls-connect._tcp.\t_adb-tls-connect._tcp.\t192.168.2.224:46289\n".into(),
            ),
            ..FakeAdb::with_connect_failure()
        };
        let events = companion_bridge_events(&adb, "192.168.2.224");
        assert!(events[0].contains("自动连接镜像通道 192.168.2.224:46289 失败"), "{events:?}");
    }

    // 客户端侧取消授权（X10-32）。

    #[test]
    fn revoke_runs_every_step_and_reports_honestly() {
        let adb = FakeAdb::with_devices(vec![device("79j7kn9tkjt8rwss", DeviceState::Ready)]);
        let endpoints = vec!["79j7kn9tkjt8rwss".to_string()];
        let receipt = revoke_device_access_with(&AppRuntimes {
            adb: Arc::new(adb),
            mirror: Box::new(FakeMirror::running()),
            serial_cache: Mutex::new(HashMap::new()),
        }, &endpoints);
        let joined = receipt.steps.join("\n");
        assert!(joined.contains("已在手机上打开「开发者选项」"), "{joined}");
        assert!(joined.contains("已关闭手机上的「无线调试」"), "{joined}");
        assert!(joined.contains("已关闭手机上的「USB 调试」"), "{joined}");
        assert!(joined.contains("撤销 USB 调试授权"), "{joined}");
    }

    #[test]
    fn revoke_collects_endpoints_usb_first() {
        let devices = vec![AdbDevice {
            serial: "79j7kn9tkjt8rwss".into(),
            label: "M2104K10AC".into(),
            state: DeviceState::Ready,
            physical_serial: Some("PHYS-1".into()),
            connections: vec![
                ConnectionEndpoint {
                    serial: "79j7kn9tkjt8rwss".into(),
                    kind: ConnectionKind::Usb,
                    state: DeviceState::Ready,
                },
                ConnectionEndpoint {
                    serial: "192.168.2.90:41901".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Ready,
                },
            ],
        }];
        // 用无线端点查也要能找到这台手机。
        let endpoints = device_endpoints_for(&devices, "192.168.2.90:41901").unwrap();
        assert_eq!(endpoints[0], "79j7kn9tkjt8rwss", "USB 通道排最前");
        assert!(endpoints.contains(&"192.168.2.90:41901".to_string()));
        assert!(device_endpoints_for(&devices, "missing").is_none());
    }

    #[test]
    fn revoke_falls_back_to_next_endpoint_when_one_fails() {
        // 无线端点的设置写入失败时，用 USB 端点兜底完成。
        struct WirelessOnlyFails;
        impl AdbRuntime for WirelessOnlyFails {
            fn list_devices(&self) -> Result<Vec<AdbDevice>, std::io::Error> { Ok(Vec::new()) }
            fn device_properties(&self, _serial: &str) -> Result<String, std::io::Error> { Err(std::io::Error::other("x")) }
            fn physical_serial(&self, _serial: &str) -> Result<Option<String>, std::io::Error> { Ok(None) }
            fn wake_screen(&self, _serial: &str) -> Result<(), std::io::Error> { Ok(()) }
            fn screen_off_timeout(&self, _serial: &str) -> Result<Option<u64>, std::io::Error> { Ok(None) }
            fn set_screen_off_timeout(&self, _serial: &str, _millis: u64) -> Result<(), std::io::Error> { Ok(()) }
            fn stay_on_while_plugged_in(&self, _serial: &str) -> Result<Option<u64>, std::io::Error> { Ok(None) }
            fn set_stay_on_while_plugged_in(&self, _serial: &str, _bits: u64) -> Result<(), std::io::Error> { Ok(()) }
            fn window_policy(&self, _serial: &str) -> Result<String, std::io::Error> { Ok(String::new()) }
            fn power_state(&self, _serial: &str) -> Result<String, std::io::Error> { Ok(String::new()) }
            fn display_state(&self, _serial: &str) -> Result<String, std::io::Error> { Ok(String::new()) }
            fn press_key(&self, _serial: &str, _keycode: &str) -> Result<(), std::io::Error> { Ok(()) }
            fn screencap_probe_bytes(&self, _serial: &str) -> Result<u64, std::io::Error> { Ok(0) }
            fn screenshot_png(&self, _serial: &str) -> Result<Vec<u8>, std::io::Error> { Ok(Vec::new()) }
            fn make_directory(&self, _serial: &str, _remote_dir: &str) -> Result<(), std::io::Error> { Ok(()) }
            fn list_directory(&self, _serial: &str, _remote_dir: &str) -> Result<String, std::io::Error> { Ok(String::new()) }
            fn push_file(&self, _serial: &str, _local: &Path, _remote_dir: &str) -> Result<String, std::io::Error> { Ok(String::new()) }
            fn pull_file(&self, _serial: &str, _remote_path: &str, _local: &Path) -> Result<String, std::io::Error> { Ok(String::new()) }
            fn remove_device_file(&self, _serial: &str, _remote_path: &str) -> Result<(), std::io::Error> { Ok(()) }
            fn install_apk(&self, _serial: &str, _apk: &Path) -> Result<String, std::io::Error> { Ok(String::new()) }
            fn pair(&self, _endpoint: &str, _pairing_code: &str) -> Result<(), std::io::Error> { Ok(()) }
            fn connect(&self, _endpoint: &str) -> Result<(), std::io::Error> { Ok(()) }
            fn disconnect(&self, _endpoint: &str) -> Result<(), std::io::Error> { Ok(()) }
            fn open_developer_settings(&self, _serial: &str) -> Result<(), std::io::Error> { Ok(()) }
            fn open_keyboard_layout_settings(&self, _serial: &str) -> Result<(), std::io::Error> { Ok(()) }
            fn disable_wireless_debugging(&self, serial: &str) -> Result<(), std::io::Error> {
                if serial.contains(':') {
                    Err(std::io::Error::other("wireless transport dead"))
                } else {
                    Ok(())
                }
            }
            fn disable_usb_debugging(&self, serial: &str) -> Result<(), std::io::Error> {
                if serial.contains(':') {
                    Err(std::io::Error::other("wireless transport dead"))
                } else {
                    Ok(())
                }
            }
            fn mdns_services(&self) -> Result<String, std::io::Error> { Ok(String::new()) }
            fn list_device_apps(&self, _serial: &str) -> Result<String, std::io::Error> { Ok(String::new()) }
        }
        let runtimes = AppRuntimes {
            adb: Arc::new(WirelessOnlyFails),
            mirror: Box::new(FakeMirror::running()),
            serial_cache: Mutex::new(HashMap::new()),
        };
        let endpoints = vec![
            "79j7kn9tkjt8rwss".to_string(),
            "192.168.2.90:41901".to_string(),
        ];
        let receipt = revoke_device_access_with(&runtimes, &endpoints);
        let joined = receipt.steps.join("\n");
        assert!(joined.contains("已关闭手机上的「无线调试」"), "USB 兜底应成功：{joined}");
        assert!(joined.contains("已关闭手机上的「USB 调试」"), "{joined}");
    }

    #[test]
    fn revoke_receipt_for_unreachable_device_stays_user_friendly() {
        // 离线设备取消授权：各步都会失败，回执必须说人话，
        // 不得透出 adb 原始英文报错（X10-35 用户截图场景）。
        let adb = FakeAdb::with_device_write_failure();
        let endpoints = vec!["79j7kn9tkjt8rwss".to_string()];
        let receipt = revoke_device_access_with(&AppRuntimes {
            adb: Arc::new(adb),
            mirror: Box::new(FakeMirror::running()),
            serial_cache: Mutex::new(HashMap::new()),
        }, &endpoints);
        let joined = receipt.steps.join("\n");
        assert!(joined.contains("设备当前无响应"), "{joined}");
        assert!(!joined.contains("device offline"), "不得透出原始报错：{joined}");
        assert!(!joined.contains("adb"), "不得透出 adb 字样：{joined}");
    }

    #[test]
    fn merges_same_physical_device_across_usb_and_wireless() {
        let devices = vec![
            AdbDevice {
                serial: "ABC123".into(),
                label: "Pixel 8".into(),
                state: DeviceState::Ready,
                physical_serial: Some("PHYS-XYZ".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "ABC123".into(),
                    kind: ConnectionKind::Usb,
                    state: DeviceState::Ready,
                }],
            },
            AdbDevice {
                serial: "192.168.1.5:4321".into(),
                label: "Pixel 8".into(),
                state: DeviceState::Ready,
                physical_serial: Some("PHYS-XYZ".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "192.168.1.5:4321".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Ready,
                }],
            },
            AdbDevice {
                serial: "OTHER9".into(),
                label: "Pixel 7".into(),
                state: DeviceState::Ready,
                physical_serial: Some("PHYS-OTHER".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "OTHER9".into(),
                    kind: ConnectionKind::Usb,
                    state: DeviceState::Ready,
                }],
            },
        ];
        let merged = dedup_devices(devices);
        assert_eq!(merged.len(), 2, "同物理设备应合并为一条");
        let merged_device = merged
            .iter()
            .find(|device| device.physical_serial.as_deref() == Some("PHYS-XYZ"))
            .unwrap();
        // 首选通道优先 USB。
        assert_eq!(merged_device.serial, "ABC123");
        assert_eq!(merged_device.connections.len(), 2);
        assert_eq!(merged_device.state, DeviceState::Ready);
    }

    #[test]
    fn does_not_merge_devices_missing_physical_serial() {
        // 未授权设备读不到硬件序列号，型号相同也不得强行合并。
        let devices = vec![
            AdbDevice {
                serial: "AAA".into(),
                label: "Pixel 8".into(),
                state: DeviceState::Unauthorized,
                physical_serial: None,
                connections: vec![ConnectionEndpoint {
                    serial: "AAA".into(),
                    kind: ConnectionKind::Usb,
                    state: DeviceState::Unauthorized,
                }],
            },
            AdbDevice {
                serial: "BBB:1".into(),
                label: "Pixel 8".into(),
                state: DeviceState::Unauthorized,
                physical_serial: None,
                connections: vec![ConnectionEndpoint {
                    serial: "BBB:1".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Unauthorized,
                }],
            },
        ];
        assert_eq!(dedup_devices(devices).len(), 2);
    }

    #[test]
    fn endpoint_is_ready_matches_merged_connection_serial() {
        // 合并后 `serial` 是首选通道，但用任一原始端点查询仍应判为就绪。
        let devices = vec![AdbDevice {
            serial: "ABC123".into(),
            label: "Pixel 8".into(),
            state: DeviceState::Ready,
            physical_serial: Some("PHYS-XYZ".into()),
            connections: vec![
                ConnectionEndpoint { serial: "ABC123".into(), kind: ConnectionKind::Usb, state: DeviceState::Ready },
                ConnectionEndpoint { serial: "192.168.1.5:4321".into(), kind: ConnectionKind::Wireless, state: DeviceState::Ready },
            ],
        }];
        assert!(endpoint_is_ready(&devices, "ABC123"));
        assert!(endpoint_is_ready(&devices, "192.168.1.5:4321"));
        assert!(!endpoint_is_ready(&devices, "10.0.0.9:5555"));
    }

    #[test]
    fn offline_usb_ghost_of_same_phone_is_suppressed_into_ready_card() {
        // 用户场景（X10-32）：同一台手机既有就绪 Wi-Fi 通道，又残留一条 offline
        // USB 条目（拔线 ghost）。两者硬件序列号一致 → 影子端点不再单独成卡，
        // 用户看到的这台手机只有一张卡；且就绪卡不被污染（仍只有无线通道）。
        let devices = vec![
            AdbDevice {
                serial: "192.168.1.5:4321".into(),
                label: "Pixel 8".into(),
                state: DeviceState::Ready,
                physical_serial: Some("PHYS-XYZ".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "192.168.1.5:4321".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Ready,
                }],
            },
            AdbDevice {
                serial: "ABC123".into(),
                label: "Pixel 8".into(),
                state: DeviceState::Offline,
                physical_serial: Some("PHYS-XYZ".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "ABC123".into(),
                    kind: ConnectionKind::Usb,
                    state: DeviceState::Offline,
                }],
            },
        ];
        let merged = dedup_devices(devices);
        assert_eq!(merged.len(), 1, "同一台手机只出一张卡");
        let wifi = &merged[0];
        assert_eq!(wifi.serial, "192.168.1.5:4321");
        // 徽标口径不受影响：就绪卡的 connections 只含就绪通道（无线）。
        assert_eq!(wifi.connections.len(), 1);
        assert_eq!(wifi.connections[0].kind, ConnectionKind::Wireless);
    }

    #[test]
    fn mdns_ghost_of_another_phone_stays_visible() {
        // 无线调试重连后端口变了：旧 mDNS 条目与新 IP:端口 并存。mDNS 实例名
        // 内嵌 USB 序列号 → 判定为同一台手机，旧条目不再单独成卡。
        let devices = vec![
            AdbDevice {
                serial: "192.168.2.224:43735".into(),
                label: "M2104K10AC".into(),
                state: DeviceState::Ready,
                physical_serial: Some("qc8d8tonbmmzm7qs".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "192.168.2.224:43735".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Ready,
                }],
            },
            AdbDevice {
                serial: "adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp".into(),
                label: "M2104K10AC".into(),
                state: DeviceState::Offline,
                physical_serial: None,
                connections: vec![ConnectionEndpoint {
                    serial: "adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Offline,
                }],
            },
        ];
        // mDNS 名里是另一台手机的序列号（79j7…），与就绪组（qc8d8…）不同 → 保留。
        assert_eq!(dedup_devices(devices).len(), 2);
    }

    #[test]
    fn mdns_ghost_matching_group_usb_serial_is_merged() {
        let devices = vec![
            AdbDevice {
                serial: "79j7kn9tkjt8rwss".into(),
                label: "M2104K10AC".into(),
                state: DeviceState::Ready,
                physical_serial: Some("PHYS-1".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "79j7kn9tkjt8rwss".into(),
                    kind: ConnectionKind::Usb,
                    state: DeviceState::Ready,
                }],
            },
            AdbDevice {
                serial: "adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp".into(),
                label: "M2104K10AC".into(),
                state: DeviceState::Offline,
                physical_serial: None,
                connections: vec![ConnectionEndpoint {
                    serial: "adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Offline,
                }],
            },
        ];
        let merged = dedup_devices(devices);
        assert_eq!(merged.len(), 1, "mDNS 残影内嵌同一 USB 序列号 → 归并");
        assert_eq!(merged[0].state, DeviceState::Ready);
    }

    #[test]
    fn stale_port_ghost_with_same_ip_is_suppressed() {
        // 无线调试重启后换端口：旧 IP:端口 条目残留为 offline。同 IP 不同端口
        // → 判定为同一台手机，旧端点不再单独成卡。
        let devices = vec![
            AdbDevice {
                serial: "192.168.2.90:41901".into(),
                label: "M2104K10AC".into(),
                state: DeviceState::Ready,
                physical_serial: Some("PHYS-1".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "192.168.2.90:41901".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Ready,
                }],
            },
            AdbDevice {
                serial: "192.168.2.90:39999".into(),
                label: "M2104K10AC".into(),
                state: DeviceState::Offline,
                physical_serial: None,
                connections: vec![ConnectionEndpoint {
                    serial: "192.168.2.90:39999".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Offline,
                }],
            },
        ];
        let merged = dedup_devices(devices);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].serial, "192.168.2.90:41901");
    }

    #[test]
    fn offline_phone_collapses_usb_and_mdns_ghosts_into_one_card() {
        // 整台手机离线：USB 残影 + 它的 mDNS 残影 → 归并成一张离线卡。
        let devices = vec![AdbDevice {
            serial: "79j7kn9tkjt8rwss".into(),
            label: "M2104K10AC".into(),
            state: DeviceState::Offline,
            physical_serial: None,
            connections: vec![ConnectionEndpoint {
                serial: "79j7kn9tkjt8rwss".into(),
                kind: ConnectionKind::Usb,
                state: DeviceState::Offline,
            }],
        }, AdbDevice {
            serial: "adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp".into(),
            label: "M2104K10AC".into(),
            state: DeviceState::Offline,
            physical_serial: None,
            connections: vec![ConnectionEndpoint {
                serial: "adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp".into(),
                kind: ConnectionKind::Wireless,
                state: DeviceState::Offline,
            }],
        }];
        let merged = dedup_devices(devices);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].state, DeviceState::Offline);
    }

    #[test]
    fn unrelated_offline_device_stays_visible() {
        // 判定不了归属的离线设备必须保持可见——宁可多一张卡，不能静默吞掉。
        let devices = vec![
            AdbDevice {
                serial: "192.168.1.5:4321".into(),
                label: "Pixel 8".into(),
                state: DeviceState::Ready,
                physical_serial: Some("PHYS-XYZ".into()),
                connections: vec![ConnectionEndpoint {
                    serial: "192.168.1.5:4321".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Ready,
                }],
            },
            AdbDevice {
                serial: "10.0.0.9:5555".into(),
                label: "Pixel 7".into(),
                state: DeviceState::Offline,
                physical_serial: None,
                connections: vec![ConnectionEndpoint {
                    serial: "10.0.0.9:5555".into(),
                    kind: ConnectionKind::Wireless,
                    state: DeviceState::Offline,
                }],
            },
        ];
        assert_eq!(dedup_devices(devices).len(), 2);
    }

    #[test]
    fn accepts_ip_endpoints_but_not_shell_like_input() {
        assert_eq!(
            validate_endpoint("192.168.1.20:37123").unwrap(),
            "192.168.1.20:37123"
        );
        assert_eq!(
            validate_endpoint("192.168.1.20:37123; something")
                .unwrap_err()
                .code,
            "endpoint_invalid"
        );
    }

    #[test]
    fn pairing_code_must_be_six_digits() {
        assert_eq!(validate_pairing_code("012345").unwrap(), "012345");
        assert_eq!(
            validate_pairing_code("12345").unwrap_err().code,
            "pairing_code_invalid"
        );
        assert!(validate_pairing_code("123 45").is_err());
    }

    // -- scrcpy 运行时定位 --

    #[test]
    fn selects_an_explicit_runtime_before_the_development_runtime() {
        let explicit = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("missing-scrcpy");

        assert_eq!(
            select_scrcpy_binary(Some(explicit.clone()), &development, None),
            explicit
        );
    }

    #[test]
    fn a_bundled_runtime_is_preferred_over_path_in_release_mode() {
        // 模拟发行包布局：<root>/MirrorDock 旁是 <root>/resources/scrcpy/scrcpy。
        let layout = scratch_dir("bundled-runtime");
        let res = layout.join("resources").join("scrcpy");
        fs::create_dir_all(&res).unwrap();
        fs::write(res.join("scrcpy"), "#!/bin/sh\n").unwrap();

        let found = select_scrcpy_binary(None, Path::new("/nonexistent"), Some(&layout));
        assert_eq!(
            found,
            res.join("scrcpy"),
            "发行模式必须先找随包运行时，而不是指望用户 PATH 里有 scrcpy"
        );
    }

    #[test]
    fn missing_explicit_and_bundled_runtimes_fall_back_to_path() {
        let empty = scratch_dir("no-bundled-runtime");
        fs::create_dir_all(&empty).unwrap();
        let found = select_scrcpy_binary(None, Path::new("/nonexistent"), Some(&empty));
        assert_eq!(found, PathBuf::from("scrcpy"), "都找不到时才退回 PATH");
    }

    #[test]
    fn bundled_adb_prefers_the_platform_specific_location() {
        let layout = scratch_dir("bundled-adb");
        // macOS 布局：platform-tools 放在 Contents/Resources/resources 下。
        let res = layout
            .join("MirrorDock.app")
            .join("Contents")
            .join("Resources")
            .join("resources");
        let tools = res.join("platform-tools");
        fs::create_dir_all(&tools).unwrap();
        fs::write(tools.join("adb"), "#!/bin/sh\n").unwrap();

        let exe_dir = layout.join("MirrorDock.app").join("Contents").join("MacOS");
        let roots = bundled_runtime_dir_candidates(&exe_dir);
        let found = roots
            .iter()
            .map(|root| root.join("platform-tools/adb"))
            .find(|path| path.is_file());
        assert_eq!(
            found,
            Some(tools.join("adb")),
            "macOS 的 adb 应在应用资源目录的 platform-tools 下找到，实际候选：{roots:?}"
        );
    }

    // -- 启动参数白名单 --

    #[test]
    fn session_options_reject_invalid_input() {
        let invalid = SessionOptions {
            rotation: 45,
            ..Default::default()
        };
        assert_eq!(invalid.arguments().unwrap_err().code, "invalid_rotation");
        assert!(serde_json::from_str::<SessionOptions>(r#"{"quality":"custom"}"#).is_err());
        assert!(serde_json::from_str::<SessionOptions>(r#"{"shell":"anything"}"#).is_err());
    }

    #[test]
    fn session_presets_produce_bounded_arguments() {
        for (quality, size, bitrate) in [
            (Quality::Smooth, 1024, "2M"),
            (Quality::Balanced, 1920, "8M"),
            (Quality::Sharp, 2560, "16M"),
        ] {
            let options = SessionOptions {
                quality,
                rotation: 90,
                fullscreen: true,
                always_on_top: true,
                keep_awake: true,
                record: false,
                clipboard_autosync: true,
                audio: true,
                shortcut_mod: None,
                show_touches: false,
                keyboard_uhid: true,
                read_only: false,
                max_fps: None,
                desktop_mode: false,
                desktop_app: None,
                camera_source: false,
            };
            let args = options.arguments().unwrap();
            assert!(args.contains(&format!("--max-size={size}")));
            assert!(args.contains(&format!("--video-bit-rate={bitrate}")));
            assert!(args.contains(&"--video-codec=h264".into()));
            assert!(args.contains(&"--stay-awake".into()));
            assert!(args.contains(&"--keyboard=uhid".into()));
            assert!(args.contains(&"--fullscreen".into()));
            assert!(args.contains(&"--always-on-top".into()));
            assert!(args.contains(&"--display-orientation=90".into()));
        }
    }

    /// 自动横竖屏（B-旋转）：rotation=0 表示「跟随手机」，必须**不传**
    /// `--display-orientation`——传了 0 反而会把画面锁死在竖屏，横屏游戏
    /// 打开时镜像不会跟着转（真机问题定案）。
    #[test]
    fn rotation_zero_omits_display_orientation_so_device_rotation_is_followed() {
        let options = SessionOptions::default();
        assert_eq!(options.rotation, 0);
        let args = options.arguments().unwrap();
        assert!(!args.iter().any(|arg| arg.starts_with("--display-orientation")));
        // 显式选择其它角度时仍然锁定。
        let rotated = SessionOptions { rotation: 270, ..Default::default() };
        assert!(rotated.arguments().unwrap().contains(&"--display-orientation=270".into()));
    }

    /// mDNS 解析（B-开发者中心配对）：只认配对与连接两类服务，端点必须是 ip:port。
    /// 真实 adb 输出是「实例名 + 服务类型 + 端点」三列，这里同时覆盖三列与两列旧格式。
    #[test]
    fn mdns_parsing_keeps_only_pairing_and_connect_endpoints() {
        let raw = "List of discovered mdns services\nadb-79j7kn9tkjt8rwss-rF7qH8\t_adb-tls-connect._tcp\t192.168.1.9:33739\nstudio-58m7E2\t_adb-tls-pairing._tcp\t192.168.1.20:37123\nadb-other\t_adb-tls-connect._tcp\tnot-an-endpoint\nstudio-58m7E2\t_adb-tls-pairing._tcp\t192.168.1.20:37123\n";
        let (pairing, connect) = parse_mdns_services(raw);
        assert_eq!(pairing, vec!["192.168.1.20:37123".to_owned()]);
        assert_eq!(connect, vec!["192.168.1.9:33739".to_owned()]);
        assert!(parse_mdns_services("").0.is_empty());
        assert!(parse_mdns_services("").1.is_empty());
    }

    /// 扫码配对：按「服务类型 + 实例名」精确命中手机广播的配对服务；实例名不同的
    /// 其他配对服务（别的电脑正在配对）绝不能误配。
    #[test]
    fn mdns_entries_match_pairing_instance_exactly() {
        let raw = "adb-xxx-A1\t_adb-tls-pairing._tcp\t192.168.1.20:37001\nmirrordock-abc234\t_adb-tls-pairing._tcp\t192.168.1.20:37002\nmirrordock-abc234\t_adb-tls-connect._tcp\t192.168.1.20:41002\n";
        let entries = parse_mdns_entries(raw);
        let found = entries
            .iter()
            .find(|entry| entry.service == "_adb-tls-pairing._tcp" && entry.instance == "mirrordock-abc234")
            .map(|entry| entry.endpoint.clone());
        assert_eq!(found, Some("192.168.1.20:37002".to_owned()));
    }

    /// 扫码配对：二维码载荷必须是 Android 识别的 WIFI:T:ADB 格式；生成的配对码
    /// 必须通过既有校验（6 位数字）。
    #[test]
    fn qr_pairing_generates_valid_payload_and_code() {
        let store = QrPairingStore::default();
        let offer = {
            // begin_qr_pairing 只依赖 store；直接内联等价逻辑以避免构造 Tauri State。
            let service_name = "mirrordock-test1".to_owned();
            let pairing_code = random_pairing_code();
            store.insert(&service_name);
            QrPairingOffer {
                payload: format!("WIFI:T:ADB;S:{service_name};P:{pairing_code};;"),
                service_name,
                pairing_code,
            }
        };
        assert!(offer.payload.starts_with("WIFI:T:ADB;S:mirrordock-"));
        assert!(offer.payload.ends_with(";;"));
        assert!(validate_pairing_code(&offer.pairing_code).is_ok());
        assert_eq!(random_pairing_code().len(), 6);
        assert!(random_service_name().starts_with("mirrordock-"));
    }

    /// X10-91：配对成功后只按「配对阶段 adb pair 命中的手机 IP」连接对应的
    /// `_adb-tls-connect` 条目；局域网里同时广播无线调试的其他手机（不同 IP）
    /// 绝不能被误连。这复现用户场景——家里多台手机，配对成功却连不上/连错。
    #[test]
    fn qr_pairing_connects_only_the_phone_with_the_paired_ip() {
        let store = QrPairingStore::default();
        let service = "mirrordock-x10r91";
        store.insert(service);
        // 配对阶段 adb pair 命中 192.168.1.20（用户手机）。
        store.mark_paired(service, Some("192.168.1.20".to_owned()));
        assert!(store.is_paired(service));
        assert_eq!(store.paired_ip(service).as_deref(), Some("192.168.1.20"));

        // 手机回到主页面后改广播 connect；同网还有另一台手机（192.168.1.30）也在广播。
        let raw = "adb-phoneA-x1\t_adb-tls-connect._tcp\t192.168.1.30:41001\nadb-phoneB-y2\t_adb-tls-connect._tcp\t192.168.1.20:41002\n";
        let entries = parse_mdns_entries(raw);
        let paired_ip = store.paired_ip(service);
        let chosen = entries.iter().find(|entry| {
            entry.service == "_adb-tls-connect._tcp"
                && paired_ip
                    .as_deref()
                    .is_some_and(|ip| entry.endpoint.rsplit_once(':').map(|(host, _)| host) == Some(ip))
        });
        // 必须选中同 IP 的那台（用户手机），而不是第一台/别的手机。
        assert_eq!(chosen.map(|e| e.endpoint.as_str()), Some("192.168.1.20:41002"));

        // 若配对 IP 不在广播里（手机还没回主页面），不得连接任何设备。
        store.mark_paired(service, Some("192.168.1.99".to_owned()));
        let paired_ip = store.paired_ip(service);
        let chosen = entries.iter().find(|entry| {
            entry.service == "_adb-tls-connect._tcp"
                && paired_ip
                    .as_deref()
                    .is_some_and(|ip| entry.endpoint.rsplit_once(':').map(|(host, _)| host) == Some(ip))
        });
        assert!(chosen.is_none());
    }

    // -- 结构化错误契约 --

    #[test]
    fn structured_errors_always_carry_code_message_and_recovery() {
        let value = serde_json::to_value(AppError::new(
            "device_offline",
            "手机当前处于离线状态。",
            "请重新插拔数据线后重试。",
        ))
        .unwrap();

        assert_eq!(value["code"], "device_offline");
        assert!(!value["message"].as_str().unwrap().is_empty());
        assert!(!value["recovery"].as_str().unwrap().is_empty());
        assert_eq!(value.as_object().unwrap().len(), 3);
    }

    // -- 会话状态机（使用假运行时，无需真机） --

    #[test]
    fn active_session_cannot_be_replaced_even_for_the_same_device() {
        let store = SessionStore::default();
        reserve_session(&store, MirrorSession::connecting("phone".into())).unwrap();
        assert_eq!(
            reserve_session(&store, MirrorSession::connecting("phone".into()))
                .unwrap_err()
                .code,
            "session_busy"
        );
        mark_session(&store, MirrorSession::streaming("phone".into()));
        assert!(reserve_session(&store, MirrorSession::connecting("phone".into())).is_err());
        // 结束该设备会话（回 Idle 后按设备移除条目），同设备即可再次启动。
        mark_session(&store, MirrorSession::streaming("phone".into()));
        {
            let mut map = store.lock().unwrap();
            let state = session_entry_mut(&mut map, "phone");
            state.epoch = state.epoch.wrapping_add(1);
            state.process = None;
            state.session = MirrorSession::idle();
        }
        prune_idle_entry(&store, "phone");
        assert!(reserve_session(&store, MirrorSession::connecting("phone".into())).is_ok());
    }

    #[test]
    fn two_devices_hold_concurrent_sessions_and_do_not_block_each_other() {
        // X10-27 核心语义：A 在镜像中时，B 的启动不再被 session_busy 挡住；
        // 互斥只收在「同一台设备」上。
        let store = SessionStore::default();
        reserve_session(&store, MirrorSession::connecting("phone-a".into())).unwrap();
        assert!(
            reserve_session(&store, MirrorSession::connecting("phone-b".into())).is_ok(),
            "另一台设备的会话不得被 A 的会话阻塞"
        );
        // 同设备仍然互斥。
        assert_eq!(
            reserve_session(&store, MirrorSession::connecting("phone-a".into()))
                .unwrap_err()
                .code,
            "session_busy"
        );
        // 主会话选取：BTreeMap 按序列号稳定排序，取第一台。
        let map = store.lock().unwrap();
        assert_eq!(primary_session_serial(&map).as_deref(), Some("phone-a"));
        assert_eq!(map.len(), 2, "两台设备各有一条会话记录");
    }

    #[test]
    fn unauthorized_device_reports_its_own_state_not_a_generic_failure() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Unauthorized)]),
            FakeMirror::running(),
        );
        let store = SessionStore::default();

        let error =
            start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None)
                .unwrap_err();

        assert_eq!(error.code, "device_unauthorized");
        let session = snapshot(&store);
        assert_eq!(session.phase, SessionPhase::Unauthorized);
        assert_eq!(session.serial.as_deref(), Some("phone"));
        assert_eq!(
            session.error.as_ref().map(|error| error.code),
            Some("device_unauthorized")
        );
    }

    #[test]
    fn offline_device_reports_its_own_state_not_a_generic_failure() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Offline)]),
            FakeMirror::running(),
        );
        let store = SessionStore::default();

        let error =
            start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None)
                .unwrap_err();

        assert_eq!(error.code, "device_offline");
        assert_eq!(snapshot(&store).phase, SessionPhase::Offline);
    }

    #[test]
    fn missing_adb_is_distinct_from_a_missing_device() {
        let store = SessionStore::default();

        let unavailable = runtimes(FakeAdb::unavailable(), FakeMirror::running());
        let error = start_mirroring_with(
            &unavailable,
            &store,
            "phone".into(),
            SessionOptions::default(),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code, "adb_unavailable");

        let empty = runtimes(FakeAdb::with_devices(Vec::new()), FakeMirror::running());
        let error =
            start_mirroring_with(&empty, &store, "phone".into(), SessionOptions::default(), None)
                .unwrap_err();
        assert_eq!(error.code, "device_not_connected");
    }

    #[test]
    fn missing_mirror_runtime_fails_before_touching_the_device() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::unavailable(),
        );
        let store = SessionStore::default();

        let error =
            start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None)
                .unwrap_err();

        assert_eq!(error.code, "mirror_runtime_missing");
        assert_eq!(snapshot(&store).phase, SessionPhase::Failed);
    }

    #[test]
    fn starting_a_session_reports_streaming_without_claiming_first_frame() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::running(),
        );
        let store = SessionStore::default();

        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None).unwrap();

        let session = snapshot(&store);
        assert_eq!(session.phase, SessionPhase::Streaming);
        assert_eq!(
            session.first_frame,
            FirstFrame::Unknown,
            "进程运行不等于首帧已到达，未接入探针前不得上报 Reached"
        );
        assert_eq!(session.serial.as_deref(), Some("phone"));
    }

    #[test]
    fn stopping_returns_to_idle_and_kills_the_process() {
        let mirror = FakeMirror::running();
        let killed = Arc::clone(&mirror.killed);
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            mirror,
        );
        let store = SessionStore::default();

        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None).unwrap();
        stop_mirroring_with(&store, Some("phone".into())).unwrap();

        assert!(*killed.lock().unwrap());
        assert_eq!(snapshot(&store).phase, SessionPhase::Idle);
        assert_eq!(
            stop_mirroring_with(&store, Some("phone".into())).unwrap_err().code,
            "session_not_running"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_graceful_stop_lets_a_recording_finalize_instead_of_killing() {
        // 用一个真实的子进程验证系统实现的 stop：SIGTERM 后进程退出，而不是被强杀。
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut process = SystemMirrorProcess {
            child,
            output: std::sync::Arc::new(ScrcpyOutputSink::default()),
            is_recorder: false,
        };

        let started = std::time::Instant::now();
        process.stop().unwrap();

        assert!(
            started.elapsed() < PROCESS_GRACEFUL_TIMEOUT,
            "SIGTERM 路径应当迅速退出，而不是等满超时"
        );
        // 被信号终止的进程 status.success() 为 false，这里只断言「已退出且被回收」。
        assert!(process.try_wait().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_process_that_ignores_sigterm_is_still_killed_before_the_timeout_budget_runs_out() {
        // 兜底路径：忽略 SIGTERM 的进程在宽限期后被强杀，stop 最终返回成功。
        // （sh 对 trap 的行为因实现而异，python 的 SIG_IGN 语义可靠。）
        use std::io::BufRead;
        let mut child = Command::new("python3")
            .arg("-c")
            .arg("import signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print('ready', flush=True); time.sleep(30)")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // 等处理器的注册完成：SIGTERM 若在注册前送达，python 会被默认处置杀死，
        // 测的就不再是兜底路径。
        let mut ready = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready).unwrap();
        assert!(ready.contains("ready"), "子进程未就绪：{ready}");
        let mut process = SystemMirrorProcess {
            child,
            output: std::sync::Arc::new(ScrcpyOutputSink::default()),
            is_recorder: false,
        };

        let started = std::time::Instant::now();
        process.stop().unwrap();

        assert!(
            started.elapsed() >= PROCESS_GRACEFUL_TIMEOUT,
            "忽略 SIGTERM 的进程应当等满宽限期再被强杀"
        );
        assert!(
            started.elapsed() < PROCESS_GRACEFUL_TIMEOUT + PROCESS_REAP_TIMEOUT,
            "强杀之后应当很快回收，不应长时间挂起"
        );
        assert!(process.try_wait().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_recorder_is_stopped_with_sigint_so_the_recording_finalizes() {
        // X10-95：录制进程必须收到 SIGINT（而非 SIGTERM），且要在宽限期内退出——
        // 这是 MP4 写出 moov 索引、不变砖的前提。用一个「忽略 SIGTERM、
        // 只在收到 SIGINT 时退出」的探针进程验证 stop 发的是 SIGINT。
        use std::io::BufRead;
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(concat!(
                "import signal, time, sys;",
                "signal.signal(signal.SIGTERM, signal.SIG_IGN);",  // 若发 SIGTERM 会卡住
                "signal.signal(signal.SIGINT, lambda s,f: sys.exit(0));",  // SIGINT 才退出
                "print('ready', flush=True);",
                "time.sleep(30)"
            ))
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready).unwrap();
        assert!(ready.contains("ready"), "探针未就绪：{ready}");
        let mut process = SystemMirrorProcess {
            child,
            output: std::sync::Arc::new(ScrcpyOutputSink::default()),
            is_recorder: true, // 录制进程语义
        };

        let started = std::time::Instant::now();
        process.stop().unwrap();

        // SIGINT 被探针捕获后立即退出——远小于宽限期；若误发 SIGTERM 会卡满超时。
        assert!(
            started.elapsed() < PROCESS_GRACEFUL_TIMEOUT,
            "录制进程应当被 SIGINT 迅速停止（走了 SIGTERM 才会卡满超时）"
        );
        assert!(process.try_wait().is_some());
    }

    // -- 会话中应用设置（镜像窗口形态由启动参数决定，只能靠「结束 + 重开」生效） --

    /// 「已经启动的会话」在测试中的句柄：运行时、会话存储，以及可断言的镜像替身状态。
    struct StartedSession {
        runtimes: AppRuntimes,
        store: SessionStore,
        killed: Arc<Mutex<bool>>,
        starts: Arc<Mutex<Vec<SessionOptions>>>,
    }

    /// 启动一个使用默认参数的会话。
    fn started_session() -> StartedSession {
        let mirror = FakeMirror::running();
        let killed = Arc::clone(&mirror.killed);
        let starts = Arc::clone(&mirror.starts);
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            mirror,
        );
        let store = SessionStore::default();
        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None).unwrap();
        StartedSession {
            runtimes,
            store,
            killed,
            starts,
        }
    }

    #[test]
    fn applying_new_options_restarts_the_mirror_window_with_the_new_arguments() {
        let session = started_session();

        let update = apply_session_options_with(
            &session.runtimes,
            &session.store,
            SessionOptions {
                rotation: 90,
                fullscreen: true,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();

        assert!(update.applied, "有新设置时应当真的重启镜像窗口");
        assert!(update.note.is_none());
        let recorded = session.starts.lock().unwrap().clone();
        assert_eq!(recorded.len(), 2, "应当恰好启动两次：原会话 + 按新设置重启");
        assert_eq!(recorded[1].rotation, 90);
        assert!(recorded[1].fullscreen);
        assert!(
            *session.killed.lock().unwrap(),
            "旧镜像窗口必须被结束，不能留下两个窗口"
        );
        let live = snapshot(&session.store);
        assert_eq!(live.phase, SessionPhase::Streaming);
        assert_eq!(live.serial.as_deref(), Some("phone"));
        assert_eq!(
            update.session, live,
            "返回的会话状态必须与轮询到的状态一致"
        );
    }

    #[test]
    fn unchanged_options_never_interrupt_a_healthy_session() {
        let session = started_session();

        let update = apply_session_options_with(
            &session.runtimes,
            &session.store,
            SessionOptions::default(),
            None,
            None,
        )
        .unwrap();

        assert!(!update.applied);
        assert!(
            update.note.is_some(),
            "未重启时必须说明原因，不能让用户以为改动被忽略了"
        );
        assert_eq!(session.starts.lock().unwrap().len(), 1);
        assert!(
            !*session.killed.lock().unwrap(),
            "参数没变就不该关闭正在正常运行的窗口"
        );
        assert_eq!(snapshot(&session.store).phase, SessionPhase::Streaming);
    }

    #[test]
    fn invalid_options_are_rejected_without_disturbing_the_running_session() {
        let session = started_session();

        let error = apply_session_options_with(
            &session.runtimes,
            &session.store,
            SessionOptions {
                rotation: 45,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap_err();

        assert_eq!(error.code, "invalid_rotation");
        assert_eq!(session.starts.lock().unwrap().len(), 1);
        assert!(!*session.killed.lock().unwrap());
        assert_eq!(
            snapshot(&session.store).phase,
            SessionPhase::Streaming,
            "一个非法请求绝不能打断一次正常的镜像"
        );
    }

    #[test]
    fn applying_options_without_a_running_session_is_refused_and_explains_when_it_applies() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::running(),
        );
        let store = SessionStore::default();

        let error = apply_session_options_with(&runtimes, &store, SessionOptions::default(), None, None)
            .unwrap_err();
        assert_eq!(error.code, "session_not_running");
        assert!(
            error.recovery.contains("下次开始镜像"),
            "必须说明设置何时生效，而不是只说没有会话：{}",
            error.recovery
        );

        // 「已配对但还没开始镜像」同样不是进行中的会话。
        mark_session(&store, MirrorSession::paired("192.168.1.20:37123".into()));
        assert_eq!(
            apply_session_options_with(&runtimes, &store, SessionOptions::default(), None, None)
                .unwrap_err()
                .code,
            "session_not_running"
        );
    }

    #[test]
    fn a_failed_restart_says_the_settings_did_not_apply() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::failing_start(),
        );
        let store = SessionStore::default();
        // 用一个必然失败的镜像替身无法走完首次启动，因此这里手工占位一个运行中的会话。
        reserve_session(&store, MirrorSession::connecting("phone".into())).unwrap();
        mark_running_for_test(&store, SessionOptions::default());

        let error = apply_session_options_with(
            &runtimes,
            &store,
            SessionOptions {
                fullscreen: true,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap_err();

        assert_eq!(error.code, "mirror_start_failed", "必须保留具体原因");
        assert!(
            error.recovery.contains("本次修改未生效"),
            "用户必须知道新设置没有生效：{}",
            error.recovery
        );
        let session = snapshot(&store);
        assert_eq!(session.phase, SessionPhase::Failed);
        assert!(
            session
                .error
                .as_ref()
                .is_some_and(|stored| stored.recovery.contains("本次修改未生效")),
            "轮询到的会话错误也要带上同样的上下文"
        );
    }

    #[test]
    fn a_restart_that_loses_the_device_reports_that_device_state() {
        let (adb, live) = FakeAdb::with_live_devices(vec![device("phone", DeviceState::Ready)]);
        let mirror = FakeMirror::running();
        let runtimes = runtimes(adb, mirror);
        let store = SessionStore::default();
        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None).unwrap();

        // 重启过程中手机掉线：必须落到设备状态，而不是笼统的“重启失败”。
        *live.lock().unwrap() = vec![device("phone", DeviceState::Offline)];
        let error = apply_session_options_with(
            &runtimes,
            &store,
            SessionOptions {
                quality: Quality::Smooth,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap_err();

        assert_eq!(error.code, "device_offline");
        assert!(error.recovery.contains("本次修改未生效"));
        assert_eq!(snapshot(&store).phase, SessionPhase::Offline);

        // 上一次重启已经让会话停机，此时再点「应用」应当明确回答没有进行中的会话，
        // 而不是去 kill 一个不存在的进程。
        *live.lock().unwrap() = Vec::new();
        let error = apply_session_options_with(
            &runtimes,
            &store,
            SessionOptions {
                quality: Quality::Sharp,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(error.code, "session_not_running");
    }

    #[test]
    fn a_restart_keeps_the_session_in_connecting_instead_of_dropping_to_idle() {
        let session = started_session();

        let handed_over = begin_session_restart(&session.store, "phone".into())
            .unwrap()
            .is_some();
        assert!(handed_over, "运行中的会话应当交出进程用于重启");

        let mid_restart = snapshot(&session.store);
        assert_eq!(
            mid_restart.phase,
            SessionPhase::Connecting,
            "重启途中必须停在“正在启动”，出现瞬间 Idle 会让用户以为镜像已经结束"
        );
        assert_eq!(mid_restart.serial.as_deref(), Some("phone"));

        // 没有进程时不得改写会话状态。
        let idle = SessionStore::default();
        assert!(begin_session_restart(&idle, "phone".into()).unwrap().is_none());
        assert_eq!(snapshot(&idle).phase, SessionPhase::Idle);
    }

    #[test]
    fn stopping_after_a_restart_still_returns_to_idle() {
        let session = started_session();

        apply_session_options_with(
            &session.runtimes,
            &session.store,
            SessionOptions {
                always_on_top: true,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();
        assert_eq!(session.starts.lock().unwrap().len(), 2);

        stop_mirroring_with(&session.store, Some("phone".into())).unwrap();
        assert!(*session.killed.lock().unwrap());
        assert_eq!(snapshot(&session.store).phase, SessionPhase::Idle);
    }

    #[test]
    fn monitor_returns_to_idle_when_the_process_exits_cleanly() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::exiting(true),
        );
        let store = SessionStore::default();

        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None).unwrap();

        assert_eq!(wait_until_idle_or_failed(&store).phase, SessionPhase::Idle);
    }

    #[test]
    fn monitor_reports_a_recoverable_failure_when_the_process_exits_abnormally() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::exiting(false),
        );
        let store = SessionStore::default();

        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None).unwrap();

        let session = wait_until_idle_or_failed(&store);
        assert_eq!(session.phase, SessionPhase::Failed);
        let error = session.error.expect("failed session must carry an error");
        assert_eq!(error.code, "mirror_exited");
        assert!(!error.recovery.is_empty());
        // 失败后可重试
        assert!(reserve_session(&store, MirrorSession::connecting("phone".into())).is_ok());
    }

    // -- 无线会话的亮屏补偿（使用假运行时，无需真机） --

    #[test]
    fn wireless_serials_are_detected_but_usb_serials_are_not() {
        // USB 序列号：纯设备号，不含冒号与 `.tcp`，也不以 `adb-` 开头。
        assert!(!is_wireless_serial("79j7kn9tkjt8rwss"));
        assert!(!is_wireless_serial("emulator-5554"));
        // 无线：ip:port 与 mDNS 派生序列号。
        assert!(is_wireless_serial("192.168.1.9:33739"));
        assert!(is_wireless_serial("adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp"));
        // 边界：空串与畸形输入按 USB 处理（不做补偿）。
        assert!(!is_wireless_serial(""));
    }

    #[test]
    fn screen_timeout_output_is_parsed_strictly() {
        assert_eq!(parse_settings_number("30000\n"), Some(30000));
        assert_eq!(parse_settings_number("  60000  "), Some(60000));
        assert_eq!(parse_settings_number("0"), Some(0));
        // `null`（键不存在）与垃圾输出一律视为读不到。
        assert_eq!(parse_settings_number("null\n"), None);
        assert_eq!(parse_settings_number(""), None);
        assert_eq!(parse_settings_number("-1\n"), None);
        assert_eq!(parse_settings_number("not-a-number\n"), None);
    }

    #[test]
    fn wireless_keep_awake_extends_timeout_and_backs_up_the_original() {
        let adb = FakeAdb {
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            ..FakeAdb::default()
        };
        let store = SessionStore::default();

        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");

        // 设备侧被写成延长值；内存备份保存的是原值 30000。
        assert_eq!(*adb.screen_timeout.lock().unwrap(), Some(WIRELESS_KEEP_AWAKE_TIMEOUT_MS));
        let backup = keep_awake_backup_of(&store, "192.168.1.9:33739").expect("backup must exist");
        assert_eq!(backup.serial, "192.168.1.9:33739");
        assert_eq!(backup.millis, 30000);
    }

    #[test]
    fn wireless_keep_awake_arms_timeout_and_stay_on_levers() {
        // 会话期间延长熄屏时间并打开「充电时保持唤醒」两把杠杆；无线亮屏由 scrcpy
        // `--stay-awake` 负责，不再伪造充电（见 BUG-充电掩盖）。
        let adb = FakeAdb {
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            stay_on_bits: Arc::new(Mutex::new(Some(0))),
            ..FakeAdb::default()
        };
        let store = SessionStore::default();

        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");

        assert_eq!(*adb.screen_timeout.lock().unwrap(), Some(WIRELESS_KEEP_AWAKE_TIMEOUT_MS));
        assert_eq!(*adb.stay_on_bits.lock().unwrap(), Some(STAY_ON_WHILE_PLUGGED_IN_ALL));
        let backup = keep_awake_backup_of(&store, "192.168.1.9:33739").expect("backup must exist");
        assert_eq!(backup.millis, 30000);
        assert_eq!(backup.stay_on_while_plugged_in, Some(0));
        // 不得伪造充电：设备侧不得出现 set_charging 调用。
        assert!(
            !adb.calls.lock().unwrap().iter().any(|call| call.starts_with("set_charging")),
            "{:?}",
            adb.calls.lock().unwrap()
        );
    }

    #[test]
    fn wireless_keep_awake_skips_levers_whose_original_value_is_unreadable() {
        // 读不到原值就不改它：宁可不生效，也绝不留下无法还原的改动。
        let adb = FakeAdb {
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            ..FakeAdb::default()
        };
        let store = SessionStore::default();

        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");

        assert_eq!(*adb.screen_timeout.lock().unwrap(), Some(WIRELESS_KEEP_AWAKE_TIMEOUT_MS));
        assert_eq!(*adb.stay_on_bits.lock().unwrap(), None);
        let calls = adb.calls.lock().unwrap().clone();
        assert!(!calls.iter().any(|call| call.starts_with("set_stay_on")), "{calls:?}");
        assert!(!calls.iter().any(|call| call.starts_with("set_charging")), "{calls:?}");
        let backup = keep_awake_backup_of(&store, "192.168.1.9:33739").unwrap();
        assert_eq!(backup.stay_on_while_plugged_in, None);
    }

    #[test]
    fn disabling_restores_levers_with_stay_on_before_timeout() {
        let adb = FakeAdb {
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            stay_on_bits: Arc::new(Mutex::new(Some(0))),
            ..FakeAdb::default()
        };
        let store = SessionStore::default();
        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");
        adb.calls.lock().unwrap().clear();

        assert!(disable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739"));

        assert_eq!(*adb.screen_timeout.lock().unwrap(), Some(30000));
        assert_eq!(*adb.stay_on_bits.lock().unwrap(), Some(0));
        assert!(keep_awake_backup_of(&store, "192.168.1.9:33739").is_none());
        // 还原顺序：先 stay_on，再 timeout；且绝不伪造充电（不得出现 set_charging）。
        let calls = adb.calls.lock().unwrap().clone();
        assert!(
            !calls.iter().any(|call| call.starts_with("set_charging")),
            "不得伪造充电：{calls:?}"
        );
        let first_restore = calls
            .iter()
            .find(|call| call.starts_with("set_stay_on") || call.starts_with("set_timeout"));
        assert!(
            matches!(first_restore, Some(c) if c.starts_with("set_stay_on")),
            "撤销顺序：{calls:?}"
        );
    }

    #[test]
    fn session_restart_re_arms_every_lever_without_touching_the_backup() {
        let adb = FakeAdb {
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            stay_on_bits: Arc::new(Mutex::new(Some(0))),
            ..FakeAdb::default()
        };
        let store = SessionStore::default();
        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");
        // 模拟设备在会话重启期间被外部复位。
        *adb.stay_on_bits.lock().unwrap() = Some(0);
        let before = adb.calls.lock().unwrap().len();

        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");

        assert_eq!(*adb.stay_on_bits.lock().unwrap(), Some(STAY_ON_WHILE_PLUGGED_IN_ALL));
        let backup = keep_awake_backup_of(&store, "192.168.1.9:33739").unwrap();
        assert_eq!(backup.millis, 30000, "重启不得覆盖原值账本");
        assert_eq!(backup.stay_on_while_plugged_in, Some(0), "重启不得覆盖原值账本");
        let calls = adb.calls.lock().unwrap().clone();
        let restarted = &calls[before..];
        assert!(
            restarted.iter().all(|call| !call.starts_with("get_")),
            "重启分支只写不读：{restarted:?}"
        );
    }

    #[test]
    fn legacy_backup_file_without_the_new_levers_still_loads() {
        // 兼容老版本崩溃残留的账本（只记了 serial + millis）：新字段取默认值，
        // 因而只还原熄屏时间，绝不凭空写回一个猜测出来的「充电时保持唤醒」。
        let path = std::env::temp_dir().join("mirrordock-legacy-keep-awake.json");
        fs::write(&path, br#"{"serial":"192.168.1.9:33739","millis":15000}"#).unwrap();

        let backup = load_keep_awake_ledger(&path);
        assert_eq!(backup.len(), 1, "legacy file must parse");
        assert_eq!(backup[0].millis, 15000);
        assert_eq!(backup[0].stay_on_while_plugged_in, None);

        let adb = FakeAdb::default();
        restore_persisted_keep_awake(&adb, &path);

        assert_eq!(*adb.screen_timeout.lock().unwrap(), Some(15000));
        let calls = adb.calls.lock().unwrap().clone();
        assert!(!calls.iter().any(|call| call.starts_with("set_stay_on")), "{calls:?}");
        assert!(!calls.iter().any(|call| call.starts_with("set_charging")), "{calls:?}");
        assert!(!path.exists(), "还原成功后账本文件必须删除");
    }

    #[test]
    fn usb_sessions_never_touch_screen_timeout() {
        let adb = FakeAdb {
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            ..FakeAdb::default()
        };
        let store = SessionStore::default();

        enable_wireless_keep_awake(&adb, &store, "79j7kn9tkjt8rwss");

        // USB 序列号不做补偿：不读也不写，设备设置保持原样；会话表也不留账本。
        assert_eq!(*adb.screen_timeout.lock().unwrap(), Some(30000));
        assert!(keep_awake_backup_of(&store, "79j7kn9tkjt8rwss").is_none());
        assert!(keep_awake_backup_of(&store, "192.168.1.9:33739").is_none());
        assert!(adb.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn session_restart_rewrites_extension_without_replacing_the_backup() {
        let adb = FakeAdb {
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            ..FakeAdb::default()
        };
        let store = SessionStore::default();
        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");

        // 模拟重启：当前设备值已是延长值。再次补偿必须**重复写延长值**，
        // 而不能把延长值当新原值备份——那会让真正的原值 30000 永久丢失。
        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");

        let backup = keep_awake_backup_of(&store, "192.168.1.9:33739").unwrap();
        assert_eq!(backup.millis, 30000, "重启不得覆盖原值备份");
        assert_eq!(*adb.screen_timeout.lock().unwrap(), Some(WIRELESS_KEEP_AWAKE_TIMEOUT_MS));
        // 第二次启用只写不读。
        let calls = adb.calls.lock().unwrap().clone();
        assert_eq!(
            calls.iter().filter(|call| call.starts_with("get_timeout")).count(),
            1,
            "只有第一次启用需要读原值：{calls:?}"
        );
    }

    #[test]
    fn keep_awake_is_skipped_when_the_original_timeout_is_unreadable() {
        // screen_timeout = None：模拟 `settings get` 返回 null 或设备无响应。
        let adb = FakeAdb::default();
        let store = SessionStore::default();

        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");

        assert!(keep_awake_backup_of(&store, "192.168.1.9:33739").is_none());
        // 读不到原值就不写：绝不瞎猜一个"默认值"当原值。
        assert!(adb.calls.lock().unwrap().iter().all(|call| !call.starts_with("set_timeout")));
    }

    #[test]
    fn disabling_keep_awake_restores_the_original_value() {
        let adb = FakeAdb {
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            ..FakeAdb::default()
        };
        let store = SessionStore::default();
        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");

        assert!(disable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739"));

        assert_eq!(*adb.screen_timeout.lock().unwrap(), Some(30000));
        assert!(keep_awake_backup_of(&store, "192.168.1.9:33739").is_none());
    }

    #[test]
    fn failed_restore_keeps_the_backup_for_a_later_retry() {
        let adb = FakeAdb {
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            ..FakeAdb::default()
        };
        let store = SessionStore::default();
        enable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739");

        // 补偿成功后设备离线：写入原值必然失败。
        *adb.device_writes_fail.lock().unwrap() = true;
        assert!(!disable_wireless_keep_awake(&adb, &store, "192.168.1.9:33739"));
        // 设备离线导致还原失败：备份必须保留，等下次会话或下次启动重试。
        assert!(keep_awake_backup_of(&store, "192.168.1.9:33739").is_some());
    }

    #[test]
    fn full_wireless_session_compensates_and_restores_screen_timeout() {
        // 端到端：无线序列号 + 保持唤醒 ⇒ 启动后两把杠杆全部拉满、停止后逐项还原。
        let adb = FakeAdb {
            devices: vec![device("192.168.1.9:33739", DeviceState::Ready)],
            screen_timeout: Arc::new(Mutex::new(Some(30000))),
            stay_on_bits: Arc::new(Mutex::new(Some(4))),
            ..FakeAdb::default()
        };
        let device_timeout = adb.screen_timeout.clone();
        let device_stay_on = adb.stay_on_bits.clone();
        let runtimes = runtimes(adb, FakeMirror::running());
        let store = SessionStore::default();

        start_mirroring_with(
            &runtimes,
            &store,
            "192.168.1.9:33739".into(),
            SessionOptions::default(),
            None,
        )
        .unwrap();
        assert_eq!(*device_timeout.lock().unwrap(), Some(WIRELESS_KEEP_AWAKE_TIMEOUT_MS));
        assert_eq!(*device_stay_on.lock().unwrap(), Some(STAY_ON_WHILE_PLUGGED_IN_ALL));

        stop_mirroring_with(&store, Some("192.168.1.9:33739".into())).unwrap();
        disable_wireless_keep_awake(runtimes.adb.as_ref(), &store, "192.168.1.9:33739");
        assert_eq!(*device_timeout.lock().unwrap(), Some(30000));
        assert_eq!(*device_stay_on.lock().unwrap(), Some(4));
        assert!(keep_awake_backup_of(&store, "192.168.1.9:33739").is_none());
    }

    #[test]
    fn connecting_a_wireless_device_marks_paired_only_when_idle() {
        let store = SessionStore::default();

        mark_paired_if_idle(&store, "192.168.1.20:41839".into());
        let session = snapshot(&store);
        assert_eq!(session.phase, SessionPhase::Paired);
        assert_eq!(session.serial.as_deref(), Some("192.168.1.20:41839"));

        mark_session(&store, MirrorSession::streaming("192.168.1.20:41839".into()));
        mark_paired_if_idle(&store, "192.168.1.20:5555".into());
        assert_eq!(
            snapshot(&store).phase,
            SessionPhase::Streaming,
            "运行中的会话不得被配对状态覆盖"
        );
    }

    #[test]
    fn forgetting_the_paired_endpoint_returns_to_idle() {
        let store = SessionStore::default();
        mark_paired_if_idle(&store, "192.168.1.20:41839".into());

        clear_paired_if_matches(&store, "192.168.1.20:9999");
        assert_eq!(snapshot(&store).phase, SessionPhase::Paired);

        clear_paired_if_matches(&store, "192.168.1.20:41839");
        assert_eq!(snapshot(&store).phase, SessionPhase::Idle);
    }

    #[test]
    fn every_session_phase_is_serialized_as_a_distinct_snake_case_tag() {
        let phases = [
            SessionPhase::Idle,
            SessionPhase::Unauthorized,
            SessionPhase::Offline,
            SessionPhase::Paired,
            SessionPhase::Connecting,
            SessionPhase::Streaming,
            SessionPhase::Failed,
        ];
        let mut tags: Vec<String> = phases
            .iter()
            .map(|phase| serde_json::to_value(phase).unwrap().as_str().unwrap().to_owned())
            .collect();
        let total = tags.len();
        tags.sort();
        tags.dedup();
        assert_eq!(tags, ["connecting", "failed", "idle", "offline", "paired", "streaming", "unauthorized"]);
        assert_eq!(tags.len(), total);
    }

    // -- 设备能力探测（使用假运行时，无需真机） --

    const PROPERTY_DUMP: &str = "\
[ro.build.version.release]: [13]
[ro.build.version.sdk]: [33]
[ro.product.manufacturer]: [Xiaomi]
[ro.product.brand]: [Redmi]
[ro.product.model]: [M2104K10AC]
[ro.build.characteristics]: [default]
";

    #[test]
    fn parses_capabilities_from_a_property_dump() {
        let capabilities =
            capabilities_from_properties("phone".into(), &parse_device_properties(PROPERTY_DUMP));

        assert_eq!(capabilities.serial, "phone");
        assert_eq!(capabilities.label, "Xiaomi M2104K10AC");
        assert_eq!(capabilities.android_release.as_deref(), Some("13"));
        assert_eq!(capabilities.sdk, Some(33));
        assert_eq!(capabilities.mirroring_supported, Some(true));
        assert_eq!(capabilities.audio_forwarding_supported, Some(true));
    }

    #[test]
    fn capability_support_follows_the_reported_android_version() {
        let probe = |sdk: &str| {
            capabilities_from_properties(
                "phone".into(),
                &parse_device_properties(&format!("[ro.build.version.sdk]: [{sdk}]\n")),
            )
        };

        let older = probe("29");
        assert_eq!(older.mirroring_supported, Some(true));
        assert_eq!(older.audio_forwarding_supported, Some(false));
        assert!(older
            .notices
            .iter()
            .any(|notice| notice.code == "audio_forwarding_unavailable"));

        assert_eq!(probe("30").audio_forwarding_supported, Some(true));

        let too_old = probe("23");
        assert_eq!(too_old.mirroring_supported, Some(false));
        assert!(too_old
            .notices
            .iter()
            .any(|notice| notice.code == "android_too_old"));
    }

    #[test]
    fn unknown_android_version_stays_unknown_instead_of_defaulting_to_supported() {
        let capabilities = capabilities_from_properties(
            "phone".into(),
            &parse_device_properties("[ro.build.version.sdk]: [not-a-number]\n"),
        );

        assert_eq!(capabilities.sdk, None);
        assert_eq!(capabilities.mirroring_supported, None);
        assert_eq!(capabilities.audio_forwarding_supported, None);
        assert!(capabilities
            .notices
            .iter()
            .any(|notice| notice.code == "android_version_unknown"));
    }

    #[test]
    fn device_property_text_is_treated_as_untrusted_input() {
        let hostile = format!(
            "[ro.product.model]: [a\u{7}b\rc{}]\n[ro.build.version.sdk]: [33]\n",
            "x".repeat(500)
        );

        let properties = parse_device_properties(&hostile);
        let model = properties
            .get("ro.product.model")
            .expect("model property must be parsed");
        assert!(!model.chars().any(char::is_control));
        assert!(model.chars().count() <= MAX_PROPERTY_LEN);

        let capabilities = capabilities_from_properties("phone".into(), &properties);
        assert!(!capabilities.label.chars().any(char::is_control));
        assert_eq!(capabilities.sdk, Some(33));
    }

    #[test]
    fn every_capability_report_explains_the_platform_limits() {
        for dump in [PROPERTY_DUMP, "garbage\n", "[ro.build.version.sdk]: [23]\n"] {
            let capabilities =
                capabilities_from_properties("phone".into(), &parse_device_properties(dump));
            let codes: Vec<&str> = capabilities
                .notices
                .iter()
                .map(|notice| notice.code)
                .collect();

            for required in [
                "protected_content",
                "input_restricted_by_apps",
                "oem_differences",
            ] {
                assert!(codes.contains(&required), "缺少受限能力说明：{required}");
            }
            assert!(capabilities
                .notices
                .iter()
                .all(|notice| !notice.title.is_empty() && !notice.detail.is_empty()));
        }
    }

    #[test]
    fn probing_reuses_the_same_device_state_errors_as_starting_a_session() {
        let cases: [(AppRuntimes, &str); 5] = [
            (
                runtimes(
                    FakeAdb::with_devices(vec![device("phone", DeviceState::Unauthorized)]),
                    FakeMirror::running(),
                ),
                "device_unauthorized",
            ),
            (
                runtimes(
                    FakeAdb::with_devices(vec![device("phone", DeviceState::Offline)]),
                    FakeMirror::running(),
                ),
                "device_offline",
            ),
            (runtimes(FakeAdb::unavailable(), FakeMirror::running()), "adb_unavailable"),
            (
                runtimes(FakeAdb::with_devices(Vec::new()), FakeMirror::running()),
                "device_not_connected",
            ),
            (
                runtimes(
                    FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
                    FakeMirror::running(),
                ),
                "probe_failed",
            ),
        ];

        for (runtimes, expected) in cases {
            assert_eq!(
                probe_device_capabilities_with(&runtimes, "phone".into())
                    .unwrap_err()
                    .code,
                expected
            );
        }
    }

    #[test]
    fn probing_a_ready_device_is_read_only() {
        let adb =
            FakeAdb::with_capabilities(vec![device("phone", DeviceState::Ready)], PROPERTY_DUMP);
        let calls = Arc::clone(&adb.calls);
        let runtimes = runtimes(adb, FakeMirror::running());

        let capabilities = probe_device_capabilities_with(&runtimes, "phone".into()).unwrap();

        assert_eq!(capabilities.sdk, Some(33));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], "properties phone");
        assert!(
            calls.iter().all(|call| call.starts_with("properties")),
            "能力探测不得配对、连接或断开设备：{calls:?}"
        );
    }

    #[test]
    fn device_identifiers_are_never_retained_by_the_probe() {
        let dump = "\
[ro.serialno]: [SN1234567890]
[ril.serialnumber]: [SN1234567890]
[gsm.sim.operator.alpha]: [Carrier]
[net.hostname]: [phone]
[ro.build.version.sdk]: [33]
";

        let properties = parse_device_properties(dump);
        assert_eq!(properties.len(), 1);
        assert_eq!(
            properties.get("ro.build.version.sdk").map(String::as_str),
            Some("33")
        );

        let capabilities = capabilities_from_properties("phone".into(), &properties);
        let rendered = serde_json::to_string(&capabilities).unwrap();
        assert!(!rendered.contains("SN1234567890"));
        assert!(!rendered.contains("Carrier"));
        assert!(!rendered.contains("hostname"));
    }

    #[test]
    fn serial_validation_rejects_option_like_and_hostile_input() {
        assert_eq!(
            validate_serial("   ").unwrap_err().code,
            "device_not_selected"
        );
        assert_eq!(
            validate_serial("--help").unwrap_err().code,
            "device_serial_invalid"
        );
        assert_eq!(
            validate_serial("phone; rm -rf /").unwrap_err().code,
            "device_serial_invalid"
        );
        assert_eq!(
            validate_serial("phone\n--serial").unwrap_err().code,
            "device_serial_invalid"
        );
        assert_eq!(
            validate_serial(&"a".repeat(MAX_SERIAL_LEN + 1))
                .unwrap_err()
                .code,
            "device_serial_invalid"
        );
        assert_eq!(validate_serial(" R5CT1 ").unwrap(), "R5CT1");
        assert_eq!(
            validate_serial("192.168.1.20:41839").unwrap(),
            "192.168.1.20:41839"
        );
    }

    // -- 锁屏诊断（样本取自真机 test-runs/keyguard-transition-*.txt）--

    /// Android 13 / Redmi M2104K10AC 实测的 `dumpsys window policy` 片段。
    const SECURE_KEYGUARD_POLICY: &str = "\
  WindowManagerPolicy
    KeyguardServiceDelegate
      showing=true
      showingAndNotOccluded=true
      inputRestricted=false
      occluded=false
      secure=true
      dreaming=false
      systemIsReady=true
      deviceHasKeyguard=true
      enabled=true
";

    #[test]
    fn a_secure_keyguard_is_reported_as_locked_with_secure_lock_enabled() {
        let (keyguard, secure) = parse_keyguard_state(SECURE_KEYGUARD_POLICY);
        assert_eq!(keyguard, KeyguardState::Locked);
        assert_eq!(secure, Some(true));
    }

    #[test]
    fn a_swipe_only_keyguard_is_locked_but_not_secure() {
        let dump = SECURE_KEYGUARD_POLICY.replace("secure=true", "secure=false");
        let (keyguard, secure) = parse_keyguard_state(&dump);
        assert_eq!(keyguard, KeyguardState::Locked);
        assert_eq!(secure, Some(false));
    }

    #[test]
    fn a_dump_without_the_keyguard_block_stays_unknown_instead_of_unlocked() {
        let (keyguard, secure) =
            parse_keyguard_state("  WindowManagerPolicy\n    mSafeMode=false\n");
        assert_eq!(
            keyguard,
            KeyguardState::Unknown,
            "读不到 keyguard 段落时不得默认成已解锁"
        );
        assert_eq!(secure, None);
    }

    #[test]
    fn a_dismissed_keyguard_reads_as_unlocked() {
        let dump = SECURE_KEYGUARD_POLICY.replace("showing=true", "showing=false");
        assert_eq!(parse_keyguard_state(&dump).0, KeyguardState::Unlocked);
    }

    #[test]
    fn screen_state_follows_wakefulness_and_never_guesses() {
        assert_eq!(
            parse_screen_state("  mWakefulness=Asleep\n"),
            ScreenState::Asleep
        );
        assert_eq!(
            parse_screen_state("  mWakefulness=Awake\n"),
            ScreenState::Awake
        );
        assert_eq!(
            parse_screen_state("  mWakefulness=Dozing\n"),
            ScreenState::Asleep
        );
        assert_eq!(parse_screen_state("  nothing here\n"), ScreenState::Unknown);
    }

    #[test]
    fn a_locked_device_is_never_promised_password_free_control() {
        for secure in [Some(true), Some(false), None] {
            let (explanation, recovery) =
                describe_lock_state(KeyguardState::Locked, secure, ScreenState::Awake);
            assert!(
                explanation.contains("锁屏"),
                "锁屏状态必须被明确说出：{explanation}"
            );
            assert!(
                recovery.contains("手机上解锁") || recovery.contains("解锁凭据"),
                "必须把解锁动作交还给用户：{recovery}"
            );
        }
    }

    #[test]
    fn waking_is_offered_only_for_an_unlocked_but_asleep_device() {
        let (_, recovery) =
            describe_lock_state(KeyguardState::Unlocked, Some(true), ScreenState::Asleep);
        assert!(recovery.contains("屏幕唤醒"));

        let (explanation, _) =
            describe_lock_state(KeyguardState::Unknown, None, ScreenState::Unknown);
        assert!(explanation.contains("无法确认"));
    }

    // -- 密码输入页（安全表面）探测与远程解锁键盘 --

    #[test]
    fn pin_pad_probe_activates_only_when_lockscreen_capture_is_blocked() {
        // 真机定案的判据：锁屏中 + 屏幕点亮 + screencap 输出异常小（实测 0 字节）。
        let adb = FakeAdb {
            probe_bytes: Some(0),
            ..FakeAdb::with_lock_state(
                vec![device("phone", DeviceState::Ready)],
                SECURE_KEYGUARD_POLICY,
                "  mWakefulness=Awake\n",
            )
        };
        let runtimes = runtimes(adb, FakeMirror::running());
        let report = pin_pad_probe_with(&runtimes, "phone".into()).unwrap();
        assert!(report.active, "密码页在屏时必须激活：{report:?}");
        assert_eq!(report.detail, "secure_surface_blocked_capture");
    }

    #[test]
    fn pin_pad_probe_stays_inactive_when_capture_looks_normal() {
        // 正常锁屏壁纸页约 3.4~3.7 MB：可捕获 ⇒ 不是密码页，镜像黑屏另有原因。
        let adb = FakeAdb {
            probe_bytes: Some(3_700_000),
            ..FakeAdb::with_lock_state(
                vec![device("phone", DeviceState::Ready)],
                SECURE_KEYGUARD_POLICY,
                "  mWakefulness=Awake\n",
            )
        };
        let runtimes = runtimes(adb, FakeMirror::running());
        let report = pin_pad_probe_with(&runtimes, "phone".into()).unwrap();
        assert!(!report.active);
        assert_eq!(report.detail, "capture_ok");
    }

    #[test]
    fn pin_pad_probe_requires_locked_and_awake_device() {
        // 熄屏时截屏也可能是黑的，但那不是密码页：绝不激活。
        let asleep = FakeAdb {
            probe_bytes: Some(0),
            ..FakeAdb::with_lock_state(
                vec![device("phone", DeviceState::Ready)],
                SECURE_KEYGUARD_POLICY,
                "  mWakefulness=Asleep\n",
            )
        };
        let report = pin_pad_probe_with(&runtimes(asleep, FakeMirror::running()), "phone".into()).unwrap();
        assert!(!report.active);
        assert_eq!(report.detail, "screen_not_awake");

        // 读不到锁屏状态（dumpsys 失败）时保持保守：不激活。
        let unknown = FakeAdb {
            probe_bytes: Some(0),
            ..FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)])
        };
        let report = pin_pad_probe_with(&runtimes(unknown, FakeMirror::running()), "phone".into()).unwrap();
        assert!(!report.active);
        assert_eq!(report.detail, "keyguard_not_showing");

        // 探测本身失败 ≠ 密码页激活：宁可少弹键盘，不可误报。
        let broken = FakeAdb {
            ..FakeAdb::with_lock_state(
                vec![device("phone", DeviceState::Ready)],
                SECURE_KEYGUARD_POLICY,
                "  mWakefulness=Awake\n",
            )
        };
        let report = pin_pad_probe_with(&runtimes(broken, FakeMirror::running()), "phone".into()).unwrap();
        assert!(!report.active);
        assert_eq!(report.detail, "capture_failed");
    }

    #[test]
    fn lock_report_reuses_the_same_device_state_errors_as_starting_a_session() {
        let runtimes = runtimes(
            FakeAdb::with_lock_state(
                vec![device("phone", DeviceState::Unauthorized)],
                SECURE_KEYGUARD_POLICY,
                "  mWakefulness=Asleep\n",
            ),
            FakeMirror::running(),
        );

        assert_eq!(
            lock_report_with(&runtimes, "phone".into()).unwrap_err().code,
            "device_unauthorized"
        );
        assert_eq!(
            lock_report_with(&runtimes, "   ".into()).unwrap_err().code,
            "device_not_selected"
        );
    }

    #[test]
    fn lock_report_is_read_only_and_answers_from_the_device_dump() {
        let probe = FakeAdb::with_lock_state(
            vec![device("phone", DeviceState::Ready)],
            SECURE_KEYGUARD_POLICY,
            "  mWakefulness=Asleep\n",
        );
        let calls = Arc::clone(&probe.calls);
        let runtimes = runtimes(probe, FakeMirror::running());

        let report = lock_report_with(&runtimes, "phone".into()).unwrap();
        assert_eq!(report.keyguard, KeyguardState::Locked);
        assert_eq!(report.secure_lock, Some(true));
        assert_eq!(report.screen, ScreenState::Asleep);
        assert!(!report.explanation.is_empty() && !report.recovery.is_empty());

        let calls = calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .all(|call| call.starts_with("window_policy") || call.starts_with("power_state")),
            "锁屏诊断不得触发任何写操作：{calls:?}"
        );
    }

    #[test]
    fn waking_only_sends_the_wake_key_and_never_an_unlock_sequence() {
        let probe = FakeAdb::with_lock_state(
            vec![device("phone", DeviceState::Ready)],
            SECURE_KEYGUARD_POLICY,
            "  mWakefulness=Asleep\n",
        );
        let calls = Arc::clone(&probe.calls);
        let runtimes = runtimes(probe, FakeMirror::running());

        let serial = validate_serial("phone").unwrap();
        runtimes.adb.wake_screen(&serial).unwrap();

        assert_eq!(calls.lock().unwrap().as_slice(), ["wake phone"]);
    }

    // -- 变暗（DIM）守护：stay_on 拦得住熄屏拦不住变暗，需注入 BACK 拉回 --

    fn dim_display() -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(Some("  mPowerRequest=policy=DIM\n".into())))
    }

    fn bright_display() -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(Some("  mPowerRequest=policy=BRIGHT\n".into())))
    }

    #[test]
    fn display_policy_parser_maps_bright_dim_and_missing() {
        assert_eq!(parse_display_policy("  mPowerRequest=policy=BRIGHT"), DisplayPolicy::Bright);
        assert_eq!(parse_display_policy("  mPowerRequest=policy=DIM, x"), DisplayPolicy::Dim);
        assert_eq!(parse_display_policy("  mPowerRequest=policy=OFF"), DisplayPolicy::Other);
        assert_eq!(parse_display_policy("no such field"), DisplayPolicy::Other);
    }

    #[test]
    fn dimmed_display_is_revived_with_back_without_screen_cycle() {
        let adb = FakeAdb {
            display_dump: dim_display(),
            policy_after_key: Some(DisplayPolicy::Bright),
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&adb.calls);

        assert!(revive_dimmed_display(&adb, "phone"));
        // 只注入 BACK，不触发熄屏-点亮环（屏幕不闪黑）。
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["key KEYCODE_BACK phone", "display_state phone"]
        );
    }

    #[test]
    fn display_cycle_runs_when_back_cannot_restore() {
        // 模拟设备对任何注入都不响应（BACK 后仍 DIM）：revive 走完
        // SLEEP→WAKEUP 环后仍失败，如实返回 false。
        let adb = FakeAdb {
            display_dump: dim_display(),
            policy_after_key: Some(DisplayPolicy::Dim),
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&adb.calls);
        assert!(!revive_dimmed_display(&adb, "phone"));
        let calls = calls.lock().unwrap();
        assert!(calls.iter().any(|c| c.contains("KEYCODE_BACK")));
        assert!(calls.iter().any(|c| c.contains("KEYCODE_SLEEP")));
        assert!(calls.iter().any(|c| c.contains("KEYCODE_WAKEUP")));
    }

    #[test]
    fn wake_command_revives_a_dimmed_screen_instead_of_wake_key() {
        // 用户点「屏幕唤醒」时屏幕卡在 DIM：KEYCODE_WAKEUP 实测无效（设备已
        // Awake），必须注入 BACK。这里验证命令路径选择了正确动作。
        let probe = FakeAdb {
            display_dump: dim_display(),
            policy_after_key: Some(DisplayPolicy::Bright),
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&probe.calls);
        let runtimes = runtimes(probe, FakeMirror::running());

        let serial = validate_serial("phone").unwrap();
        wake_screen_for_serial(&runtimes, &serial).unwrap();

        let calls = calls.lock().unwrap();
        assert!(calls.iter().any(|c| c.contains("KEYCODE_BACK")));
        assert!(!calls.iter().any(|c| c.starts_with("wake ")));
    }

    #[test]
    fn wake_command_is_a_noop_when_screen_is_already_bright() {
        let probe = FakeAdb {
            display_dump: bright_display(),
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&probe.calls);
        let runtimes = runtimes(probe, FakeMirror::running());

        let serial = validate_serial("phone").unwrap();
        wake_screen_for_serial(&runtimes, &serial).unwrap();

        // 屏幕正常亮着时：只读了一次显示策略，没有任何按键注入。
        assert_eq!(calls.lock().unwrap().as_slice(), ["display_state phone"]);
    }

    #[test]
    fn guard_revives_dimmed_screen_only_while_keyguard_is_showing() {
        // 锁屏 + DIM + keep_awake：注入 BACK 恢复（用户输密码的场景）。
        let probe = FakeAdb {
            display_dump: dim_display(),
            policy_after_key: Some(DisplayPolicy::Bright),
            ..FakeAdb::with_lock_state(
                vec![device("phone", DeviceState::Ready)],
                SECURE_KEYGUARD_POLICY,
                "  mWakefulness=Awake\n",
            )
        };
        let calls = Arc::clone(&probe.calls);
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::running(),
        );
        let store = SessionStore::default();
        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None)
            .unwrap();
        let epoch = store.lock().unwrap()["phone"].epoch;

        // 用带状态的 adb 直调 tick（线程循环即逐次调用 tick）。
        assert!(keep_awake_guard_tick(&probe, &store, epoch, "phone"));
        assert!(calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains("KEYCODE_BACK")));
    }

    #[test]
    fn guard_never_injects_keys_when_device_is_unlocked() {
        // 解锁状态下 BACK 会后退用户界面：即使 DIM 也不注入。
        let unlocked_policy = "  KeyguardShowing=false\n  mInputRestricted=false\n";
        let probe = FakeAdb {
            display_dump: dim_display(),
            policy_after_key: Some(DisplayPolicy::Bright),
            ..FakeAdb::with_lock_state(
                vec![device("phone", DeviceState::Ready)],
                unlocked_policy,
                "  mWakefulness=Awake\n",
            )
        };
        let calls = Arc::clone(&probe.calls);
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::running(),
        );
        let store = SessionStore::default();
        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None)
            .unwrap();
        let epoch = store.lock().unwrap()["phone"].epoch;

        assert!(keep_awake_guard_tick(&probe, &store, epoch, "phone"));
        assert!(calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| !c.contains("KEYCODE_")));
    }

    #[test]
    fn guard_stops_when_the_session_epoch_has_moved_on() {
        let probe = FakeAdb {
            display_dump: dim_display(),
            ..FakeAdb::with_lock_state(
                vec![device("phone", DeviceState::Ready)],
                SECURE_KEYGUARD_POLICY,
                "  mWakefulness=Awake\n",
            )
        };
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::running(),
        );
        let store = SessionStore::default();
        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None)
            .unwrap();

        // 用过期的 epoch 调用：守护应当退出（返回 false），不注入任何按键。
        assert!(!keep_awake_guard_tick(&probe, &store, 9999, "phone"));
        assert!(probe
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| !c.contains("KEYCODE_")));
    }

    // -- 桌面模式偏好（X10-90）--

    #[test]
    fn normalize_device_identity_key_extracts_physical_id_from_mdns_endpoint() {
        assert_eq!(
            normalize_device_identity_key("adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp"),
            "qc8d8tonbmmzm7qs"
        );
        // ip:port 端点不含物理 id，原样返回（留给设备在线时反查）。
        assert_eq!(
            normalize_device_identity_key("192.168.1.5:42137"),
            "192.168.1.5:42137"
        );
        // 短物理 id / USB serial 原样返回。
        assert_eq!(normalize_device_identity_key("qc8d8tonbmmzm7qs"), "qc8d8tonbmmzm7qs");
    }

    #[test]
    fn desktop_pref_key_for_resolves_wireless_endpoint_to_physical_serial() {
        let mut device = adb_device(
            "adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp",
            "M2104K10AC",
            DeviceState::Ready,
        );
        device.physical_serial = Some("qc8d8tonbmmzm7qs".into());
        let devices = vec![device];
        // 设备在线：反查 physical_serial（稳定，重连不变）。
        assert_eq!(
            desktop_pref_key_for(&devices, "adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp"),
            "qc8d8tonbmmzm7qs"
        );
        // 设备离线/查不到：归一化兜底，仍能命中同一台手机的 pref。
        assert_eq!(
            desktop_pref_key_for(&[], "adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp"),
            "qc8d8tonbmmzm7qs"
        );
    }

    // -- 最近设备 --

    fn adb_device(serial: &str, label: &str, state: DeviceState) -> AdbDevice {
        let kind = if is_wireless_endpoint(serial) {
            ConnectionKind::Wireless
        } else {
            ConnectionKind::Usb
        };
        AdbDevice {
            serial: serial.into(),
            label: label.into(),
            state,
            physical_serial: None,
            connections: vec![ConnectionEndpoint {
                serial: serial.into(),
                kind,
                state,
            }],
        }
    }

    #[test]
    fn tray_connect_target_prefers_a_ready_recent_device() {
        let recent = vec![RecentDevice {
            serial: "b".into(),
            label: "B".into(),
            last_used_at: 2,
        }];
        let devices = vec![
            adb_device("a", "A", DeviceState::Ready),
            adb_device("b", "B", DeviceState::Ready),
        ];
        // 最近设备优先于列表顺序：不是第一台就绪设备，而是「我刚才用的那台」。
        assert_eq!(
            pick_tray_target(&recent, &devices),
            Some(("b".into(), "B".into()))
        );
    }

    #[test]
    fn tray_connect_target_falls_back_to_the_first_ready_device() {
        let recent = vec![RecentDevice {
            serial: "gone".into(),
            label: "已拔出".into(),
            last_used_at: 9,
        }];
        let devices = vec![
            adb_device("u", "U", DeviceState::Unauthorized),
            adb_device("a", "A", DeviceState::Ready),
        ];
        // 最近记录里的设备不在线时，取第一台就绪设备；未授权设备不能作为目标。
        assert_eq!(
            pick_tray_target(&recent, &devices),
            Some(("a".into(), "A".into()))
        );
    }

    #[test]
    fn tray_connect_target_is_none_without_ready_devices() {
        let devices = vec![adb_device("u", "U", DeviceState::Unauthorized)];
        assert_eq!(pick_tray_target(&[], &devices), None);
    }

    #[test]
    fn desktop_prefs_round_trip_through_disk_and_tolerate_garbage() {
        let path = std::env::temp_dir().join(format!(
            "mirrordock-desktop-prefs-test-{}.json",
            std::process::id()
        ));
        let mut prefs = std::collections::HashMap::new();
        prefs.insert(
            "qc8d8tonbmmzm7qs".to_string(),
            DesktopPrefEntry {
                desktop_mode: true,
                desktop_app: Some("com.netease.dhxy.qihoo".into()),
            },
        );
        save_desktop_prefs(&path, &prefs).expect("写入应成功");
        let loaded = load_desktop_prefs(&path);
        assert_eq!(loaded.get("qc8d8tonbmmzm7qs"), prefs.get("qc8d8tonbmmzm7qs"));
        // 文件被手改坏 → 返回空表而非 panic/报错（与前端容错策略一致）。
        fs::write(&path, b"not json").unwrap();
        assert!(load_desktop_prefs(&path).is_empty());
        // 不存在的文件 → 空表。
        let _ = fs::remove_file(&path);
        assert!(load_desktop_prefs(&path).is_empty());
    }

    #[test]
    fn recent_devices_dedupe_and_keep_the_most_recent_first() {
        let mut devices = Vec::new();
        record_recent_device(
            &mut devices,
            RecentDevice {
                serial: "a".into(),
                label: "A".into(),
                last_used_at: 1,
            },
        );
        record_recent_device(
            &mut devices,
            RecentDevice {
                serial: "b".into(),
                label: "B".into(),
                last_used_at: 2,
            },
        );
        record_recent_device(
            &mut devices,
            RecentDevice {
                serial: "a".into(),
                label: "A2".into(),
                last_used_at: 3,
            },
        );

        assert_eq!(
            devices
                .iter()
                .map(|device| device.serial.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(devices[0].label, "A2");
        assert_eq!(devices[0].last_used_at, 3);
    }

    #[test]
    fn recent_devices_are_capped_at_the_declared_limit() {
        let mut devices = Vec::new();
        for index in 0..(MAX_RECENT_DEVICES + 5) {
            record_recent_device(
                &mut devices,
                RecentDevice {
                    serial: format!("device-{index}"),
                    label: format!("设备 {index}"),
                    last_used_at: index as u64,
                },
            );
        }

        assert_eq!(devices.len(), MAX_RECENT_DEVICES);
        assert_eq!(
            devices[0].serial,
            format!("device-{}", MAX_RECENT_DEVICES + 4)
        );
    }

    #[test]
    fn forgetting_a_recent_device_removes_only_that_record() {
        let mut devices = Vec::new();
        for (serial, label, at) in [("usb-a", "A", 1u64), ("192.168.1.20:37123", "B", 2)] {
            record_recent_device(
                &mut devices,
                RecentDevice {
                    serial: serial.into(),
                    label: label.into(),
                    last_used_at: at,
                },
            );
        }

        assert!(forget_recent_device_from(&mut devices, "usb-a"));
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].serial, "192.168.1.20:37123");

        // 移除不存在的记录不算成功，也不得改动列表；重复移除必须幂等。
        assert!(!forget_recent_device_from(&mut devices, "usb-a"));
        assert_eq!(devices.len(), 1);
    }

    #[test]
    fn a_recent_device_record_is_removable_from_disk() {
        let directory = std::env::temp_dir().join(format!(
            "mirrordock-recent-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        let path = directory.join("recent-devices.json");

        assert!(load_recent_devices(&path).unwrap().is_empty());
        save_recent_devices(
            &path,
            &[RecentDevice {
                serial: "usb-a".into(),
                label: "A".into(),
                last_used_at: 7,
            }],
        )
        .unwrap();

        let remaining = forget_recent_device_at(&path, "usb-a").unwrap();
        assert!(remaining.is_empty());
        assert!(
            load_recent_devices(&path).unwrap().is_empty(),
            "删除必须真正落盘，不能只改内存"
        );

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn clearing_recent_devices_wipes_the_file_physically() {
        let directory = std::env::temp_dir().join(format!(
            "mirrordock-recent-clear-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        let path = directory.join("recent-devices.json");

        save_recent_devices(
            &path,
            &[
                RecentDevice { serial: "usb-a".into(), label: "A".into(), last_used_at: 7 },
                RecentDevice { serial: "192.168.1.20:37123".into(), label: "B".into(), last_used_at: 8 },
            ],
        )
        .unwrap();

        // 清空命令 = 物理删除：落盘空列表，重启后也不会回来。
        save_recent_devices(&path, &[]).unwrap();
        assert!(load_recent_devices(&path).unwrap().is_empty());

        let _ = fs::remove_dir_all(&directory);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn show_focus_policy_never_revives_dock_icon_when_hidden() {
        // 开启隐藏 Dock：回到主窗口绝不切回 Regular，否则图标会被拉出来。
        assert!(show_focus_policy(true).is_none());
        // 未开启：切回 Regular 抢前台，行为不变。
        assert!(matches!(
            show_focus_policy(false),
            Some(tauri::ActivationPolicy::Regular)
        ));
    }

    #[test]
    fn app_settings_default_when_missing_or_corrupt() {        let directory = std::env::temp_dir().join(format!(
            "mirrordock-settings-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        let path = directory.join("nested").join("app-settings.json");

        // 缺文件 → 默认值：设置坏了/没写过都不该挡住应用启动。
        assert!(!load_app_settings(&path).hide_dock_icon);

        // 损坏文件 → 默认值，而不是报错退出。
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ not json").unwrap();
        assert!(!load_app_settings(&path).hide_dock_icon);

        // 空对象（旧版本缺字段）→ serde(default) 兜底。
        fs::write(&path, b"{}").unwrap();
        assert_eq!(load_app_settings(&path), AppSettings::default());

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn app_settings_roundtrip_on_disk() {
        let directory = std::env::temp_dir().join(format!(
            "mirrordock-settings-roundtrip-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        let path = directory.join("app-settings.json");

        let enabled = AppSettings { hide_dock_icon: true, auto_reconnect: true, recording_dir: None };
        save_app_settings(&path, &enabled).unwrap();
        assert_eq!(load_app_settings(&path), enabled);

        let disabled = AppSettings::default();
        save_app_settings(&path, &disabled).unwrap();
        assert_eq!(load_app_settings(&path), disabled);

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn stay_awake_is_on_by_default_and_can_be_turned_off() {
        assert!(SessionOptions::default().keep_awake);
        assert!(SessionOptions::default()
            .arguments()
            .unwrap()
            .contains(&"--stay-awake".into()));

        let off = SessionOptions {
            keep_awake: false,
            ..Default::default()
        };
        assert!(!off.arguments().unwrap().contains(&"--stay-awake".into()));

        // 字段缺省时走 serde 默认值，同样是开启。
        let parsed: SessionOptions = serde_json::from_str(r#"{"rotation":90}"#).unwrap();
        assert!(parsed.keep_awake);
    }

    #[test]
    fn clipboard_autosync_is_on_by_default_and_only_sends_the_disable_flag_when_turned_off() {
        // 默认不传任何剪贴板参数：自动同步是 scrcpy 的默认行为，MirrorDock 不画蛇添足。
        let args = SessionOptions::default().arguments().unwrap();
        assert!(!args
            .iter()
            .any(|argument| argument.contains("clipboard")));

        let off = SessionOptions {
            clipboard_autosync: false,
            ..Default::default()
        };
        assert!(off
            .arguments()
            .unwrap()
            .contains(&"--no-clipboard-autosync".into()));

        // 字段缺省时同样视为开启（隐私开关默认关闭 = 同步默认开启）。
        let parsed: SessionOptions = serde_json::from_str(r#"{"rotation":0}"#).unwrap();
        assert!(parsed.clipboard_autosync);
    }

    #[test]
    fn audio_forwarding_is_on_by_default_and_only_sends_the_disable_flag_when_turned_off() {
        // 默认不传任何音频参数：转发系统声音是 scrcpy 的默认行为（`--audio-source`
        // 默认 `output`），MirrorDock 不画蛇添足；麦克风采集不在选项之内。
        let args = SessionOptions::default().arguments().unwrap();
        assert!(!args.iter().any(|argument| argument.contains("audio")));

        // 关闭后显式传 `--no-audio`：让 scrcpy 的行为由用户可感知的开关决定。
        let off = SessionOptions {
            audio: false,
            ..Default::default()
        };
        assert!(off.arguments().unwrap().contains(&"--no-audio".into()));

        // 字段缺省时同样视为开启。
        let parsed: SessionOptions = serde_json::from_str(r#"{"rotation":0}"#).unwrap();
        assert!(parsed.audio);
    }

    #[test]
    fn shortcut_modifier_is_whitelisted_and_off_by_default() {
        // 默认不传参数：沿用 scrcpy 默认修饰键（左 Alt / 左 Super）。
        let args = SessionOptions::default().arguments().unwrap();
        assert!(!args.iter().any(|argument| argument.contains("shortcut-mod")));

        // 白名单内的值原样透传为固定参数。
        let custom = SessionOptions {
            shortcut_mod: Some("rctrl".to_owned()),
            ..Default::default()
        };
        assert!(custom
            .arguments()
            .unwrap()
            .contains(&"--shortcut-mod=rctrl".into()));

        // 白名单外的值拒绝——不把任意字符串透传进 scrcpy 参数。
        let hostile = SessionOptions {
            shortcut_mod: Some("--video-codec=h265".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            hostile.arguments().unwrap_err().code,
            "shortcut_mod_invalid"
        );
    }

    #[test]
    fn show_touches_and_read_only_are_off_by_default_and_send_fixed_flags() {
        // 两个演示相关开关默认关闭，不传任何参数。
        let args = SessionOptions::default().arguments().unwrap();
        assert!(!args.iter().any(|argument| argument.contains("show-touches")));
        assert!(!args.iter().any(|argument| argument.contains("no-control")));

        let demo = SessionOptions {
            show_touches: true,
            read_only: true,
            ..Default::default()
        };
        let args = demo.arguments().unwrap();
        assert!(args.contains(&"--show-touches".into()));
        assert!(args.contains(&"--no-control".into()));

        // 字段缺省时同样视为关闭。
        let parsed: SessionOptions = serde_json::from_str(r#"{"rotation":0}"#).unwrap();
        assert!(!parsed.show_touches);
        assert!(!parsed.read_only);
        assert!(parsed.shortcut_mod.is_none());
    }

    #[test]
    fn auto_reconnect_only_for_abnormal_exits_and_all_connection_kinds() {
        // 异常退出 + 开关开 → 重连；无线与 USB 一视同仁（X10-59：拔线实测
        // 旧版「不提示也不重连」被感知为闪退，现在 USB 等重新插线自动恢复）。
        assert!(should_auto_reconnect(false, "192.168.2.224:46289", true));
        assert!(should_auto_reconnect(false, "ABC123456", true));
        // 用户主动关闭镜像窗口（正常退出）→ 不打扰。
        assert!(!should_auto_reconnect(true, "192.168.2.224:46289", true));
        assert!(!should_auto_reconnect(true, "ABC123456", true));
        // 用户关闭了自动重连。
        assert!(!should_auto_reconnect(false, "192.168.2.224:46289", false));
        assert!(!should_auto_reconnect(false, "ABC123456", false));
    }

    #[test]
    fn reconnect_probe_classifies_device_readiness() {
        let endpoint = "192.168.2.224:46289";
        // 设备就绪 → Ready。
        let ready = FakeAdb {
            devices: vec![device(endpoint, DeviceState::Ready)],
            ..FakeAdb::default()
        };
        assert_eq!(classify_reconnect_probe(&ready, endpoint), ReconnectProbe::Ready);
        // 设备在表里但未授权 → 继续等。
        let unauthorized = FakeAdb {
            devices: vec![device(endpoint, DeviceState::Unauthorized)],
            ..FakeAdb::default()
        };
        assert_eq!(classify_reconnect_probe(&unauthorized, endpoint), ReconnectProbe::NotReady);
        // 设备还没回来 → 继续等。
        let absent = FakeAdb::default();
        assert_eq!(classify_reconnect_probe(&absent, endpoint), ReconnectProbe::NotReady);
        // adb 服务不可用 → 继续等（服务可能正在重启）。
        let down = FakeAdb { unavailable: true, ..FakeAdb::default() };
        assert_eq!(classify_reconnect_probe(&down, endpoint), ReconnectProbe::Unavailable);
    }

    #[test]
    fn max_fps_defaults_to_unlimited_and_whitelists_values() {
        // 默认不限帧率：不传任何 --max-fps 参数（跟随设备）。
        let args = SessionOptions::default().arguments().unwrap();
        assert!(!args.iter().any(|argument| argument.contains("max-fps")));

        // 白名单内的值原样透传。
        for fps in [24, 30, 60] {
            let limited = SessionOptions {
                max_fps: Some(fps),
                ..Default::default()
            };
            assert!(limited.arguments().unwrap().contains(&format!("--max-fps={fps}")));
        }

        // 白名单外一律拒绝，不透传任意数字。
        let hostile = SessionOptions {
            max_fps: Some(999),
            ..Default::default()
        };
        assert_eq!(hostile.arguments().unwrap_err().code, "max_fps_invalid");

        // 旧配置（缺字段）经 serde 默认视为不限帧率。
        let parsed: SessionOptions = serde_json::from_str(r#"{"rotation":0}"#).unwrap();
        assert!(parsed.max_fps.is_none());
    }

    #[test]
    fn video_sources_are_mutually_exclusive_and_camera_forces_no_audio() {
        // 默认（屏幕源）不传任何视频源参数。
        let args = SessionOptions::default().arguments().unwrap();
        assert!(!args.iter().any(|a| a.contains("new-display")));
        assert!(!args.iter().any(|a| a.contains("video-source")));

        // 桌面模式：--new-display。
        let desktop = SessionOptions {
            desktop_mode: true,
            ..Default::default()
        };
        assert!(desktop.arguments().unwrap().contains(&"--new-display".into()));

        // 摄像头源：--video-source=camera，且强制 --no-audio（不采集麦克风）。
        let camera = SessionOptions {
            camera_source: true,
            ..Default::default()
        };
        let args = camera.arguments().unwrap();
        assert!(args.contains(&"--video-source=camera".into()));
        assert!(args.contains(&"--no-audio".into()));

        // 互斥：同时开启直接拒绝。
        let conflict = SessionOptions {
            desktop_mode: true,
            camera_source: true,
            ..Default::default()
        };
        assert_eq!(
            conflict.arguments().unwrap_err().code,
            "video_source_conflict"
        );

        // 旧配置（缺字段）视为都关闭。
        let parsed: SessionOptions = serde_json::from_str(r#"{"rotation":0}"#).unwrap();
        assert!(!parsed.desktop_mode);
        assert!(!parsed.camera_source);
    }

    /// 桌面模式虚拟屏启动应用（X10-53）：包名经白名单校验后透传 `--start-app`，
    /// 非法包名拒绝（不把任意字符串拼进参数）；未填则只创建虚拟屏。
    #[test]
    fn desktop_app_composes_start_app_and_rejects_invalid_packages() {
        // 合法包名：与 --new-display 同时出现。
        let with_app = SessionOptions {
            desktop_mode: true,
            desktop_app: Some(" com.android.browser ".into()),
            ..Default::default()
        };
        let args = with_app.arguments().unwrap();
        assert!(args.contains(&"--new-display".into()));
        assert!(args.contains(&"--start-app=com.android.browser".into()));

        // 留空（None）只创建虚拟屏，不传 --start-app。
        let no_app = SessionOptions {
            desktop_mode: true,
            ..Default::default()
        };
        assert!(!no_app.arguments().unwrap().iter().any(|a| a.contains("start-app")));

        // 非法字符（空格/斜杠/冒号/中文）一律拒绝。
        for hostile in ["com evil app", "com/evil", "com:8080", "应用"] {
            let bad = SessionOptions {
                desktop_mode: true,
                desktop_app: Some(hostile.into()),
                ..Default::default()
            };
            assert_eq!(
                bad.arguments().unwrap_err().code,
                "desktop_app_invalid",
                "包名 {hostile} 应被拒绝"
            );
        }

        // 过长的包名同样拒绝（防参数爆炸）。
        let too_long = SessionOptions {
            desktop_mode: true,
            desktop_app: Some("a.".repeat(80)),
            ..Default::default()
        };
        assert_eq!(too_long.arguments().unwrap_err().code, "desktop_app_invalid");

        // 摄像头源模式下忽略 desktop_app（不传 --start-app）。
        let camera = SessionOptions {
            camera_source: true,
            desktop_app: Some("com.android.browser".into()),
            ..Default::default()
        };
        assert!(!camera.arguments().unwrap().iter().any(|a| a.contains("start-app")));

        // X10-79：异常退出时把 scrcpy 输出尾部附进恢复建议，用户能看到原文。
        let exited = resolve_process_exit("phone", false, "ERROR: Could not open icon image\n[server] INFO: Cleaning up");
        let error = exited.error.expect("异常退出必须是失败态");
        assert!(error.recovery.contains("scrcpy 输出"), "恢复建议应带上 scrcpy 原文");
        assert!(error.recovery.contains("Could not open icon image"));
        // 正常退出与空输出都不附加。
        assert!(resolve_process_exit("phone", true, "whatever").error.is_none());
        assert!(!resolve_process_exit("phone", false, "  \n ")
            .error
            .unwrap()
            .recovery
            .contains("scrcpy 输出"));

        // X10-80：从 scrcpy 输出解析虚拟屏 id。
        let output = "[server] INFO: New display: 1072x2400/440 (id=84)\n[server] INFO: Starting app \"大话西游\" [com.netease.dhxy.qihoo] on display 84...";
        assert_eq!(parse_desktop_display_id(output), Some(84));
        assert_eq!(parse_desktop_display_id("没有任何线索"), None);

        // X10-80：落地判定——真机失败案例的形状：虚拟屏分段里没有该应用的记录。
        let dumpsys = "Display #84 (activities from top to bottom):\n  * Task{... type=standard A=1000:com.android.settings.root ...}\n    packageName=com.android.settings\nDisplay #0 (activities from top to bottom):\n    packageName=com.netease.dhxy.qihoo\n";
        // 应用在别的屏（#0）不算落地 —— 正是真机上「游戏跑到主屏之外消失」的形状。
        assert!(!desktop_app_landed(dumpsys, Some(84), "com.netease.dhxy.qihoo"));
        // 在指定屏上有记录才算落地。
        let landed = "Display #84 (activities from top to bottom):\n    packageName=com.netease.dhxy.qihoo\n";
        assert!(desktop_app_landed(landed, Some(84), "com.netease.dhxy.qihoo"));
        // 解析不出分段时退化为全局查找（宁可不报，不误报）。
        assert!(desktop_app_landed(dumpsys, None, "com.netease.dhxy.qihoo"));

        // 旧配置（缺字段）视为未填。
        let parsed: SessionOptions = serde_json::from_str(r#"{"desktop_mode":true}"#).unwrap();
        assert!(parsed.desktop_app.is_none());
    }

    #[test]
    fn desktop_recording_captures_the_display_channel_virtual_screen() {
        // X10-103：桌面模式录制用 --display-id 捕获显示通道那块虚拟屏，不建第二块、
        // 不带 --start-app（否则挤掉显示通道→闪退白屏，或录到一块空屏）。
        let desktop = SessionOptions {
            desktop_mode: true,
            desktop_app: Some("com.netease.dhxy.qihoo".into()),
            ..Default::default()
        };
        let args = desktop.record_arguments(Some(105)).unwrap();
        assert!(args.contains(&"--display-id=105".into()), "应捕获指定虚拟屏：{args:?}");
        assert!(!args.iter().any(|a| a.contains("new-display")), "录制不建第二块屏：{args:?}");
        assert!(!args.iter().any(|a| a.contains("start-app")), "录制不重复拉起应用：{args:?}");

        // 拿不到显示通道虚拟屏 id 时拒绝启动（而非静默录空屏）。
        let err = desktop.record_arguments(None).unwrap_err();
        assert_eq!(err.code, "desktop_display_unknown");

        // 非桌面模式（手机屏幕）：不需要 display-id，保持 --no-control。
        let phone = SessionOptions::default();
        let phone_args = phone.record_arguments(None).unwrap();
        assert!(phone_args.contains(&"--no-control".into()));
        assert!(!phone_args.iter().any(|a| a.contains("display-id")));
    }

    #[test]
    fn device_apps_are_parsed_from_pm_list_output_and_sorted() {
        let adb = FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            apps_output: Some(
                "package:com.miui.home\npackage:com.android.browser\n\npackage:org.mozilla.firefox\npackage:com.android.browser\n"
                    .to_owned(),
            ),
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        let apps = list_device_apps_scoped(&runtimes, "phone".into(), |_| None).unwrap();
        assert_eq!(
            apps,
            vec![
                DeviceApp {
                    package: "com.android.browser".into(),
                    name: "com.android.browser".into(),
                },
                DeviceApp {
                    package: "com.miui.home".into(),
                    name: "com.miui.home".into(),
                },
                DeviceApp {
                    package: "org.mozilla.firefox".into(),
                    name: "org.mozilla.firefox".into(),
                },
            ]
        );
        assert!(calls.lock().unwrap().iter().any(|c| c.contains("list_device_apps")));

        // 未就绪设备拒绝（与文件传输同一套就绪门槛）。
        let offline = FakeAdb::with_devices(vec![device("phone", DeviceState::Unauthorized)]);
        let runtimes = screenshot_runtimes(offline);
        assert_eq!(
            list_device_apps_scoped(&runtimes, "phone".into(), |_| None)
                .unwrap_err()
                .code,
            "device_unauthorized"
        );
    }

    #[test]
    fn scrcpy_app_list_parses_names_and_ignores_noise() {
        let sample = "[server] INFO: Device: [Xiaomi] Redmi M2104K10AC (Android 13)\n\
                      [server] INFO: List of apps:\n\
                      * 浏览器                            com.android.browser\n\
                      * MirrorDock Mirror                 com.gstar.mirrordock\n\
                      * 相册                              com.miui.gallery\n\
                      * 下载管理                          com.android.providers.downloads.ui\n\
                      * 下载管理                          com.android.providers.downloads.ui\n\
                      garbage line without package\n";
        let apps = parse_scrcpy_app_list(sample);
        // 按应用名排序（大小写不敏感）、按包名去重。
        assert_eq!(
            apps,
            vec![
                DeviceApp {
                    package: "com.gstar.mirrordock".into(),
                    name: "MirrorDock Mirror".into(),
                },
                DeviceApp {
                    package: "com.android.providers.downloads.ui".into(),
                    name: "下载管理".into(),
                },
                DeviceApp {
                    package: "com.android.browser".into(),
                    name: "浏览器".into(),
                },
                DeviceApp {
                    package: "com.miui.gallery".into(),
                    name: "相册".into(),
                },
            ]
        );

        // 控制字符被剔除（设备输出属于不可信输入）。
        let hostile = parse_scrcpy_app_list("* 名\u{7}字\tcom.example.app\n");
        assert_eq!(hostile.len(), 1);
        assert_eq!(hostile[0].name, "名字");
        assert_eq!(hostile[0].package, "com.example.app");
    }

    #[test]
    fn named_apps_prefer_scrcpy_output_and_fall_back_to_pm_list() {
        let make = || FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            apps_output: Some("package:com.fallback.app\n".to_owned()),
            ..FakeAdb::default()
        };

        // scrcpy 路径成功：直接采用，不读 pm list。
        let adb = make();
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);
        let apps = list_device_apps_scoped(&runtimes, "phone".into(), |_| {
            Some("* 浏览器 com.android.browser\n".to_owned())
        })
        .unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "浏览器");
        assert!(!calls.lock().unwrap().iter().any(|c| c.contains("list_device_apps")));

        // scrcpy 失败：回退 pm list，应用名退化为包名。
        let runtimes = screenshot_runtimes(make());
        let apps = list_device_apps_scoped(&runtimes, "phone".into(), |_| None).unwrap();
        assert_eq!(
            apps,
            vec![DeviceApp {
                package: "com.fallback.app".into(),
                name: "com.fallback.app".into(),
            }]
        );

        // scrcpy 输出只有日志、解析不出应用：同样回退。
        let runtimes = screenshot_runtimes(make());
        let apps = list_device_apps_scoped(&runtimes, "phone".into(), |_| {
            Some("[server] INFO: nothing\n".to_owned())
        })
        .unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].package, "com.fallback.app");
    }

    #[test]
    fn keyboard_mode_defaults_to_uhid_and_falls_back_to_scrcpy_injection() {
        // 默认 UHID 物理键盘：软键盘收起，电脑键盘直接打字。
        let args = SessionOptions::default().arguments().unwrap();
        assert!(args.contains(&"--keyboard=uhid".into()));

        // 显式关闭：回到 scrcpy 注入模式，仍是显式传参不依赖 scrcpy 默认。
        let injected = SessionOptions {
            keyboard_uhid: false,
            ..Default::default()
        };
        let args = injected.arguments().unwrap();
        assert!(args.contains(&"--keyboard=scrcpy".into()));
        assert!(!args.iter().any(|argument| argument.contains("uhid")));

        // 旧配置（缺字段）经 serde 默认走 UHID。
        let parsed: SessionOptions = serde_json::from_str(r#"{"rotation":0}"#).unwrap();
        assert!(parsed.keyboard_uhid);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mirror_bundle_wraps_scrcpy_with_icon_and_plist() {
        let directory = std::env::temp_dir().join(format!(
            "mirrordock-bundle-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let runtime = directory.join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        // 假的 scrcpy / adb / scrcpy-server / 图标源。
        let scrcpy = runtime.join("scrcpy");
        std::fs::write(&scrcpy, b"fake-scrcpy").unwrap();
        let adb = directory.join("adb");
        std::fs::write(&adb, b"fake-adb").unwrap();
        std::fs::write(runtime.join("scrcpy-server"), b"fake-server").unwrap();
        let icon = directory.join("icon.icns");
        std::fs::write(&icon, b"fake-icns").unwrap();

        let exec = macos_mirror_bundle_exec(&scrcpy, &adb).expect("bundle 构建应成功");
        assert!(exec.ends_with("MirrorDock Mirror.app/Contents/MacOS/scrcpy"));
        let contents = exec.parent().unwrap().parent().unwrap();
        assert_eq!(
            std::fs::read(contents.join("MacOS/scrcpy")).unwrap(),
            b"fake-scrcpy"
        );
        assert_eq!(
            std::fs::read(contents.join("MacOS/adb")).unwrap(),
            b"fake-adb"
        );
        assert!(contents.join("MacOS/scrcpy-server").is_file());
        // 图标由 app_icon_icns() 解析（发行包/仓库内），这里只验证已复制到位。
        assert!(contents.join("Resources/AppIcon.icns").is_file());
        let plist = std::fs::read_to_string(contents.join("Info.plist")).unwrap();
        assert!(plist.contains("com.mirrordock.mirror"));
        assert!(plist.contains("MirrorDock 镜像"));
        assert!(plist.contains("AppIcon"));

        let _ = std::fs::remove_dir_all(&directory);
    }

    // -- 截图：可见、可撤销、失败必须能被发现 --

    /// 一个只有文件头的最小 PNG：足以通过「这确实是图片」的判定。
    const TINY_PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D',
        b'R',
    ];

    fn scratch_dir(tag: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("mirrordock-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    fn screenshot_runtimes(adb: FakeAdb) -> AppRuntimes {
        runtimes(adb, FakeMirror::running())
    }

    #[test]
    fn a_captured_screen_is_written_to_disk_and_reported_back() {
        let directory = scratch_dir("screenshot-save");
        let runtimes = screenshot_runtimes(FakeAdb::with_screenshot(
            vec![device("phone", DeviceState::Ready)],
            TINY_PNG.to_vec(),
        ));

        let saved = capture_screenshot_into(&runtimes, &directory, "phone".into(), "shot.png".into())
            .unwrap();

        assert_eq!(saved.file_name, "shot.png");
        assert_eq!(saved.bytes, TINY_PNG.len());
        assert_eq!(
            fs::read(directory.join("shot.png")).unwrap(),
            TINY_PNG,
            "写出的字节必须与设备返回的完全一致"
        );
        assert!(saved.path.ends_with("shot.png"));

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_payload_that_is_not_an_image_is_never_written_as_a_screenshot() {
        let directory = scratch_dir("screenshot-corrupt");
        let runtimes = screenshot_runtimes(FakeAdb::returning_corrupt_screenshot(vec![device(
            "phone",
            DeviceState::Ready,
        )]));

        let error =
            capture_screenshot_into(&runtimes, &directory, "phone".into(), "shot.png".into())
                .unwrap_err();

        assert_eq!(error.code, "screenshot_not_image");
        assert!(
            !directory.join("shot.png").exists(),
            "命令成功但内容不是图片时，绝不能留下一个打不开的文件"
        );

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_screenshot_file_name_can_never_escape_the_screenshot_directory() {
        for name in [
            "../escape.png",
            "sub/dir.png",
            "sub\\dir.png",
            ".hidden.png",
            "shot.jpg",
            "",
            "   ",
        ] {
            assert!(
                validate_screenshot_name(name).is_err(),
                "文件名 {name:?} 必须被拒绝"
            );
        }
        assert!(validate_screenshot_name("MirrorDock-20260928-171825.png").is_ok());

        let too_long = format!("{}.png", "a".repeat(MAX_MEDIA_NAME_LEN));
        assert!(validate_screenshot_name(&too_long).is_err());
    }

    #[test]
    fn two_screenshots_in_the_same_second_do_not_overwrite_each_other() {
        let directory = scratch_dir("screenshot-collision");
        let runtimes = screenshot_runtimes(FakeAdb::with_screenshot(
            vec![device("phone", DeviceState::Ready)],
            TINY_PNG.to_vec(),
        ));

        let first =
            capture_screenshot_into(&runtimes, &directory, "phone".into(), "shot.png".into())
                .unwrap();
        let second =
            capture_screenshot_into(&runtimes, &directory, "phone".into(), "shot.png".into())
                .unwrap();

        assert_eq!(first.file_name, "shot.png");
        assert_eq!(second.file_name, "shot-1.png");
        assert!(directory.join("shot.png").exists());
        assert!(directory.join("shot-1.png").exists());

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_screenshot_can_be_undone_and_missing_files_are_reported_honestly() {
        let directory = scratch_dir("screenshot-delete");
        let runtimes = screenshot_runtimes(FakeAdb::with_screenshot(
            vec![device("phone", DeviceState::Ready)],
            TINY_PNG.to_vec(),
        ));
        capture_screenshot_into(&runtimes, &directory, "phone".into(), "shot.png".into()).unwrap();

        remove_screenshot_file(&directory, "shot.png").unwrap();
        assert!(!directory.join("shot.png").exists());

        let error = remove_screenshot_file(&directory, "shot.png").unwrap_err();
        assert_eq!(error.code, "screenshot_missing");

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_unauthorized_device_is_refused_before_any_screen_is_read() {
        let directory = scratch_dir("screenshot-unauthorized");
        let adb = FakeAdb::with_screenshot(
            vec![device("phone", DeviceState::Unauthorized)],
            TINY_PNG.to_vec(),
        );
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        let error =
            capture_screenshot_into(&runtimes, &directory, "phone".into(), "shot.png".into())
                .unwrap_err();

        assert_eq!(error.code, "device_unauthorized");
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|call| call.starts_with("screenshot")),
            "设备未授权时不得去读屏幕内容"
        );

        let _ = fs::remove_dir_all(&directory);
    }

    // -- 录制：默认关闭、路径交给镜像进程、可撤销 --

    #[test]
    fn a_recording_is_never_prepared_unless_the_session_asked_for_one() {
        assert!(
            !SessionOptions::default().record,
            "录制会产生一份屏幕副本，必须由用户显式开启"
        );

        let mirror = FakeMirror::running();
        let records = Arc::clone(&mirror.records);
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            mirror,
        );
        let store = SessionStore::default();
        start_mirroring_with(
            &runtimes,
            &store,
            "phone".into(),
            SessionOptions::default(),
            None,
        )
        .unwrap();

        assert_eq!(
            records.lock().unwrap().clone(),
            vec![None],
            "没开录制时不得给镜像进程传任何录制路径"
        );
        assert!(current_recording_with(&store).unwrap().is_none());
    }

    #[test]
    fn the_recording_path_goes_to_a_separate_recorder_never_the_display_process() {
        let directory = scratch_dir("recording-start");
        let mirror = FakeMirror::running();
        let records = Arc::clone(&mirror.records);
        let recorder_records = Arc::clone(&mirror.recorder_records);
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            mirror,
        );
        let store = SessionStore::default();
        let path = directory.join("MirrorDock-20260928-171825.mp4");

        // X10-92（方案甲）：开始镜像即使带 record=true 也绝不把录制塞进显示进程。
        start_mirroring_with(
            &runtimes,
            &store,
            "phone".into(),
            SessionOptions {
                record: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();

        let recorded = records.lock().unwrap().clone();
        assert_eq!(
            recorded[0], None,
            "显示进程永远不带录制路径（录制走独立通道）"
        );

        // 经 start_recorder 开录：路径原样交给独立录制进程，不碰显示进程。
        let recorder = runtimes
            .mirror
            .start_recorder("phone", &SessionOptions::default(), &path, None)
            .unwrap();
        {
            let mut map = store.lock().unwrap();
            let state = session_entry_mut(&mut map, "phone");
            state.record_path = Some(path.to_string_lossy().into_owned());
            state.record_process = Some(recorder);
        }
        let rec_recorded = recorder_records.lock().unwrap().clone();
        assert_eq!(
            rec_recorded[0],
            path.to_string_lossy().as_ref(),
            "录制路径必须原样交给独立录制进程，不能由前端拼本机路径"
        );

        let recording = current_recording_with(&store).unwrap().unwrap();
        assert_eq!(recording.file_name, "MirrorDock-20260928-171825.mp4");
        assert!(recording.active, "录制进程还在跑，录像就还在写");

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn companion_recording_notification_only_counts_truly_recording_sessions() {
        // 空会话表：没有正在录制的会话。
        let store = SessionStore::default();
        assert!(!any_session_recording(&store));

        // 进程在跑但没开录制 ⇒ 不算「正在录制」。
        {
            let runtimes = runtimes(
                FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
                FakeMirror::running(),
            );
            let store = SessionStore::default();
            start_mirroring_with(
                &runtimes,
                &store,
                "phone".into(),
                SessionOptions::default(),
                None,
            )
            .unwrap();
            assert!(!any_session_recording(&store));
        }

        // 显示进程在跑 + 独立录制通道在录 ⇒ 正在录制（伴侣通知判定的唯一真值条件）。
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::running(),
        );
        let store = SessionStore::default();
        start_mirroring_with(
            &runtimes,
            &store,
            "phone".into(),
            SessionOptions::default(),
            None,
        )
        .unwrap();
        attach_test_recording(&store, "phone", &PathBuf::from("MirrorDock-recording.mp4"));
        assert!(any_session_recording(&store));

        // 录制通道被停止（用户点停止或录制进程退出）：record_process 清空、录制条目仍留。
        // 此刻录制已经结束，通知判定不得再把这条历史条目当成「正在录制」。
        {
            let mut map = store.0.lock().unwrap();
            let state = map.get_mut("phone").unwrap();
            state.record_process = None;
        }
        assert!(
            !any_session_recording(&store),
            "录制通道已停 ⇒ 录制已结束，残留录制条目不算「正在录制」"
        );
    }

    #[test]
    fn a_recording_file_name_can_never_escape_the_recording_directory() {
        for name in [
            "../escape.mp4",
            "sub/dir.mp4",
            "sub\\dir.mp4",
            ".hidden.mp4",
            "clip.mkv",
            "clip",
            "",
        ] {
            assert!(
                validate_media_name(name, "mp4").is_err(),
                "文件名 {name:?} 必须被拒绝"
            );
        }
        assert!(validate_media_name("MirrorDock-20260928-171825.mp4", "mp4").is_ok());
        // 同一个校验函数也不得放行别的扩展名。
        assert!(validate_media_name("clip.mp4", "png").is_err());
    }

    #[test]
    fn a_recording_still_being_written_cannot_be_deleted() {
        let directory = scratch_dir("recording-live");
        fs::create_dir_all(&directory).unwrap();
        let live = directory.join("MirrorDock-20260928-171825.mp4");
        fs::write(&live, b"partial").unwrap();

        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::running(),
        );
        let store = SessionStore::default();
        // X10-92：录制与显示解耦。先起显示会话，再经独立通道开录。
        start_mirroring_with(
            &runtimes,
            &store,
            "phone".into(),
            SessionOptions::default(),
            None,
        )
        .unwrap();
        attach_test_recording(&store, "phone", &live);

        let error =
            remove_recording_file(&directory, "MirrorDock-20260928-171825.mp4", &store).unwrap_err();
        assert_eq!(error.code, "recording_in_progress");
        assert!(live.exists(), "正在写的录像文件不得被删掉");

        // 结束录制（独立通道停止）后即可删除；再删一次必须如实说「文件已经不在了」。
        stop_recording_with(&store, None).unwrap();
        remove_recording_file(&directory, "MirrorDock-20260928-171825.mp4", &store).unwrap();
        assert!(!live.exists());
        assert_eq!(
            remove_recording_file(&directory, "MirrorDock-20260928-171825.mp4", &store)
                .unwrap_err()
                .code,
            "recording_missing"
        );

        let _ = fs::remove_dir_all(&directory);
    }

    /// 测试辅助：直接给某设备挂上一条「录制中」状态（独立录制进程 + 路径）。
    /// 模拟 `start_recording_with` 成功后的 SessionState，不经过真实 spawn。
    fn attach_test_recording(store: &SessionStore, serial: &str, path: &Path) {
        let mut map = store.lock().unwrap();
        let state = session_entry_mut(&mut map, serial);
        state.record_path = Some(path.to_string_lossy().into_owned());
        state.record_process = Some(Box::new(FakeProcess {
            exit: None,
            killed: Arc::new(Mutex::new(false)),
        }));
    }

    #[test]
    fn starting_recording_uses_a_separate_channel_and_never_restarts_the_display() {
        let directory = scratch_dir("recording-channel");
        let mirror = FakeMirror::running();
        let starts = Arc::clone(&mirror.starts);
        let recorder_starts = Arc::clone(&mirror.recorder_starts);
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            mirror,
        );
        let store = SessionStore::default();
        start_mirroring_with(
            &runtimes,
            &store,
            "phone".into(),
            SessionOptions::default(),
            None,
        )
        .unwrap();

        let path = directory.join("MirrorDock-20260928-180000.mp4");
        // 双通道：开录只新增一条独立录制进程，显示进程绝不重启。
        attach_test_recording(&store, "phone", &path);

        assert_eq!(
            starts.lock().unwrap().len(),
            1,
            "显示进程只启动一次，开录不得重启它"
        );
        assert!(
            current_recording_with(&store).unwrap().unwrap().active,
            "录制中状态由独立录制进程决定"
        );

        // 结束录制：显示进程仍在，录制通道被停掉。真实录制进程退出前会把 MP4 定型落盘，
        // 这里补一个 stub 文件模拟「产物已写出」，否则 X10-99 的产物核验会如实判失败。
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, b"finalized-mp4").unwrap();
        let stopped = stop_recording_with(&store, None).unwrap().unwrap();
        assert!(!stopped.active);
        assert_eq!(starts.lock().unwrap().len(), 1, "停录也不重启显示进程");
        assert!(!current_recording_with(&store).unwrap().unwrap().active);
        let _ = recorder_starts; // FakeMirror::start_recorder 的调用记录在集成路径断言

        let _ = fs::remove_dir_all(&directory);
    }

    // -- 文件传输：目录受限、状态可见、失败可恢复 --

    #[test]
    fn a_chosen_local_file_is_pushed_into_the_device_transfer_directory() {
        let local_dir = scratch_dir("transfer-push-src");
        fs::create_dir_all(&local_dir).unwrap();
        let contents = b"\xff\xd8\xff\xe0 fake jpeg bytes".to_vec();
        let local = local_dir.join("相册 导出.jpg");
        fs::write(&local, &contents).unwrap();

        let adb = FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]);
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        let receipt = send_file_to_device_with(&runtimes, "phone".into(), local.to_string_lossy().into_owned())
            .unwrap();

        assert_eq!(receipt.file_name, "相册 导出.jpg", "中文与空格的文件名必须原样保留");
        assert_eq!(receipt.path, "/sdcard/Download/MirrorDock/相册 导出.jpg");
        assert_eq!(receipt.bytes, contents.len() as u64);
        let calls = calls.lock().unwrap().clone();
        assert!(
            calls.contains(&format!("mkdir phone {DEVICE_TRANSFER_DIR}")),
            "发送前必须先确保接收目录存在，实际调用：{calls:?}"
        );
        assert!(calls
            .iter()
            .any(|call| call.starts_with(&format!("push phone {} {DEVICE_TRANSFER_DIR}", local.to_string_lossy()))),
            "推送必须直达传输目录，实际调用：{calls:?}");

        let _ = fs::remove_dir_all(&local_dir);
    }

    #[test]
    fn a_missing_or_non_file_local_path_is_rejected_before_touching_the_device() {
        let local_dir = scratch_dir("transfer-push-missing");
        fs::create_dir_all(&local_dir).unwrap();

        let adb = FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]);
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        let missing = send_file_to_device_with(
            &runtimes,
            "phone".into(),
            local_dir.join("不存在.zip").to_string_lossy().into_owned(),
        )
        .unwrap_err();
        assert_eq!(missing.code, "transfer_local_missing");

        let directory = send_file_to_device_with(
            &runtimes,
            "phone".into(),
            local_dir.to_string_lossy().into_owned(),
        )
        .unwrap_err();
        assert_eq!(directory.code, "transfer_local_missing", "目录不能当作文件发送");

        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|call| call.starts_with("push") || call.starts_with("mkdir")),
            "本地文件不存在时不得对设备发起任何写入"
        );

        let _ = fs::remove_dir_all(&local_dir);
    }

    #[test]
    fn an_unauthorized_device_is_refused_before_any_transfer_call() {
        let adb = FakeAdb::with_devices(vec![device("phone", DeviceState::Unauthorized)]);
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        let error = list_device_files_with(&runtimes, "phone".into()).unwrap_err();

        assert_eq!(error.code, "device_unauthorized");
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|call| call.starts_with("ls") || call.starts_with("mkdir")),
            "未授权设备不得发起目录读取"
        );
    }

    #[test]
    fn transfer_names_cannot_escape_the_device_directory() {
        for name in [
            "../escape.txt",
            "sub/dir.txt",
            "sub\\dir.txt",
            ".hidden",
            "line\nbreak",
            "tab\tname",
            "",
            "   ",
        ] {
            assert!(
                validate_transfer_name(name).is_err(),
                "“{name}”必须被拒绝"
            );
        }
        assert!(validate_transfer_name(&"a".repeat(MAX_TRANSFER_NAME_LEN + 1)).is_err());

        // 设备上的真实文件名常常包含中文等非 ASCII 字符：必须放行，不能用截图那套
        // ASCII 白名单。
        for name in ["照片.jpg", "导出-数据_1.csv", "a.txt"] {
            assert_eq!(
                validate_transfer_name(name).unwrap().as_str(),
                name,
                "“{name}”是合法的设备文件名"
            );
        }
    }

    #[test]
    fn device_files_are_listed_one_per_line_and_blanks_are_dropped() {
        let adb = FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            device_listing: Some("photo.jpg\r\nnotes.txt\n\n子目录 1.zip\n".to_owned()),
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        let names = list_device_files_with(&runtimes, "phone".into()).unwrap();

        assert_eq!(names, vec!["photo.jpg", "notes.txt", "子目录 1.zip"]);
        let calls = calls.lock().unwrap().clone();
        assert!(
            calls.contains(&format!("mkdir phone {DEVICE_TRANSFER_DIR}")),
            "列出前先确保目录存在，避免把「从未发送过文件」误报成故障"
        );
    }

    #[test]
    fn a_device_file_is_pulled_into_the_local_transfer_directory() {
        let directory = scratch_dir("transfer-pull");
        let adb = FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            pulled_contents: b"hello".to_vec(),
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        let receipt =
            fetch_file_from_device_into(&runtimes, &directory, "phone".into(), "导出.csv".into())
                .unwrap();

        assert_eq!(receipt.bytes, 5);
        assert!(receipt.path.ends_with("导出.csv"));
        assert_eq!(
            fs::read(directory.join("导出.csv")).unwrap(),
            b"hello",
            "取回的文件必须真实落盘"
        );
        let calls = calls.lock().unwrap().clone();
        assert!(calls
            .iter()
            .any(|call| call == &format!("pull phone {DEVICE_TRANSFER_DIR}/导出.csv {}",
                directory.join("导出.csv").to_string_lossy())),
            "拉取必须指向传输目录内的同名文件，实际调用：{calls:?}");

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_device_file_can_be_deleted_only_inside_the_transfer_directory() {
        // 就绪设备 + 合法文件名 → 发出 rm 调用且目标锁定在传输目录内（X10-60）。
        let adb = FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        delete_device_file_with(&runtimes, "phone".into(), "发送的 报表.csv".into()).unwrap();

        let calls = calls.lock().unwrap().clone();
        assert!(calls.contains(&format!(
            "rm phone {DEVICE_TRANSFER_DIR}/发送的 报表.csv"
        )), "删除必须指向传输目录内的同名文件，实际调用：{calls:?}");

        // 名单外的一律拒绝：逃逸路径在拼路径之前就被拦下。
        for hostile in ["../escape.txt", "sub/dir.txt", ".hidden", ""] {
            let adb = FakeAdb {
                devices: vec![device("phone", DeviceState::Ready)],
                ..FakeAdb::default()
            };
            let calls = Arc::clone(&adb.calls);
            let runtimes = screenshot_runtimes(adb);
            assert!(
                delete_device_file_with(&runtimes, "phone".into(), hostile.to_owned()).is_err(),
                "“{hostile}”必须被拒绝"
            );
            assert!(
                calls.lock().unwrap().iter().all(|call| !call.starts_with("rm")),
                "非法文件名不得发起任何 rm 调用"
            );
        }

        // 未授权设备：不发起删除。
        let adb = FakeAdb {
            devices: vec![device("phone", DeviceState::Unauthorized)],
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);
        assert!(delete_device_file_with(&runtimes, "phone".into(), "a.txt".into()).is_err());
        assert!(
            calls.lock().unwrap().iter().all(|call| !call.starts_with("rm")),
            "未授权设备不得发起删除"
        );
    }

    #[test]
    fn transfer_failures_are_reported_and_never_claim_success() {
        let directory = scratch_dir("transfer-fail");
        let local_dir = scratch_dir("transfer-fail-src");
        fs::create_dir_all(&local_dir).unwrap();
        let local = local_dir.join("ok.png");
        fs::write(&local, b"png").unwrap();

        let runtimes = screenshot_runtimes(FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            transfer_fails: true,
            ..FakeAdb::default()
        });

        assert_eq!(
            send_file_to_device_with(
                &runtimes,
                "phone".into(),
                local.to_string_lossy().into_owned()
            )
            .unwrap_err()
            .code,
            "transfer_push_failed"
        );
        assert_eq!(
            list_device_files_with(&runtimes, "phone".into()).unwrap_err().code,
            "transfer_list_failed"
        );
        let pull_error =
            fetch_file_from_device_into(&runtimes, &directory, "phone".into(), "x.csv".into())
                .unwrap_err();
        assert_eq!(pull_error.code, "transfer_pull_failed");
        assert!(
            !directory.join("x.csv").exists(),
            "拉取失败时不得留下空的半成品文件"
        );

        let _ = fs::remove_dir_all(&directory);
        let _ = fs::remove_dir_all(&local_dir);
    }

    // -- APK 安装 --

    #[test]
    fn an_apk_is_installed_on_the_ready_device() {
        let directory = scratch_dir("apk-install");
        fs::create_dir_all(&directory).unwrap();
        let apk = directory.join("MirrorDock-伴侣.apk");
        fs::write(&apk, b"apk-bytes").unwrap();

        let adb = FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            install_output: Some("Performing Streamed Install\nSuccess\n".to_owned()),
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        let receipt =
            install_apk_with(&runtimes, "phone".into(), apk.to_string_lossy().into_owned())
                .unwrap();

        assert_eq!(receipt.file_name, "MirrorDock-伴侣.apk");
        assert_eq!(receipt.bytes, 9);
        assert_eq!(receipt.summary, "安装完成。");
        let calls = calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![format!("install phone {}", apk.to_string_lossy())],
            "安装必须直接指向用户选中的这个文件，且只调用一次"
        );

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_non_apk_file_is_rejected_before_touching_the_device() {
        let directory = scratch_dir("apk-reject");
        fs::create_dir_all(&directory).unwrap();
        let not_apk = directory.join("notes.txt");
        fs::write(&not_apk, b"hello").unwrap();

        let adb = FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            install_output: Some("Success\n".to_owned()),
            ..FakeAdb::default()
        };
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        let error =
            install_apk_with(&runtimes, "phone".into(), not_apk.to_string_lossy().into_owned())
                .unwrap_err();

        assert_eq!(error.code, "apk_path_invalid");
        assert!(
            !calls.lock().unwrap().iter().any(|call| call.starts_with("install")),
            "后缀不对时不应向设备发起安装"
        );

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_or_relative_apk_path_is_rejected() {
        let runtimes = screenshot_runtimes(FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            install_output: Some("Success\n".to_owned()),
            ..FakeAdb::default()
        });

        for path in ["/definitely/not/here/app.apk", "app.apk", "", "   "] {
            assert_eq!(
                install_apk_with(&runtimes, "phone".into(), path.to_owned())
                    .unwrap_err()
                    .code,
                "apk_path_invalid",
                "“{path}”必须被拒绝：只接受已存在的绝对路径"
            );
        }
    }

    #[test]
    fn install_failure_keeps_the_specific_reason_for_the_user() {
        let directory = scratch_dir("apk-downgrade");
        fs::create_dir_all(&directory).unwrap();
        let apk = directory.join("old.apk");
        fs::write(&apk, b"apk").unwrap();

        let runtimes = screenshot_runtimes(FakeAdb {
            devices: vec![device("phone", DeviceState::Ready)],
            install_output: Some(
                "Performing Streamed Install\nFailure [INSTALL_FAILED_VERSION_DOWNGRADE]\n"
                    .to_owned(),
            ),
            ..FakeAdb::default()
        });

        let error =
            install_apk_with(&runtimes, "phone".into(), apk.to_string_lossy().into_owned())
                .unwrap_err();

        assert_eq!(error.code, "apk_install_failed");
        assert!(
            error.message.contains("版本") && error.message.contains("INSTALL_FAILED_VERSION_DOWNGRADE"),
            "失败原因必须具体到可照做，并保留原始错误码，实际：{}",
            error.message
        );

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn install_output_parsing_covers_success_and_unknown_reasons() {
        assert_eq!(
            describe_apk_install_output("Performing Streamed Install\nSuccess"),
            (true, "安装完成。".to_owned())
        );
        // 未知原因保留原始错误码，便于用户复述与检索，而不是塌缩成一句「安装失败」。
        let (ok, message) = describe_apk_install_output("Failure [INSTALL_FAILED_SOMETHING_NEW]");
        assert!(!ok);
        assert!(message.contains("INSTALL_FAILED_SOMETHING_NEW"));
        // 完全没有可解析原因时也给一句可执行的话，而不是空消息。
        let (ok, message) = describe_apk_install_output("error: device offline");
        assert!(!ok);
        assert!(message.contains("手机屏幕"));
    }

    #[test]
    fn installing_is_refused_when_the_device_is_not_ready() {
        let directory = scratch_dir("apk-unauthorized");
        fs::create_dir_all(&directory).unwrap();
        let apk = directory.join("app.apk");
        fs::write(&apk, b"apk").unwrap();

        for state in [DeviceState::Unauthorized, DeviceState::Offline] {
            let adb = FakeAdb {
                devices: vec![device("phone", state)],
                install_output: Some("Success\n".to_owned()),
                ..FakeAdb::default()
            };
            let calls = Arc::clone(&adb.calls);
            let runtimes = screenshot_runtimes(adb);

            let error =
                install_apk_with(&runtimes, "phone".into(), apk.to_string_lossy().into_owned())
                    .unwrap_err();

            assert!(error.code.starts_with("device_"), "实际：{}", error.code);
            assert!(
                !calls.lock().unwrap().iter().any(|call| call.starts_with("install")),
                "{state:?} 的设备不得发起安装"
            );
        }

        let _ = fs::remove_dir_all(&directory);
    }

    // -- 集成场景：把多个命令串成完整的用户旅程，验证跨命令的状态与授权一致性。 --
    // 单元测试各自验证一个行为；这里验证它们组合后仍然守同一条红线。

    #[test]
    fn a_full_usb_session_journey_from_unauthorized_to_a_graceful_stop() {
        let (adb, live) =
            FakeAdb::with_live_devices(vec![device("phone", DeviceState::Unauthorized)]);
        let mirror = FakeMirror::running();
        let killed = Arc::clone(&mirror.killed);
        let starts = Arc::clone(&mirror.starts);
        let runtimes = runtimes(adb, mirror);
        let store = SessionStore::default();

        // 第一步：手机还没授权。启动被拒且会话如实停在 unauthorized，不得塌缩成"连接失败"。
        let error = start_mirroring_with(
            &runtimes,
            &store,
            "phone".into(),
            SessionOptions::default(),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code, "device_unauthorized");
        assert_eq!(snapshot(&store).phase, SessionPhase::Unauthorized);

        // 第二步：用户在手机上点了"允许" → 设备就绪 → 再次尝试应当成功。
        *live.lock().unwrap() = vec![device("phone", DeviceState::Ready)];
        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default(), None)
            .unwrap();
        assert_eq!(snapshot(&store).phase, SessionPhase::Streaming);

        // 第三步：会话中应用新设置 → 旧窗口被结束、按新参数重开，会话不经过 Idle。
        let update = apply_session_options_with(
            &runtimes,
            &store,
            SessionOptions {
                rotation: 90,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();
        assert!(update.applied);
        assert_eq!(
            starts.lock().unwrap().len(),
            2,
            "应当恰好启动两次：原会话 + 按新设置重启"
        );
        assert!(*killed.lock().unwrap(), "旧镜像窗口必须被结束");
        assert_eq!(snapshot(&store).phase, SessionPhase::Streaming);

        // 第四步：停止 → 回到 idle；重复停止给出明确错误而不是假装成功。
        stop_mirroring_with(&store, Some("phone".into())).unwrap();
        assert_eq!(snapshot(&store).phase, SessionPhase::Idle);
        assert_eq!(
            stop_mirroring_with(&store, Some("phone".into())).unwrap_err().code,
            "session_not_running"
        );
    }

    #[test]
    fn screenshot_and_transfer_honor_the_same_authorization_gate_end_to_end() {
        let local_dir = scratch_dir("journey-transfer-src");
        fs::create_dir_all(&local_dir).unwrap();
        let local = local_dir.join("笔记.txt");
        fs::write(&local, b"hello mirrordock").unwrap();
        let directory = scratch_dir("journey-transfer-dst");

        let (mut adb, live) =
            FakeAdb::with_live_devices(vec![device("phone", DeviceState::Unauthorized)]);
        adb.screenshot = Some(TINY_PNG.to_vec());
        adb.device_listing = Some("相册 导出.jpg\n".into());
        let calls = Arc::clone(&adb.calls);
        let runtimes = screenshot_runtimes(adb);

        // 未授权：四条数据通道全部被拒，并且没有一条调用真正打到设备上。
        assert_eq!(
            capture_screenshot_into(&runtimes, &directory, "phone".into(), "s.png".into())
                .unwrap_err()
                .code,
            "device_unauthorized"
        );
        assert_eq!(
            send_file_to_device_with(
                &runtimes,
                "phone".into(),
                local.to_string_lossy().into_owned()
            )
            .unwrap_err()
            .code,
            "device_unauthorized"
        );
        assert_eq!(
            list_device_files_with(&runtimes, "phone".into())
                .unwrap_err()
                .code,
            "device_unauthorized"
        );
        assert_eq!(
            fetch_file_from_device_into(&runtimes, &directory, "phone".into(), "a.jpg".into())
                .unwrap_err()
                .code,
            "device_unauthorized"
        );
        assert!(
            calls.lock().unwrap().is_empty(),
            "未授权时任何 adb 调用都不应发起，实际：{:?}",
            calls.lock().unwrap()
        );

        // 授权后：同一组通道全部可用，回执与文件都真实存在。
        *live.lock().unwrap() = vec![device("phone", DeviceState::Ready)];
        let shot = capture_screenshot_into(&runtimes, &directory, "phone".into(), "s.png".into())
            .unwrap();
        assert_eq!(shot.bytes, TINY_PNG.len());
        assert!(directory.join("s.png").exists());
        let receipt = send_file_to_device_with(
            &runtimes,
            "phone".into(),
            local.to_string_lossy().into_owned(),
        )
        .unwrap();
        assert_eq!(receipt.file_name, "笔记.txt");
        assert_eq!(list_device_files_with(&runtimes, "phone".into()).unwrap(), {
            vec!["相册 导出.jpg".to_string()]
        });
        fetch_file_from_device_into(&runtimes, &directory, "phone".into(), "a.jpg".into())
            .unwrap();
        assert!(directory.join("a.jpg").exists(), "拉取的文件必须真实落盘");

        let _ = fs::remove_dir_all(&directory);
        let _ = fs::remove_dir_all(&local_dir);
    }

    #[test]
    fn a_wireless_endpoint_session_follows_the_same_lifecycle_contract() {
        // 无线端点只是 serial 的另一种形态；生命周期契约（启动/重启/停止、状态不塌缩）
        // 必须与 USB 完全一致，否则前端要为两种连接方式维护两套心智模型。
        let serial = "adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp";
        let mirror = FakeMirror::running();
        let starts = Arc::clone(&mirror.starts);
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device(serial, DeviceState::Ready)]),
            mirror,
        );
        let store = SessionStore::default();

        start_mirroring_with(&runtimes, &store, serial.into(), SessionOptions::default(), None)
            .unwrap();
        assert_eq!(snapshot(&store).phase, SessionPhase::Streaming);

        apply_session_options_with(
            &runtimes,
            &store,
            SessionOptions {
                quality: Quality::Sharp,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();
        assert_eq!(starts.lock().unwrap().len(), 2);

        stop_mirroring_with(&store, Some(serial.into())).unwrap();
        let ended = snapshot(&store);
        assert_eq!(ended.phase, SessionPhase::Idle);
        assert_eq!(
            ended.serial, None,
            "停止后不得残留上一个设备的序列号"
        );
    }

    // -- 诊断包：显式同意、可预览、默认脱敏 --

    fn diagnostic_log_with_sample() -> DiagnosticsLog {
        let log = DiagnosticsLog::default();
        log.record(
            "mirror_start",
            "device_unauthorized",
            "手机未授权这台电脑调试（序列号 79j7kn9tkjt8rwss，端点 192.168.1.8:39085）。请在手机上允许 USB 调试。",
            &["79j7kn9tkjt8rwss", "192.168.1.8:39085"],
        );
        log
    }

    #[test]
    fn recorded_events_never_contain_declared_secrets() {
        let log = diagnostic_log_with_sample();
        let events = log.snapshot();

        assert_eq!(events.len(), 1);
        let detail = &events[0].detail;
        assert!(
            !detail.contains("79j7kn9tkjt8rwss") && !detail.contains("192.168.1.8:39085"),
            "声明的机密必须被擦除：{detail}"
        );
        assert!(
            detail.contains("[已脱敏]"),
            "擦除位置要有可见占位，便于确认脱敏发生了"
        );
        // 用户可见文案本身保留——没有它支持人员无法定位问题。
        assert!(detail.contains("允许 USB 调试"));
    }

    #[test]
    fn diagnostic_events_are_capped_and_keep_the_newest() {
        let log = DiagnosticsLog::default();
        for index in 0..(DIAGNOSTICS_CAP + 5) {
            log.record("probe", "ok", &format!("事件 {index}"), &[]);
        }

        let events = log.snapshot();
        assert_eq!(events.len(), DIAGNOSTICS_CAP);
        assert_eq!(events.last().unwrap().detail, "事件 204");
        assert_eq!(events.first().unwrap().detail, "事件 5");
    }

    #[test]
    fn outcome_recording_uses_the_user_facing_error_only() {
        let log = DiagnosticsLog::default();
        let error = AppError::new(
            "mirror_start_failed",
            "无法启动镜像窗口。",
            "请重新检查连接后再试。",
        );
        log.record_outcome("mirror_start", Some(&error), &[]);
        log.record_outcome("mirror_stop", None, &[]);

        let events = log.snapshot();
        assert_eq!(events[0].code, "mirror_start_failed");
        assert!(events[0].detail.contains("无法启动镜像窗口"));
        assert_eq!(events[1].code, "ok");
    }

    #[test]
    fn preview_contains_only_environment_facts_and_redacted_events() {
        let log = diagnostic_log_with_sample();
        let preview = build_diagnostics_preview(
            "0.1.0".to_owned(),
            "macos / x86_64".to_owned(),
            true,
            log.snapshot(),
        );

        assert_eq!(preview.app_version, "0.1.0");
        assert!(preview.scrcpy_available);
        assert_eq!(preview.events.len(), 1);
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(
            !serialized.contains("79j7kn9tkjt8rwss"),
            "序列化后的预览也不得包含机密"
        );
    }

    #[test]
    fn an_exported_diagnostics_file_is_json_and_matches_the_receipt() {
        let directory = scratch_dir("diagnostics-export");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("diag.json");
        let log = diagnostic_log_with_sample();
        let preview = build_diagnostics_preview(
            "0.1.0-test".to_owned(),
            "macos / x86_64".to_owned(),
            false,
            log.snapshot(),
        );

        let receipt = export_diagnostics_into(path.to_str().unwrap(), &preview).unwrap();
        assert_eq!(receipt.events, 1);

        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"app_version\": \"0.1.0-test\""));
        assert!(written.contains("[已脱敏]"));
        assert_eq!(receipt.bytes, written.len() as u64);

        let _ = fs::remove_dir_all(&directory);
    }

    // ---- R3-01 授权（licensing 模块） ----

    /// 测试专用密钥对：与生产种子完全独立，只用于签名行为的单元验证。
    fn test_signing_key() -> ed25519_dalek::SigningKey {
        let mut seed = [0u8; 32];
        for (i, byte) in seed.iter_mut().enumerate() {
            *byte = (i as u8) * 7 + 3;
        }
        ed25519_dalek::SigningKey::from_bytes(&seed)
    }

    fn test_verifying_key() -> ed25519_dalek::VerifyingKey {
        test_signing_key().verifying_key()
    }

    fn test_license(key_id: &str, expires_at: Option<u64>) -> String {
        let signing = test_signing_key();
        let payload = licensing::LicensePayload {
            product: "mirrordock".to_owned(),
            key_id: key_id.to_owned(),
            edition: "pro".to_owned(),
            expires_at,
        };
        licensing::encode_license(&payload, &signing)
    }

    #[test]
    fn base32_roundtrip_across_lengths() {
        for len in [0usize, 1, 4, 5, 6, 63, 64, 100] {
            let data: Vec<u8> = (0..len as u8).map(|b| b.wrapping_mul(31).wrapping_add(5)).collect();
            let encoded = licensing::base32_encode(&data);
            let decoded = licensing::base32_decode(&encoded).unwrap();
            assert_eq!(decoded, data, "base32 往返失败（长度 {len}）");
        }
        // 无填充 canonical base32：5 字节 = 8 字符。
        assert_eq!(licensing::base32_encode(&[0xff; 5]).len(), 8);
        assert_eq!(licensing::base32_encode(&[0xff; 1]).len(), 2);
    }

    #[test]
    fn embedded_verifying_key_matches_rfc8032_conventions() {
        // RFC 8032 TEST 1 公钥：隔离「密钥字节损坏」与「dalek 用法错误」。
        let rfc8032_pub: [u8; 32] = [
            0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
            0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68,
            0xf7, 0x07, 0x51, 0x1a,
        ];
        assert!(ed25519_dalek::VerifyingKey::from_bytes(&rfc8032_pub).is_ok());
        assert!(ed25519_dalek::VerifyingKey::from_bytes(&LICENSE_VERIFYING_KEY).is_ok());
    }

    #[test]
    fn license_roundtrip_and_grouping() {
        let now = 1_700_000_000u64;
        let license = test_license("test-001", Some(now + 365 * 86_400));
        assert!(license.starts_with("MD1-"));
        assert!(license.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        let payload = licensing::verify_license(&license, &test_verifying_key(), now).unwrap();
        assert_eq!(payload.key_id, "test-001");
        assert_eq!(payload.edition, "pro");
        assert_eq!(payload.product, "mirrordock");
        // 大小写与空格/分隔符容错。
        let spaced = license.to_lowercase().replace('-', " ");
        assert!(licensing::verify_license(&spaced, &test_verifying_key(), now).is_ok());
    }

    /// Beta 全量内置测试码必须能通过编译进二进制的公钥验签：
    /// 防止将来换钥或换码时只改其一导致所有安装静默失去解锁。
    #[test]
    fn embedded_beta_license_verifies_with_embedded_key() {
        let verifying = ed25519_dalek::VerifyingKey::from_bytes(&LICENSE_VERIFYING_KEY).unwrap();
        let payload = licensing::verify_license(BETA_LICENSE_KEY, &verifying, unix_now())
            .expect("内置 Beta 许可证必须有效");
        assert_eq!(payload.key_id, "beta");
        assert_eq!(payload.edition, "pro");
        assert_eq!(payload.product, "mirrordock");
        assert_eq!(payload.expires_at, None);
    }

    #[test]
    fn license_rejects_tampered_payload() {
        let now = 1_700_000_000u64;
        let mut license = test_license("test-001", Some(now + 365 * 86_400));
        let bytes: Vec<char> = license.chars().collect();
        let idx = bytes.iter().position(|c| c.is_ascii_alphabetic()).unwrap();
        license = bytes
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if i == idx {
                    if *c == 'A' { 'B' } else { 'A' }
                } else {
                    *c
                }
            })
            .collect();
        let err = licensing::verify_license(&license, &test_verifying_key(), now).unwrap_err();
        assert!(
            matches!(err, licensing::LicenseError::BadSignature | licensing::LicenseError::Malformed),
            "篡改应被拒绝，实际 {err:?}"
        );
    }

    #[test]
    fn license_rejects_expired_and_foreign_product() {
        let now = 1_700_000_000u64;
        let expired = test_license("test-001", Some(now - 1));
        assert_eq!(
            licensing::verify_license(&expired, &test_verifying_key(), now),
            Err(licensing::LicenseError::Expired)
        );
        let signing = test_signing_key();
        let foreign = licensing::encode_license(
            &licensing::LicensePayload {
                product: "other-app".to_owned(),
                key_id: "x".to_owned(),
                edition: "pro".to_owned(),
                expires_at: None,
            },
            &signing,
        );
        assert_eq!(
            licensing::verify_license(&foreign, &test_verifying_key(), now),
            Err(licensing::LicenseError::WrongProduct)
        );
        assert_eq!(
            licensing::verify_license("XX1-AAAA", &test_verifying_key(), now),
            Err(licensing::LicenseError::Malformed)
        );
    }

    #[test]
    fn current_edition_falls_back_to_free_on_missing_or_corrupt_store() {
        let directory = scratch_dir("entitlement-fallback");
        assert_eq!(licensing::current_edition(&directory, 0), Edition::Free);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(licensing::ENTITLEMENT_FILE), "not json").unwrap();
        assert_eq!(licensing::current_edition(&directory, 0), Edition::Free);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn pro_gate_blocks_recording_on_free() {
        let mut options = SessionOptions::default();
        // 默认会话（audio 开、录制关）对免费版必须开箱即用。
        assert!(licensing::ensure_pro_features(Edition::Free, &options).is_ok());

        options.record = true;
        let err = licensing::ensure_pro_features(Edition::Free, &options).unwrap_err();
        assert_eq!(err.code, "pro_required");

        options.record = true;
        options.audio = false;
        assert!(licensing::ensure_pro_features(Edition::Pro, &options).is_ok());
    }

    #[test]
    fn activation_rejects_foreign_signed_license_before_writing_store() {
        let directory = scratch_dir("entitlement-activate");
        let now = 1_700_000_000u64;
        let license = test_license("test-001", Some(now + 365 * 86_400));
        // activate_into 内部使用编译进二进制的公钥，测试密钥签的许可应验签失败且不落盘。
        let err = licensing::activate_into(&directory, &license, now).unwrap_err();
        assert_eq!(err, licensing::LicenseError::BadSignature);
        assert!(!directory.join(licensing::ENTITLEMENT_FILE).exists());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn diagnostics_never_contain_license_key() {
        let log = DiagnosticsLog::default();
        let raw_key = "MD1-ABCDEF-GHIJKL-MNOPQR-STUVWX-YZ2345-6789AB";
        let error = AppError::new(
            "license_invalid",
            "许可证签名无效。",
            "请确认许可证来自官方渠道。",
        );
        log.record_outcome("license_activate", Some(&error), &[raw_key]);
        for event in log.snapshot() {
            assert!(!event.detail.contains(raw_key), "诊断不得回显许可证原文");
        }
    }

    /// 真实密钥端到端：仅在本地设置了 MIRRORDOCK_LICENSE_SEED 时运行
    /// （CI 与无种子环境自动跳过），验证「example 签发 → activate_into → Pro」全链路。
    #[test]
    fn real_key_end_to_end_when_seed_present() {
        let Ok(seed_hex) = std::env::var("MIRRORDOCK_LICENSE_SEED") else {
            eprintln!("跳过：未设置 MIRRORDOCK_LICENSE_SEED");
            return;
        };
        let mut seed = [0u8; 32];
        for (i, byte) in seed.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&seed_hex[i * 2..i * 2 + 2], 16).unwrap();
        }
        let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
        let now = 1_700_000_000u64;
        let license = licensing::encode_license(
            &licensing::LicensePayload {
                product: "mirrordock".to_owned(),
                key_id: "e2e".to_owned(),
                edition: "pro".to_owned(),
                expires_at: Some(now + 86_400),
            },
            &signing,
        );
        let directory = scratch_dir("entitlement-e2e");
        licensing::activate_into(&directory, &license, now).unwrap();
        assert_eq!(licensing::current_edition(&directory, now), Edition::Pro);
        let _ = fs::remove_dir_all(&directory);
    }
}
