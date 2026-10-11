//! REST 接口。
//!
//! 错误一律返回 md-core 的 AppError 结构（code/message/recovery），与桌面端
//! 同形——前端不必为两种形态各写一套错误处理分支。

use crate::adb;
use crate::record::{Quality, Recording};
use crate::State;
use axum::extract::{Path, Query, State as AxumState};
use axum::http::StatusCode;
use axum::Json;
use md_core::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 统一的错误出口：HTTP 状态码用 400，真正的语义由 body 里的 code 表达。
fn err(error: AppError) -> (StatusCode, Json<AppError>) {
    (StatusCode::BAD_REQUEST, Json(error))
}

/// 显式写出错误类型：axum 里 `Result<T, E>` 有多个 IntoResponse 实现，
/// 用 `impl IntoResponse` 兜着会让编译器无法选实现（E0283）。
pub type ApiResult = Result<Json<serde_json::Value>, (StatusCode, Json<AppError>)>;

#[derive(Serialize)]
pub struct Status {
    pub version: String,
    pub data_dir: String,
    pub adb: String,
    pub scrcpy: String,
    pub recording: Option<String>,
    /// 归档保留天数。0 = 不自动清理。前端据此显示「录像保留 N 天」，
    /// 让用户一眼知道旧录像会被回收——不然某天文件不见了会以为是丢数据。
    pub retention_days: u64,
}

pub async fn status(AxumState(state): AxumState<Arc<State>>) -> Json<Status> {
    let guard = state.recording.lock().await;
    Json(Status {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        data_dir: state.data_dir.display().to_string(),
        adb: adb::adb_binary(),
        scrcpy: adb::scrcpy_binary(),
        recording: guard.as_ref().map(|r| r.path.display().to_string()),
        retention_days: state.retention_days,
    })
}

/// 设备列表。去重与「USB 优先」由 md-core 决定，与桌面端给出同一份答案。
pub async fn devices() -> ApiResult {
    match adb::list_devices().await {
        Ok(list) => {
            let views: Vec<_> = list.iter().map(adb::to_view).collect();
            Ok(Json(serde_json::json!({ "devices": views })))
        }
        Err(message) => Err(err(AppError::new(
            "device_list_failed",
            "读取设备列表失败。",
            &format!(
                "请确认容器已透传 USB（--privileged 且挂载 /dev/bus/usb），并检查手机已授权本设备（adb 提示 {message}）。"
            ),
        ))),
    }
}

/// 截图。容器没有画面窗口，截图是无显示器场景下唯一能「看见手机」的手段，
/// 因此在 NAS 版里是核心能力而非附属功能。
#[derive(Deserialize)]
pub struct SerialQuery {
    pub serial: String,
}

pub async fn screenshot(Query(q): Query<SerialQuery>) -> ApiResult {
    let name = format!("shot-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let dir = std::env::var("MIRRORDOCK_DATA").unwrap_or_else(|_| "/var/lib/mirrordock".to_owned());
    let dir = std::path::Path::new(&dir).join("screenshots");
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        return Err(err(AppError::new(
            "screenshot_dir_unavailable",
            "无法创建截图目录。",
            &format!("请检查数据卷是否可写：{e}"),
        )));
    }
    let path = dir.join(&name);
    // adb exec-out screencap -p 直接输出 PNG 二进制。注意必须改用 output() 收字节
    // 流而不是 shell 重定向：部分设备的 shell 会把换行翻成 CRLF，落地的 PNG
    // 会被判损坏（这一条在命令行里怎么试都试不出来）。
    let args = vec![
        "-s".into(),
        q.serial.clone(),
        "exec-out".into(),
        "screencap".into(),
        "-p".into(),
    ];
    let out = std::process::Command::new(adb::adb_binary())
        .args(&args)
        .output();
    match out {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => {
            if let Err(e) = tokio::fs::write(&path, &o.stdout).await {
                return Err(err(AppError::new(
                    "screenshot_write_failed",
                    "截图写入失败。",
                    &format!("请检查数据卷是否可写：{e}"),
                )));
            }
            Ok(Json(
                serde_json::json!({ "file": name, "bytes": o.stdout.len() }),
            ))
        }
        Ok(o) => Err(err(AppError::new(
            "screenshot_failed",
            "截图失败。",
            &format!(
                "请确认手机已解锁并授权（{}）。",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        ))),
        Err(e) => Err(err(AppError::new(
            "screenshot_failed",
            "截图失败。",
            &format!("无法执行 adb：{e}"),
        ))),
    }
}

#[derive(Deserialize)]
pub struct StartRecord {
    pub serial: String,
    #[serde(default)]
    pub quality: Option<String>,
}

pub async fn start_record(
    AxumState(state): AxumState<Arc<State>>,
    Json(body): Json<StartRecord>,
) -> ApiResult {
    let mut guard = state.recording.lock().await;
    if guard.is_some() {
        return Err(err(AppError::new(
            "recording_in_progress",
            "已有录制正在进行。",
            "请先结束当前录制再开始新的录制。",
        )));
    }
    let quality = match body.quality.as_deref() {
        Some("smooth") => Quality::Smooth,
        Some("sharp") => Quality::Sharp,
        _ => Quality::Balanced,
    };
    let dir = state.data_dir.join("recordings");
    let started = Recording::start(
        &body.serial,
        &dir,
        quality,
        &adb::scrcpy_binary(),
        &adb::adb_binary(),
    )
    .await;
    match started {
        Ok(rec) => {
            let file = rec.path.display().to_string();
            *guard = Some(rec);
            Ok(Json(
                serde_json::json!({ "file": file, "serial": body.serial }),
            ))
        }
        Err(message) => Err(err(AppError::new(
            "record_start_failed",
            "录制启动失败。",
            &message,
        ))),
    }
}

pub async fn stop_record(AxumState(state): AxumState<Arc<State>>) -> ApiResult {
    let mut guard = state.recording.lock().await;
    let Some(mut rec) = guard.take() else {
        return Err(err(AppError::new(
            "recording_missing",
            "当前没有进行中的录制。",
            "请先开始录制。",
        )));
    };
    match rec.stop().await {
        Ok(()) => Ok(Json(
            serde_json::json!({ "file": rec.path.display().to_string() }),
        )),
        Err(message) => Err(err(AppError::new(
            "record_stop_failed",
            "录制结束异常。",
            &message,
        ))),
    }
}

/// 录像归档列表——这是「24h 录制归档」场景的门面。
pub async fn recordings(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let dir = state.data_dir.join("recordings");
    let mut read = match tokio::fs::read_dir(&dir).await {
        Ok(r) => r,
        Err(_) => return Json(serde_json::json!({ "recordings": [] })),
    };
    let mut items = Vec::new();
    while let Ok(Some(entry)) = read.next_entry().await {
        let meta = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() || !entry.file_name().to_string_lossy().ends_with(".mp4") {
            continue;
        }
        items.push(serde_json::json!({
            "name": entry.file_name().to_string_lossy(),
            "bytes": meta.len(),
            "modified": meta.modified().ok().map(|t| {
                t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
            }),
        }));
    }
    items.sort_by(|a, b| {
        let key = |v: &serde_json::Value| v["modified"].as_u64().unwrap_or(0);
        key(b).cmp(&key(a))
    });
    Json(serde_json::json!({ "recordings": items }))
}

#[derive(Deserialize)]
pub struct FileQuery {
    pub serial: String,
    #[serde(default)]
    pub path: Option<String>,
}

/// 列出手机上的文件。`ls -1` 用 argv 形式传入——串接成 "ls -1 <dir>" 与 argv
/// 形式的解析结果不同（桌面端 X10-74 排查时被这个差异误导过一次）。
pub async fn list_files(Query(q): Query<FileQuery>) -> ApiResult {
    let dir = q.path.clone().unwrap_or_else(|| "/sdcard".to_owned());
    let args = vec![
        "-s".into(),
        q.serial.clone(),
        "shell".into(),
        "ls".into(),
        "-1".into(),
        dir.clone(),
    ];
    match adb::run_adb(&args).await {
        Ok(raw) => {
            let entries: Vec<String> = raw
                .lines()
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty())
                .collect();
            Ok(Json(serde_json::json!({ "path": dir, "entries": entries })))
        }
        Err(message) => Err(err(AppError::new(
            "transfer_list_failed",
            "读取手机文件列表失败。",
            &format!("请确认路径存在且手机已授权（{message}）。"),
        ))),
    }
}

pub async fn delete_recording(
    AxumState(state): AxumState<Arc<State>>,
    Path(name): Path<String>,
) -> ApiResult {
    // 只接受纯文件名，杜绝 `../` 穿越到录像目录之外。
    if name.contains('/') || name.contains("..") {
        return Err(err(AppError::new(
            "recording_missing",
            "文件名不合法。",
            "请提供不含路径分隔符的文件名。",
        )));
    }
    let path = state.data_dir.join("recordings").join(&name);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(Json(serde_json::json!({ "deleted": name }))),
        Err(e) => Err(err(AppError::new(
            "recording_delete_failed",
            "删除录像失败。",
            &e.to_string(),
        ))),
    }
}
