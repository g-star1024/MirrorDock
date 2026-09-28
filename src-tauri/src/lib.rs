use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

/// 会话状态机轮询运行中进程的间隔。
const MONITOR_INTERVAL: Duration = Duration::from_millis(200);

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
///   device_not_selected / device_unauthorized / device_offline / device_not_connected
///   adb_missing / adb_unavailable
///   mirror_runtime_missing / mirror_start_failed / mirror_exited / mirror_stop_failed
///   session_unavailable / session_busy / session_not_running
///   invalid_rotation / endpoint_invalid / pairing_code_invalid / pairing_failed
///   connect_failed / connect_not_ready
///   trusted_list_unavailable / trusted_list_unreadable / trusted_list_write_failed
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
    fn pair(&self, endpoint: &str, pairing_code: &str) -> Result<(), std::io::Error>;
    fn connect(&self, endpoint: &str) -> Result<(), std::io::Error>;
    fn disconnect(&self, endpoint: &str) -> Result<(), std::io::Error>;
}

/// 一个已启动的镜像进程。抽象出 `try_wait` 与 `kill`，使会话生命周期可在测试中验证。
trait MirrorProcess: Send {
    /// `Some(success)` 表示进程已退出，`None` 表示仍在运行。
    fn try_wait(&mut self) -> Option<bool>;
    fn kill(&mut self) -> Result<(), std::io::Error>;
}

trait MirrorRuntime: Send + Sync {
    fn is_available(&self) -> bool;
    fn start(
        &self,
        serial: &str,
        options: &SessionOptions,
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

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Quality {
    Smooth,
    #[default]
    Balanced,
    Sharp,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SessionOptions {
    quality: Quality,
    fullscreen: bool,
    always_on_top: bool,
    rotation: u16,
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
        let output = Command::new("adb").args(args).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("adb returned a failing status"))
        }
    }
}

impl AdbRuntime for SystemAdbRuntime {
    fn list_devices(&self) -> Result<Vec<AdbDevice>, std::io::Error> {
        let output = Command::new("adb").args(["devices", "-l"]).output()?;
        if output.status.success() {
            Ok(parse_adb_devices(&String::from_utf8_lossy(&output.stdout)))
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
    ) -> Result<Box<dyn MirrorProcess>, std::io::Error> {
        let child = Command::new(scrcpy_binary())
            .arg("--serial")
            .arg(serial)
            .args(
                options
                    .arguments()
                    .map_err(|_| std::io::Error::other("invalid session options"))?,
            )
            .spawn()?;
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
}

struct SessionStore(Arc<Mutex<SessionState>>);

impl Default for SessionStore {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(SessionState {
            session: MirrorSession::idle(),
            process: None,
            epoch: 0,
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
) -> Result<u64, AppError> {
    let mut state = store.lock()?;
    state.epoch = state.epoch.wrapping_add(1);
    state.session = MirrorSession::streaming(serial);
    state.process = Some(process);
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
// scrcpy 运行时定位与解析
// ---------------------------------------------------------------------------

fn select_scrcpy_binary(explicit_path: Option<PathBuf>, development_path: &Path) -> PathBuf {
    if let Some(path) = explicit_path.filter(|path| path.is_file()) {
        return path;
    }

    if cfg!(debug_assertions) && development_path.is_file() {
        return development_path.to_path_buf();
    }

    PathBuf::from("scrcpy")
}

fn scrcpy_binary() -> PathBuf {
    let explicit_path = std::env::var_os("MIRRORDOCK_SCRCPY_PATH").map(PathBuf::from);
    let development_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tools/scrcpy/macos-x86_64/scrcpy");
    select_scrcpy_binary(explicit_path, &development_path)
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
fn start_mirroring(
    runtimes: State<AppRuntimes>,
    sessions: State<SessionStore>,
    serial: String,
    options: Option<SessionOptions>,
) -> Result<(), AppError> {
    start_mirroring_with(&runtimes, &sessions, serial, options.unwrap_or_default())
}

fn start_mirroring_with(
    runtimes: &AppRuntimes,
    sessions: &SessionStore,
    serial: String,
    options: SessionOptions,
) -> Result<(), AppError> {
    options.arguments()?;

    let serial = serial.trim().to_owned();
    if serial.is_empty() {
        return Err(AppError::new(
            "device_not_selected",
            "未选择可用设备。",
            "请重新检查连接后选择手机。",
        ));
    }

    reserve_session(sessions, MirrorSession::connecting(serial.clone()))?;

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

    match device_lookup(runtimes, &serial) {
        DeviceLookup::Ready => {}
        DeviceLookup::Unauthorized => {
            let error = AppError::new(
                "device_unauthorized",
                "手机尚未允许这台电脑进行调试。",
                "请解锁手机，在“允许 USB 调试吗？”提示中选择允许，然后重新检查。",
            );
            mark_session(
                sessions,
                MirrorSession::unauthorized(serial.clone(), error.clone()),
            );
            return Err(error);
        }
        DeviceLookup::Offline => {
            let error = AppError::new(
                "device_offline",
                "手机当前处于离线状态。",
                "请重新插拔数据线或重新连接无线调试，保持手机解锁后重试。",
            );
            mark_session(sessions, MirrorSession::offline(serial.clone(), error.clone()));
            return Err(error);
        }
        DeviceLookup::NotConnected => {
            return Err(fail_session(
                sessions,
                Some(serial),
                AppError::new(
                    "device_not_connected",
                    "找不到这台手机。",
                    "请确认数据线或无线连接仍然有效，然后重新检查。",
                ),
            ));
        }
        DeviceLookup::AdbUnavailable => {
            return Err(fail_session(
                sessions,
                Some(serial),
                AppError::new(
                    "adb_unavailable",
                    "Android 调试服务暂时不可用。",
                    "请拔下数据线后重新连接，或重新启动手机上的无线调试。",
                ),
            ));
        }
    }

    let process = match runtimes.mirror.start(&serial, &options) {
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

    let epoch = attach_process(sessions, process, serial.clone())?;
    spawn_session_monitor(sessions.clone(), epoch, serial);
    Ok(())
}

#[tauri::command]
fn stop_mirroring(sessions: State<SessionStore>) -> Result<(), AppError> {
    stop_mirroring_with(&sessions)
}

fn stop_mirroring_with(store: &SessionStore) -> Result<(), AppError> {
    match take_running_process(store)? {
        Some(mut process) => process.kill().map_err(|_| {
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

/// 应用退出时回收子进程，避免残留 scrcpy 进程。
fn reclaim_children(app: &AppHandle) {
    if let Some(store) = app.try_state::<SessionStore>() {
        if let Ok(Some(mut process)) = take_running_process(&store) {
            let _ = process.kill();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(SessionStore::default())
        .manage(AppRuntimes::system())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            check_adb_devices,
            start_mirroring,
            stop_mirroring,
            mirror_session,
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
        unavailable: bool,
        calls: Mutex<Vec<String>>,
    }

    impl FakeAdb {
        fn with_devices(devices: Vec<AdbDevice>) -> Self {
            Self {
                devices,
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
            if self.unavailable {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            } else {
                Ok(self.devices.clone())
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
    }

    impl FakeMirror {
        /// 启动后持续运行的镜像进程。
        fn running() -> Self {
            Self {
                available: true,
                exit: None,
                killed: Arc::new(Mutex::new(false)),
            }
        }

        /// 启动后立即以给定退出码结束的镜像进程。
        fn exiting(success: bool) -> Self {
            Self {
                available: true,
                exit: Some(success),
                killed: Arc::new(Mutex::new(false)),
            }
        }

        fn unavailable() -> Self {
            Self {
                available: false,
                exit: None,
                killed: Arc::new(Mutex::new(false)),
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
            _options: &SessionOptions,
        ) -> Result<Box<dyn MirrorProcess>, std::io::Error> {
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
            select_scrcpy_binary(Some(explicit.clone()), &development),
            explicit
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
            };
            let args = options.arguments().unwrap();
            assert!(args.contains(&format!("--max-size={size}")));
            assert!(args.contains(&format!("--video-bit-rate={bitrate}")));
            assert!(args.contains(&"--video-codec=h264".into()));
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
            start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default())
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
            start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default())
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
        )
        .unwrap_err();
        assert_eq!(error.code, "adb_unavailable");

        let empty = runtimes(FakeAdb::with_devices(Vec::new()), FakeMirror::running());
        let error =
            start_mirroring_with(&empty, &store, "phone".into(), SessionOptions::default())
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
            start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default())
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

        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default()).unwrap();

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

        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default()).unwrap();
        stop_mirroring_with(&store).unwrap();

        assert!(*killed.lock().unwrap());
        assert_eq!(snapshot(&store).phase, SessionPhase::Idle);
        assert_eq!(
            stop_mirroring_with(&store).unwrap_err().code,
            "session_not_running"
        );
    }

    #[test]
    fn monitor_returns_to_idle_when_the_process_exits_cleanly() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::exiting(true),
        );
        let store = SessionStore::default();

        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default()).unwrap();

        assert_eq!(wait_until_idle_or_failed(&store).phase, SessionPhase::Idle);
    }

    #[test]
    fn monitor_reports_a_recoverable_failure_when_the_process_exits_abnormally() {
        let runtimes = runtimes(
            FakeAdb::with_devices(vec![device("phone", DeviceState::Ready)]),
            FakeMirror::exiting(false),
        );
        let store = SessionStore::default();

        start_mirroring_with(&runtimes, &store, "phone".into(), SessionOptions::default()).unwrap();

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
}
