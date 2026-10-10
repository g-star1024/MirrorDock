//! 结构化错误（自桌面端 lib.rs 逐字搬入，X10-113）。
//!
//! NAS 端的 HTTP 响应直接序列化本类型，保证错误码与桌面端同源，
//! 前端不必为两种形态各写一套错误处理分支。

use serde::Serialize;

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
pub struct AppError {
    pub code: &'static str,
    pub message: String,
    pub recovery: String,
}

impl AppError {
    pub fn new(code: &'static str, message: &str, recovery: &str) -> Self {
        Self {
            code,
            message: message.to_owned(),
            recovery: recovery.to_owned(),
        }
    }
}

pub fn adb_missing_error() -> AppError {
    AppError::new(
        "adb_missing",
        "未找到 Android 平台工具。",
        "请重新安装 MirrorDock 或联系支持人员。",
    )
}

/// 把 ADB 子进程的失败翻译成面向用户的错误；可执行文件缺失单独成码。
pub fn adb_command_error(
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
