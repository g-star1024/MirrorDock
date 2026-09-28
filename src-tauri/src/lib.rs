use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

/// 会话状态机轮询运行中进程的间隔。
const MONITOR_INTERVAL: Duration = Duration::from_millis(200);

/// 优雅结束镜像进程时等待 scrcpy 收尾（写出录像 moov 索引）的上限。
const PROCESS_GRACEFUL_TIMEOUT: Duration = Duration::from_millis(3000);

/// 兜底强杀之后等待进程真正退出的上限。SIGKILL 之后的等待只为回收，通常毫秒级。
const PROCESS_REAP_TIMEOUT: Duration = Duration::from_millis(1000);

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AdbDevice {
    serial: String,
    label: String,
    state: DeviceState,
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
///   transfer_push_failed / transfer_pull_failed / transfer_list_failed
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
fn resolve_process_exit(serial: &str, success: bool) -> MirrorSession {
    if success {
        MirrorSession::idle()
    } else {
        MirrorSession::failed(
            Some(serial.to_owned()),
            AppError::new(
                "mirror_exited",
                "镜像窗口已意外关闭。",
                "请检查手机授权与连接后重新启动镜像。",
            ),
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
    /// 点亮设备屏幕（`KEYCODE_WAKEUP`）。
    ///
    /// 只唤醒屏幕：不输入任何凭据、不解锁、不解除钥匙锁。锁屏本身不在可绕过范围内。
    fn wake_screen(&self, serial: &str) -> Result<(), std::io::Error>;
    /// 读取 `dumpsys window policy` 原始输出，用于判断钥匙锁状态。
    fn window_policy(&self, serial: &str) -> Result<String, std::io::Error>;
    /// 读取 `dumpsys power` 原始输出，用于判断屏幕是否点亮。
    fn power_state(&self, serial: &str) -> Result<String, std::io::Error>;
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
    fn pair(&self, endpoint: &str, pairing_code: &str) -> Result<(), std::io::Error>;
    fn connect(&self, endpoint: &str) -> Result<(), std::io::Error>;
    fn disconnect(&self, endpoint: &str) -> Result<(), std::io::Error>;
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
}

struct AppRuntimes {
    adb: Box<dyn AdbRuntime>,
    mirror: Box<dyn MirrorRuntime>,
}

impl AppRuntimes {
    fn system() -> Self {
        Self {
            adb: Box::new(SystemAdbRuntime),
            mirror: Box::new(ScrcpyRuntime),
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
        let (size, bitrate) = match self.quality {
            Quality::Smooth => (1024, "2M"),
            Quality::Balanced => (1920, "8M"),
            Quality::Sharp => (2560, "16M"),
        };
        let mut args = vec![
            format!("--max-size={size}"),
            format!("--video-bit-rate={bitrate}"),
            "--video-codec=h264".into(),
            format!("--display-orientation={}", self.rotation),
        ];
        if self.keep_awake {
            args.push("--stay-awake".into());
        }
        if !self.clipboard_autosync {
            args.push("--no-clipboard-autosync".into());
        }
        if !self.audio {
            args.push("--no-audio".into());
        }
        if self.fullscreen {
            args.push("--fullscreen".into());
        }
        if self.always_on_top {
            args.push("--always-on-top".into());
        }
        Ok(args)
    }
}

struct SystemAdbRuntime;

impl SystemAdbRuntime {
    /// 固定参数直接调用 `adb`，不使用 shell，也不做字符串拼接。
    fn run(args: &[&str]) -> Result<(), std::io::Error> {
        let output = Command::new(adb_binary()).args(args).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    /// 读取 `adb` 的 stdout。同样使用固定参数直接调用，不做任何 shell 拼接或插值。
    fn capture(args: &[&str]) -> Result<String, std::io::Error> {
        let output = Command::new(adb_binary()).args(args).output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }
}

impl AdbRuntime for SystemAdbRuntime {
    fn list_devices(&self) -> Result<Vec<AdbDevice>, std::io::Error> {
        let output = Command::new(adb_binary()).args(["devices", "-l"]).output()?;
        if output.status.success() {
            Ok(parse_adb_devices(&String::from_utf8_lossy(&output.stdout)))
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    fn device_properties(&self, serial: &str) -> Result<String, std::io::Error> {
        // 固定参数直接调用：serial 作为单个 argv 传入，不做任何 shell 拼接或插值。
        let output = Command::new(adb_binary())
            .args(["-s", serial, "shell", "getprop"])
            .output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }

    fn wake_screen(&self, serial: &str) -> Result<(), std::io::Error> {
        // 只发送唤醒键。这里刻意不发送任何解锁相关的输入。
        Self::run(&["-s", serial, "shell", "input", "keyevent", "KEYCODE_WAKEUP"])
    }

    fn window_policy(&self, serial: &str) -> Result<String, std::io::Error> {
        Self::capture(&["-s", serial, "shell", "dumpsys", "window", "policy"])
    }

    fn power_state(&self, serial: &str) -> Result<String, std::io::Error> {
        Self::capture(&["-s", serial, "shell", "dumpsys", "power"])
    }

    fn screenshot_png(&self, serial: &str) -> Result<Vec<u8>, std::io::Error> {
        // 用 `exec-out` 而不是 `shell`：后者会把 stdout 当作文本流，在 Windows 上
        // 可能把 \n 改写成 \r\n，从而破坏 PNG 二进制。
        // 固定参数直接调用：serial 作为单个 argv 传入，不做任何 shell 拼接或插值。
        let output = Command::new(adb_binary())
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
        let output = Command::new(adb_binary())
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
        let output = Command::new(adb_binary())
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

    fn pair(&self, endpoint: &str, pairing_code: &str) -> Result<(), std::io::Error> {
        Self::run(&["pair", endpoint, pairing_code])
    }

    fn connect(&self, endpoint: &str) -> Result<(), std::io::Error> {
        Self::run(&["connect", endpoint])
    }

    fn disconnect(&self, endpoint: &str) -> Result<(), std::io::Error> {
        Self::run(&["disconnect", endpoint])
    }
}

/// 真实的镜像进程。`Drop` 时确保子进程被终止并回收，避免应用退出后残留 scrcpy。
struct SystemMirrorProcess {
    child: Child,
}

impl MirrorProcess for SystemMirrorProcess {
    fn try_wait(&mut self) -> Option<bool> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.success()),
            _ => None,
        }
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
            // SAFETY: kill 只向本子进程的 pid 发送 SIGTERM，不触碰其它进程。
            let sent = unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
            if sent != 0 {
                // 发送失败最常见的原因是进程恰好自行退出；能 reap 就视为已结束。
                if self.try_wait().is_some() {
                    return Ok(());
                }
                return Err(std::io::Error::last_os_error());
            }
            let deadline = std::time::Instant::now() + PROCESS_GRACEFUL_TIMEOUT;
            while std::time::Instant::now() < deadline {
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
            // Windows 没有 SIGTERM 对应物：TerminateProcess 立即结束进程，
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
        let mut command = Command::new(scrcpy_binary());
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
        let child = command.spawn()?;
        Ok(Box::new(SystemMirrorProcess { child }))
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
}

struct SessionStore(Arc<Mutex<SessionState>>);

impl Default for SessionStore {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(SessionState {
            session: MirrorSession::idle(),
            process: None,
            epoch: 0,
            options: SessionOptions::default(),
            record_path: None,
        })))
    }
}

impl SessionStore {
    fn lock(&self) -> Result<MutexGuard<'_, SessionState>, AppError> {
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

/// 占位一个新的会话。已有会话运行或启动中时拒绝覆盖，即使序列号相同。
fn reserve_session(store: &SessionStore, session: MirrorSession) -> Result<(), AppError> {
    let mut state = store.lock()?;
    if is_session_active(&state) {
        return Err(AppError::new(
            "session_busy",
            "已有镜像窗口正在运行或启动。",
            "请先结束当前会话，再启动新的会话。",
        ));
    }
    state.epoch = state.epoch.wrapping_add(1);
    state.session = session;
    Ok(())
}

/// 直接改写会话状态（会终止对当前进程的跟踪，由 `Drop` 负责回收）。
fn mark_session(store: &SessionStore, session: MirrorSession) {
    if let Ok(mut state) = store.0.lock() {
        state.epoch = state.epoch.wrapping_add(1);
        state.process = None;
        state.session = session;
    }
}

fn fail_session(store: &SessionStore, serial: Option<String>, error: AppError) -> AppError {
    mark_session(store, MirrorSession::failed(serial, error.clone()));
    error
}

/// 把进程接入会话，进入 `Streaming`，并返回本次会话的 epoch。
fn attach_process(
    store: &SessionStore,
    process: Box<dyn MirrorProcess>,
    serial: String,
    options: SessionOptions,
    record_path: Option<String>,
) -> Result<u64, AppError> {
    let mut state = store.lock()?;
    state.epoch = state.epoch.wrapping_add(1);
    state.session = MirrorSession::streaming(serial);
    state.process = Some(process);
    state.options = options;
    state.record_path = record_path;
    Ok(state.epoch)
}

/// 结束当前会话并交还进程句柄；没有运行中的会话时返回 `None`。
fn take_running_process(store: &SessionStore) -> Result<Option<Box<dyn MirrorProcess>>, AppError> {
    let mut state = store.lock()?;
    let process = state.process.take();
    if process.is_some() {
        state.epoch = state.epoch.wrapping_add(1);
        state.session = MirrorSession::idle();
    }
    Ok(process)
}

/// 为「会话中应用新设置」原子地交出运行中的进程，并把会话就地标记为 `Connecting`。
///
/// 关键点：**不能先回到 `Idle` 再重新启动**。那会让界面在两次轮询之间读到“没有会话”，
/// 用户可能误以为镜像已经结束（而且“结束镜像”按钮会闪一下）。会话在整个重启过程中都
/// 应当停在「正在启动」。没有运行中的进程时返回 `None`，会话状态不作改动。
fn begin_session_restart(
    store: &SessionStore,
    serial: String,
) -> Result<Option<Box<dyn MirrorProcess>>, AppError> {
    let mut state = store.lock()?;
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

/// 轮询运行中的进程；进程退出后把结果写回会话状态。
fn spawn_session_monitor(store: SessionStore, epoch: u64, serial: String) {
    std::thread::spawn(move || loop {
        std::thread::sleep(MONITOR_INTERVAL);
        let finished = {
            let Ok(mut state) = store.0.lock() else {
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
                    state.process = None;
                    state.session = resolve_process_exit(&serial, success);
                    true
                }
                None => false,
            }
        };
        if finished {
            return;
        }
    });
}

/// 只有空闲时才把“已配对”写进会话，避免覆盖正在运行的镜像会话。
fn mark_paired_if_idle(store: &SessionStore, endpoint: String) {
    if let Ok(mut state) = store.0.lock() {
        if !is_session_active(&state) {
            state.epoch = state.epoch.wrapping_add(1);
            state.session = MirrorSession::paired(endpoint);
        }
    }
}

/// 忘记某个端点时，若当前会话正停留在该端点的“已配对”状态，则回到空闲。
fn clear_paired_if_matches(store: &SessionStore, endpoint: &str) {
    if let Ok(mut state) = store.0.lock() {
        if state.session.phase == SessionPhase::Paired
            && state.session.serial.as_deref() == Some(endpoint)
        {
            state.epoch = state.epoch.wrapping_add(1);
            state.session = MirrorSession::idle();
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
                    "点击「唤醒屏幕」，手机亮起后即可直接在镜像窗口中操作。".to_owned(),
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

fn current_recording_with(store: &SessionStore) -> Result<Option<Recording>, AppError> {
    let state = store.lock()?;
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
        active: state.process.is_some(),
    }))
}

/// 删除一个录像文件，对应界面上的「撤销」。
///
/// **正在录制的文件会被拒绝删除**：删掉它既会让用户以为已经清理干净、实际却还在写，
/// 也可能破坏正在进行中的文件。这里如实报错并指出下一步，而不是静默失败。
fn remove_recording_file(
    directory: &Path,
    name: &str,
    store: &SessionStore,
) -> Result<(), AppError> {
    let path = directory.join(name);
    {
        let state = store.lock()?;
        if state.process.is_some()
            && state.record_path.as_deref() == Some(path.to_string_lossy().as_ref())
        {
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
    Command::new(scrcpy_binary())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
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
            Some(AdbDevice {
                serial: serial.to_owned(),
                label: model.unwrap_or_else(|| "Android 设备".to_owned()),
                state,
            })
        })
        .collect()
}

fn endpoint_is_ready(devices: &[AdbDevice], endpoint: &str) -> bool {
    devices
        .iter()
        .any(|device| device.serial == endpoint && device.state == DeviceState::Ready)
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
fn check_adb_devices(runtimes: State<AppRuntimes>) -> AdbCheck {
    let scrcpy_available = runtimes.mirror.is_available();
    match runtimes.adb.list_devices() {
        Ok(devices) => AdbCheck {
            adb_available: true,
            scrcpy_available,
            devices,
            diagnostic: None,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AdbCheck {
            adb_available: false,
            scrcpy_available,
            devices: Vec::new(),
            diagnostic: Some("未找到 Android 平台工具。请重新安装 MirrorDock 或联系支持人员。".into()),
        },
        Err(_) => AdbCheck {
            adb_available: true,
            scrcpy_available,
            devices: Vec::new(),
            diagnostic: Some(
                "Android 调试服务暂时不可用。请拔下数据线后重新连接，再试一次。".to_owned(),
            ),
        },
    }
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
    serial: String,
    options: Option<SessionOptions>,
    record_file_name: Option<String>,
) -> Result<(), AppError> {
    let options = options.unwrap_or_default();
    // 先准备录制路径：目录不可写或文件名非法时，在占用会话槽位之前就失败。
    let record_path = prepare_recording_path(&app, options.record, record_file_name.as_deref())?;
    start_mirroring_with(&runtimes, &sessions, serial.clone(), options, record_path)?;

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

    let epoch = attach_process(
        sessions,
        process,
        serial.clone(),
        options,
        record_path.map(|path| path.to_string_lossy().into_owned()),
    )?;
    spawn_session_monitor(sessions.clone(), epoch, serial);
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

/// 读取正在运行的会话所使用的设备与启动参数。
///
/// 只有真正持有运行中的进程时才算“会话进行中”：处于 `Connecting` 但尚未拿到进程、
/// 以及 `Paired` / `Failed` 等阶段都会返回可恢复的错误，而不是去 kill 一个不存在的进程。
fn running_session_options(store: &SessionStore) -> Result<(String, SessionOptions), AppError> {
    let state = store.lock()?;
    if state.process.is_none() {
        return Err(session_not_running_error());
    }
    let Some(serial) = state.session.serial.clone() else {
        return Err(session_not_running_error());
    };
    Ok((serial, state.options.clone()))
}

fn session_snapshot(store: &SessionStore) -> Result<MirrorSession, AppError> {
    Ok(store.lock()?.session.clone())
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
fn annotate_session_error(store: &SessionStore, error: &AppError) {
    if let Ok(mut state) = store.0.lock() {
        if state.session.error.is_some() {
            state.session.error = Some(error.clone());
        }
    }
}

/// 会话进行中应用新设置：先结束旧窗口，再按新设置重新打开。
fn apply_session_options_with(
    runtimes: &AppRuntimes,
    sessions: &SessionStore,
    options: SessionOptions,
    record_path: Option<PathBuf>,
) -> Result<SessionUpdate, AppError> {
    // 先校验参数，再触碰正在运行的会话：一个非法请求绝不能打断一次正常的镜像。
    options.arguments()?;

    let (serial, current) = running_session_options(sessions)?;
    if current == options {
        return Ok(SessionUpdate {
            applied: false,
            note: Some("设置与当前会话一致，无需重启镜像窗口。".to_owned()),
            session: session_snapshot(sessions)?,
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
        launch_into_reserved_session(runtimes, sessions, serial, options, record_path)
    {
        let error = with_restart_context(&error);
        annotate_session_error(sessions, &error);
        return Err(error);
    }

    Ok(SessionUpdate {
        applied: true,
        note: None,
        session: session_snapshot(sessions)?,
    })
}

#[tauri::command]
fn stop_mirroring(sessions: State<SessionStore>) -> Result<(), AppError> {
    stop_mirroring_with(&sessions)
}

fn stop_mirroring_with(store: &SessionStore) -> Result<(), AppError> {
    match take_running_process(store)? {
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

#[tauri::command]
fn mirror_session(sessions: State<SessionStore>) -> Result<MirrorSession, AppError> {
    sessions.lock().map(|state| state.session.clone())
}

/// 会话进行中应用新的窗口设置。
///
/// 语义是明确的「结束旧窗口 + 按新设置重新打开」：镜像画面会短暂中断，界面必须如实
/// 告知用户，不能假装设置已经热更新。设置与当前会话一致时不做任何动作，避免无谓地
/// 打断一次正常的镜像。
#[tauri::command]
fn update_session_options(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    options: SessionOptions,
    record_file_name: Option<String>,
) -> Result<SessionUpdate, AppError> {
    // 同样是「先校验、再触碰运行中的会话」：路径不可用时不打断正在进行的镜像。
    let record_path = prepare_recording_path(&app, options.record, record_file_name.as_deref())?;
    apply_session_options_with(&runtimes, &sessions, options, record_path)
}

/// 读取最近一次会话的录制信息（若有）。`active` 表示此刻进程是否仍在写这个文件。
#[tauri::command]
fn current_recording(sessions: State<SessionStore>) -> Result<Option<Recording>, AppError> {
    current_recording_with(&sessions)
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

#[tauri::command]
fn pair_wireless_device(
    runtimes: State<AppRuntimes>,
    endpoint: String,
    pairing_code: String,
) -> Result<(), AppError> {
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
}

#[tauri::command]
fn connect_wireless_device(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    endpoint: String,
) -> Result<(), AppError> {
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

#[tauri::command]
fn wake_device(runtimes: State<AppRuntimes>, serial: String) -> Result<(), AppError> {
    let serial = validate_serial(&serial)?;
    if let Some(error) = device_readiness_error(device_lookup(&runtimes, &serial)) {
        return Err(error);
    }
    runtimes.adb.wake_screen(&serial).map_err(|error| {
        adb_command_error(
            error,
            "wake_failed",
            "无法点亮手机屏幕。",
            "请确认数据线或无线连接仍然有效；也可以直接按一下手机的电源键。",
        )
    })
}

#[tauri::command]
fn device_lock_report(
    runtimes: State<AppRuntimes>,
    serial: String,
) -> Result<DeviceLockReport, AppError> {
    lock_report_with(&runtimes, serial)
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

/// 把手机当前画面保存为本机的一张 PNG。
///
/// 文件名由前端按**本地时间**生成（后端不猜时区），随后按不可信输入严格校验。
/// 截图内容是屏幕像素：只写入用户可见的本地文件，不写日志、不进错误消息。
#[tauri::command]
fn capture_screenshot(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    serial: String,
    file_name: String,
) -> Result<Screenshot, AppError> {
    let directory = screenshot_dir(&app)?;
    capture_screenshot_into(&runtimes, &directory, serial, file_name)
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
    serial: String,
    local_path: String,
) -> Result<TransferReceipt, AppError> {
    send_file_to_device_with(&runtimes, serial, local_path)
}

/// 列出手机传输目录里的文件名，供用户挑选要取回的文件。
#[tauri::command]
fn list_device_files(
    runtimes: State<AppRuntimes>,
    serial: String,
) -> Result<Vec<String>, AppError> {
    list_device_files_with(&runtimes, serial)
}

/// 从手机取回一个文件，保存到本机「下载 / MirrorDock」。
#[tauri::command]
fn fetch_file_from_device(
    app: AppHandle,
    runtimes: State<AppRuntimes>,
    serial: String,
    file_name: String,
) -> Result<TransferReceipt, AppError> {
    let directory = transfer_download_dir(&app)?;
    fetch_file_from_device_into(&runtimes, &directory, serial, file_name)
}

/// 应用退出时回收子进程，避免残留 scrcpy 进程。优雅结束让录制文件有机会收尾。
fn reclaim_children(app: &AppHandle) {
    if let Some(store) = app.try_state::<SessionStore>() {
        if let Ok(Some(mut process)) = take_running_process(&store) {
            let _ = process.stop();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(SessionStore::default())
        .manage(AppRuntimes::system())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            check_adb_devices,
            probe_device_capabilities,
            start_mirroring,
            stop_mirroring,
            mirror_session,
            update_session_options,
            wake_device,
            device_lock_report,
            list_recent_devices,
            forget_recent_device,
            capture_screenshot,
            delete_screenshot,
            send_file_to_device,
            list_device_files,
            fetch_file_from_device,
            current_recording,
            delete_recording,
            pair_wireless_device,
            connect_wireless_device,
            list_trusted_wireless_devices,
            forget_trusted_wireless_device
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                reclaim_children(app_handle);
            }
        });
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
        /// `screencap -p` 返回的字节流；`None` 表示读取失败。
        screenshot: Option<Vec<u8>>,
        /// 设备传输目录的 `ls -1` 输出；`None` 表示读取失败。
        device_listing: Option<String>,
        /// 为真时传输类调用（mkdir/push/pull）一律失败。
        transfer_fails: bool,
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

        fn wake_screen(&self, serial: &str) -> Result<(), std::io::Error> {
            self.calls.lock().unwrap().push(format!("wake {serial}"));
            Ok(())
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

        fn pair(&self, endpoint: &str, pairing_code: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("pair {endpoint} {pairing_code}"));
            Ok(())
        }

        fn connect(&self, endpoint: &str) -> Result<(), std::io::Error> {
            self.calls.lock().unwrap().push(format!("connect {endpoint}"));
            Ok(())
        }

        fn disconnect(&self, endpoint: &str) -> Result<(), std::io::Error> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("disconnect {endpoint}"));
            Ok(())
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
    }

    fn runtimes(adb: FakeAdb, mirror: FakeMirror) -> AppRuntimes {
        AppRuntimes {
            adb: Box::new(adb),
            mirror: Box::new(mirror),
        }
    }

    fn device(serial: &str, state: DeviceState) -> AdbDevice {
        AdbDevice {
            serial: serial.to_owned(),
            label: "测试设备".to_owned(),
            state,
        }
    }

    fn snapshot(store: &SessionStore) -> MirrorSession {
        store
            .lock()
            .expect("session state must be readable in tests")
            .session
            .clone()
    }

    /// 手工写入一个「进程正在运行」的会话，用于测试无法走完首次启动的路径。
    fn mark_running_for_test(store: &SessionStore, options: SessionOptions) {
        let mut state = store.lock().unwrap();
        state.epoch = state.epoch.wrapping_add(1);
        state.session = MirrorSession::streaming("phone".into());
        state.process = Some(Box::new(FakeProcess {
            exit: None,
            killed: Arc::new(Mutex::new(false)),
        }));
        state.options = options;
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
            };
            let args = options.arguments().unwrap();
            assert!(args.contains(&format!("--max-size={size}")));
            assert!(args.contains(&format!("--video-bit-rate={bitrate}")));
            assert!(args.contains(&"--video-codec=h264".into()));
            assert!(args.contains(&"--stay-awake".into()));
            assert!(args.contains(&"--fullscreen".into()));
            assert!(args.contains(&"--always-on-top".into()));
            assert!(args.contains(&"--display-orientation=90".into()));
        }
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
        mark_session(&store, MirrorSession::idle());
        assert!(reserve_session(&store, MirrorSession::connecting("phone".into())).is_ok());
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
        stop_mirroring_with(&store).unwrap();

        assert!(*killed.lock().unwrap());
        assert_eq!(snapshot(&store).phase, SessionPhase::Idle);
        assert_eq!(
            stop_mirroring_with(&store).unwrap_err().code,
            "session_not_running"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_graceful_stop_lets_a_recording_finalize_instead_of_killing() {
        // 用一个真实的子进程验证系统实现的 stop：SIGTERM 后进程退出，而不是被强杀。
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut process = SystemMirrorProcess { child };

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
        let mut process = SystemMirrorProcess { child };

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

        let error = apply_session_options_with(&runtimes, &store, SessionOptions::default(), None)
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
            apply_session_options_with(&runtimes, &store, SessionOptions::default(), None)
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
        )
        .unwrap();
        assert_eq!(session.starts.lock().unwrap().len(), 2);

        stop_mirroring_with(&session.store).unwrap();
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
        assert!(recovery.contains("唤醒屏幕"));

        let (explanation, _) =
            describe_lock_state(KeyguardState::Unknown, None, ScreenState::Unknown);
        assert!(explanation.contains("无法确认"));
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

    // -- 最近设备 --

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
    fn a_requested_recording_path_is_handed_to_the_mirror_process() {
        let directory = scratch_dir("recording-start");
        let mirror = FakeMirror::running();
        let records = Arc::clone(&mirror.records);
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            mirror,
        );
        let store = SessionStore::default();
        let path = directory.join("MirrorDock-20260928-171825.mp4");

        start_mirroring_with(
            &runtimes,
            &store,
            "phone".into(),
            SessionOptions {
                record: true,
                ..Default::default()
            },
            Some(path.clone()),
        )
        .unwrap();

        let recorded = records.lock().unwrap().clone();
        assert_eq!(
            recorded[0].as_deref(),
            Some(path.to_string_lossy().as_ref()),
            "录制路径必须原样交给镜像进程，不能由前端拼本机路径"
        );

        let recording = current_recording_with(&store).unwrap().unwrap();
        assert_eq!(recording.file_name, "MirrorDock-20260928-171825.mp4");
        assert!(recording.active, "进程还在跑，录像就还在写");

        let _ = fs::remove_dir_all(&directory);
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
        start_mirroring_with(
            &runtimes,
            &store,
            "phone".into(),
            SessionOptions {
                record: true,
                ..Default::default()
            },
            Some(live.clone()),
        )
        .unwrap();

        let error =
            remove_recording_file(&directory, "MirrorDock-20260928-171825.mp4", &store).unwrap_err();
        assert_eq!(error.code, "recording_in_progress");
        assert!(live.exists(), "正在写的录像文件不得被删掉");

        // 结束会话后即可删除；再删一次必须如实说「文件已经不在了」。
        stop_mirroring_with(&store).unwrap();
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

    #[test]
    fn turning_recording_on_restarts_the_session_with_a_new_recording_file() {
        let directory = scratch_dir("recording-restart");
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

        let path = directory.join("MirrorDock-20260928-180000.mp4");
        let update = apply_session_options_with(
            &runtimes,
            &store,
            SessionOptions {
                record: true,
                ..Default::default()
            },
            Some(path.clone()),
        )
        .unwrap();

        assert!(update.applied, "打开录制是一次真实的会话变更，必须重启生效");
        let recorded = records.lock().unwrap().clone();
        assert_eq!(recorded.len(), 2, "应当恰好启动两次：原会话 + 开启录制后重启");
        assert_eq!(recorded[0], None);
        assert_eq!(
            recorded[1].as_deref(),
            Some(path.to_string_lossy().as_ref())
        );
        assert!(current_recording_with(&store).unwrap().unwrap().active);

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
        stop_mirroring_with(&store).unwrap();
        assert_eq!(snapshot(&store).phase, SessionPhase::Idle);
        assert_eq!(
            stop_mirroring_with(&store).unwrap_err().code,
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
        )
        .unwrap();
        assert_eq!(starts.lock().unwrap().len(), 2);

        stop_mirroring_with(&store).unwrap();
        let ended = snapshot(&store);
        assert_eq!(ended.phase, SessionPhase::Idle);
        assert_eq!(
            ended.serial, None,
            "停止后不得残留上一个设备的序列号"
        );
    }
}
