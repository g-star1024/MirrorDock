//! adb 执行层。
//!
//! 刻意做成薄壳：所有「哪些通道算同一台设备」「走 USB 还是无线」的判据都调用
//! md-core——与桌面端同源，避免 NAS 端给出和桌面端不一样的结果。

use md_core::devices::{dedup_devices, AdbDevice, ConnectionEndpoint, ConnectionKind, DeviceState};
use serde::Serialize;
use std::process::Stdio;
use tokio::process::Command;

/// adb 可执行文件位置。容器里随 scrcpy 一起装在 /opt/scrcpy，可由 ADB 覆盖。
pub fn adb_binary() -> String {
    std::env::var("ADB").unwrap_or_else(|_| "/opt/scrcpy/adb".to_owned())
}

/// scrcpy 可执行文件位置。
pub fn scrcpy_binary() -> String {
    std::env::var("SCRCPY").unwrap_or_else(|_| "/opt/scrcpy/scrcpy".to_owned())
}

/// 执行 adb 并回收输出。失败时把 stderr 一并带回——NAS 场景下最常见的
/// 「设备未授权 / 离线」原因写在 stderr 里，丢掉就无法告诉用户怎么修。
pub async fn run_adb(args: &[String]) -> Result<String, String> {
    let output = Command::new(adb_binary())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("无法执行 adb（路径 {}）：{e}", adb_binary()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// `adb devices -l` 的原始行解析。
///
/// 形态：`序列号 <TAB> 状态 [transport_id:1 ...]`。只认 `device`（已授权就绪），
/// `unauthorized` 与 `offline` 各自独立成状态——按工程铁律，禁止把它们塌缩成
/// 同一个「连接失败」，否则用户拿不到「请点手机上那次授权」这种可行动提示。
fn parse_device_line(line: &str) -> Option<ConnectionEndpoint> {
    let mut parts = line.split_whitespace();
    let serial = parts.next()?.to_owned();
    let state = match parts.next()? {
        "device" => DeviceState::Ready,
        "unauthorized" => DeviceState::Unauthorized,
        "offline" => DeviceState::Offline,
        _ => DeviceState::Unknown,
    };
    // 序列号里出现 `:` 或 ._adb-tls / ._tcp 一律按无线端点处理；与 md-core
    // 的 `is_wireless_endpoint` 保持同一口径，避免两端判定分叉。
    let kind = if serial.contains(':') || serial.contains("._adb-tls") || serial.contains("._tcp") {
        ConnectionKind::Wireless
    } else {
        ConnectionKind::Usb
    };
    Some(ConnectionEndpoint {
        serial,
        kind,
        state,
    })
}

/// 拉取已 dedup 的设备列表。去重与「USB 优先」完全由 md-core 决定。
pub async fn list_devices() -> Result<Vec<AdbDevice>, String> {
    let raw = run_adb(&["devices".into(), "-l".into()]).await?;
    let mut endpoints = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("List of devices") {
            continue;
        }
        if let Some(endpoint) = parse_device_line(trimmed) {
            endpoints.push(endpoint);
        }
    }
    // md-core 的 dedup_devices 需要 AdbDevice，这里先按每条通道造一个临时
    // AdbDevice 交给它合并——正是桌面端 list_devices 走的那条路径。
    let singletons = endpoints
        .into_iter()
        .map(|endpoint| AdbDevice {
            serial: endpoint.serial.clone(),
            label: String::new(),
            state: endpoint.state,
            physical_serial: None,
            connections: vec![endpoint],
        })
        .collect();
    Ok(dedup_devices(singletons))
}

/// 给前端的设备视图：只暴露必要字段，避免把内部结构泄漏出去。
#[derive(Debug, Clone, Serialize)]
pub struct DeviceView {
    pub serial: String,
    pub label: String,
    pub state: String,
    pub channels: Vec<String>,
    pub preferred_usb: bool,
}

pub fn to_view(device: &AdbDevice) -> DeviceView {
    DeviceView {
        serial: device.serial.clone(),
        label: if device.label.is_empty() {
            device.serial.clone()
        } else {
            device.label.clone()
        },
        state: match device.state {
            DeviceState::Ready => "ready".into(),
            DeviceState::Unauthorized => "unauthorized".into(),
            DeviceState::Offline => "offline".into(),
            DeviceState::Unknown => "unknown".into(),
        },
        channels: device
            .connections
            .iter()
            .map(|c| match c.kind {
                ConnectionKind::Usb => format!("usb:{}", c.serial),
                ConnectionKind::Wireless => format!("wireless:{}", c.serial),
            })
            .collect(),
        // md-core 合并后 serial 即首选通道；是无线则不走有线。
        preferred_usb: !md_core::devices::is_wireless_endpoint(&device.serial),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ready_usb_device() {
        let e = parse_device_line("ABC123               device product:x model:y").unwrap();
        assert_eq!(e.serial, "ABC123");
        assert_eq!(e.state, DeviceState::Ready);
        assert!(matches!(e.kind, ConnectionKind::Usb));
    }

    #[test]
    fn parses_wireless_and_unauthorized() {
        let ip = parse_device_line("192.168.1.9:5555	device").unwrap();
        assert!(matches!(ip.kind, ConnectionKind::Wireless));
        assert_eq!(ip.state, DeviceState::Ready);

        // 未经授权不能被当成「连接失败」塌缩掉——用户需要在手机上点确认。
        let ua = parse_device_line("ABC123	unauthorized").unwrap();
        assert_eq!(ua.state, DeviceState::Unauthorized);
    }

    #[test]
    fn mdns_endpoint_counts_as_wireless() {
        let mdns = parse_device_line("adb-1234._adb-tls-connect._tcp	device").unwrap();
        assert!(matches!(mdns.kind, ConnectionKind::Wireless));
    }

    #[test]
    fn skips_header_and_blank_lines() {
        assert!(parse_device_line("List of devices attached").is_some()); // 有内容但状态词不匹配 -> Unknown
        assert!(parse_device_line("").is_none());
    }
}
