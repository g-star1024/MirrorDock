//! 设备发现与通道语义（自桌面端 lib.rs 逐字搬入，X10-113）。
//!
//! 这里决定「同一台手机同时插着 USB 且开着无线时走哪条通道」——
//! NAS 端必须给出与桌面端完全一致的答案。用户在两端看到不同的优先级
//! 是最难排查的一类问题，故把判据收成唯一一份实现。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Ready,
    Unauthorized,
    Offline,
    Unknown,
}

/// 一条连接通道：adb 序列号（USB 为硬件序列号，无线为 `IP:端口`）+ 连接方式 + 状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    Usb,
    Wireless,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectionEndpoint {
    pub serial: String,
    pub kind: ConnectionKind,
    pub state: DeviceState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdbDevice {
    /// 首选连接通道的 adb 序列号（用于发起镜像/唤醒等动作）。
    pub serial: String,
    pub label: String,
    /// 所有通道里最好的状态：任一通道就绪即视为就绪。
    pub state: DeviceState,
    /// 硬件序列号（`ro.serialno`）。同一台手机无论 USB 还是无线都一致，用于跨连接去重。
    /// 仅当设备已授权（ready）时可读取；读不到时为 `None`。
    pub physical_serial: Option<String>,
    /// 该物理设备当前在 adb 中出现的全部通道（USB / 无线可能并存）。
    pub connections: Vec<ConnectionEndpoint>,
}

/// 无线设备的 adb 序列号有两类：
/// 1. `IP:端口`（含冒号）；
/// 2. 无线调试的 mDNS 发现条目，形如 `adb-<id>-<name>._adb-tls-connect._tcp`——
///    不含冒号但也不是 USB，漏判会把纯 Wi-Fi 设备误标成「USB + 无线」。
///
/// USB 序列号是纯硬件号（如 `79j7kn9tkjt8rwss`），不含冒号、点号与 `_tcp`。
/// 与前端 `looksLikeWirelessEndpoint` 保持同一判定，仅用于通道归类与界面提示。
pub fn is_wireless_endpoint(serial: &str) -> bool {
    serial.contains(':') || serial.contains("._adb-tls") || serial.contains("._tcp")
}

/// 通道/状态优先级：状态越「可用」排名越高；并列时 USB 优先于无线。
pub fn state_rank(state: DeviceState) -> u8 {
    match state {
        DeviceState::Ready => 3,
        DeviceState::Unauthorized => 2,
        DeviceState::Offline => 1,
        DeviceState::Unknown => 0,
    }
}

pub fn device_has_usb(device: &AdbDevice) -> bool {
    device
        .connections
        .iter()
        .any(|connection| connection.kind == ConnectionKind::Usb)
}

/// 合并组内选首选通道的优先级：USB > 无线 `IP:端口` > mDNS 发现名。
/// mDNS 条目（`adb-…._adb-tls-connect._tcp`）虽是有效 transport，但名字不稳定、
/// 不宜作为发起镜像的端点；`IP:端口` 是无线调试的常规稳定端点。
pub fn endpoint_preference(device: &AdbDevice) -> u8 {
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
pub fn dedup_devices(devices: Vec<AdbDevice>) -> Vec<AdbDevice> {
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
pub fn wireless_ip_endpoint(serial: &str) -> Option<&str> {
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
pub fn mdns_serial_contains_usb(mdns_serial: &str, usb_serial: &str) -> bool {
    usb_serial.len() >= 6 && mdns_serial.contains(usb_serial)
}

/// 一个就绪组（同一物理手机）的全部 USB 序列号，用于和 mDNS 实例名互认。
pub fn group_usb_serials(group: &[AdbDevice]) -> Vec<String> {
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
pub fn group_wireless_ips(group: &[AdbDevice]) -> Vec<String> {
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
pub fn device_shadows_group(
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
pub fn merge_offline_shadows(shadows: Vec<AdbDevice>) -> Vec<AdbDevice> {
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
