import { useEffect, useRef, useState, type ReactElement } from "react";
import { AppSelect } from "./AppSelect";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
// 系统文件选择器由官方 dialog 插件提供；MirrorDock 自身不枚举、不猜测用户文件。
import { open as openFilePicker, save as saveFilePicker } from "@tauri-apps/plugin-dialog";
import { check as checkForUpdate } from "@tauri-apps/plugin-updater";
// 会话中的系统级快捷键：镜像窗口（scrcpy 窗口）持有焦点时主窗口收不到键盘事件，
// 只有全局快捷键能不切回主窗口就触发截图/录制/旋转。
import { register, unregisterAll } from "@tauri-apps/plugin-global-shortcut";
import { brandGuides, detectBrand, type BrandGuide } from "./brandGuides";
import { helpArticles } from "./helpContent";
import QRCode from "qrcode";
import "./App.css";

type DeviceState = "ready" | "unauthorized" | "offline" | "unknown";

type ConnectionKind = "usb" | "wireless";

type ConnectionEndpoint = {
  serial: string;
  kind: ConnectionKind;
  state: DeviceState;
};

type Device = {
  serial: string;
  label: string;
  state: DeviceState;
  // 硬件序列号（ro.serialno）。同一台手机 USB 与无线一致，用于跨连接去重。
  // 仅已授权设备才会带此值；未授权/离线的设备为 null。
  physical_serial: string | null;
  // 该物理设备当前在 adb 中出现的所有通道（USB / 无线可能并存）。
  connections: ConnectionEndpoint[];
};

type AdbCheck = {
  adb_available: boolean;
  scrcpy_available: boolean;
  devices: Device[];
  diagnostic: string | null;
};

type TrustedWirelessDevice = { endpoint: string };
// 最近使用记录只保存在这台电脑上，用户可以逐条移除。last_used_at 是 Unix 秒。
type RecentDevice = { serial: string; label: string; last_used_at: number };
// 无线设备的标识有两类：`ip:port`（含冒号），以及无线调试的 mDNS 发现条目
// （形如 `adb-xxx._adb-tls-connect._tcp`，不含冒号但也不是 USB）。
// 仅用于决定给出哪种恢复动作；与后端 is_wireless_endpoint 保持同一判定。
function looksLikeWirelessEndpoint(serial: string) {
  return serial.includes(":") || serial.includes("._adb-tls") || serial.includes("._tcp");
}
// 通道提示：同一台设备可能同时走 USB 与无线，合并后需让用户看见。
// 仅统计「已就绪」的通道——adb 拔除后可能残留陈旧/offline 条目，
// 若把它算进徽标，会让只用 Wi-Fi 的设备误标成「USB + 无线」。
function connectionLabel(device: Device) {
  const kinds = new Set(
    device.connections
      .filter((connection) => connection.state === "ready")
      .map((connection) => connection.kind)
  );
  if (kinds.has("usb") && kinds.has("wireless")) return "USB + 无线";
  if (kinds.has("wireless")) return "无线";
  if (kinds.has("usb")) return "USB";
  return "";
}
// 同型号多台设备会显示相同名称（如两台 M2104K10AC），追加 -1、-2 后缀便于区分。
// 以设备在列表中的出现顺序统一编号，key 用实际端点 serial。
function buildDisplayLabels(devices: Device[]): Record<string, string> {
  const counts = new Map<string, number>();
  for (const device of devices) {
    counts.set(device.label, (counts.get(device.label) ?? 0) + 1);
  }
  const seen = new Map<string, number>();
  const labels: Record<string, string> = {};
  for (const device of devices) {
    if ((counts.get(device.label) ?? 1) > 1) {
      const index = (seen.get(device.label) ?? 0) + 1;
      seen.set(device.label, index);
      labels[device.serial] = `${device.label} -${index}`;
    } else {
      labels[device.serial] = device.label;
    }
  }
  return labels;
}
// 最近设备按「实际连接序列号」存储；去重后要在合并设备里按硬件序列号或任一通道匹配。
function findConnectedDevice(devices: Device[] | undefined, serial: string): Device | undefined {
  if (!devices) return undefined;
  return devices.find(
    (device) =>
      device.physical_serial === serial ||
      device.serial === serial ||
      device.connections.some((connection) => connection.serial === serial),
  );
}

// X10-84：把「adb serial / 会话 serial / 无线端点」解析成桌面模式偏好的稳定键。
// 有 physical_serial 用它（USB 与无线一致，重连不变）；解析不到则原样返回
// （可能是已下线设备的历史 pref 键，保持可读）。这是 desktopPrefs 全链路唯一的键口径。
export function desktopPrefKeyFor(devices: Device[] | undefined, serial: string | null): string | null {
  if (!serial) return null;
  const device = findConnectedDevice(devices, serial);
  if (device?.physical_serial) return device.physical_serial;
  return normalizeDeviceIdentityKey(serial);
}

// X10-87：把「adb 无线端点 / mDNS 端点」归一化成物理序列号。
// 形态：`adb-<physical>-<随机>._adb-tls-connect._tcp` → `<physical>`；
// `192.168.x.x:port` 这类 ip:port 端点不含物理 id，无法本地归并（留给设备在线时反查）。
// 其他（短物理 id / USB serial / 型号名）原样返回。
export function normalizeDeviceIdentityKey(key: string): string {
  const mdns = key.match(/^adb-([a-z0-9]+)-[a-z0-9]+\._adb-tls-connect\._tcp$/i);
  if (mdns) return mdns[1];
  return key;
}

// X10-88：设备自定义备注。键用稳定 physical_serial（与桌面模式 pref 同口径），
// 无线/USB 端点怎么变都指向同一条备注。空串视为「清除备注」，回落到型号名。
const DEVICE_NICKNAMES_KEY = "mirrordock.deviceNicknames";
export function readDeviceNicknames(): Record<string, string> {
  try {
    const value = JSON.parse(localStorage.getItem(DEVICE_NICKNAMES_KEY) ?? "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    const result: Record<string, string> = {};
    for (const [key, nickname] of Object.entries(value as Record<string, unknown>)) {
      if (typeof nickname === "string" && nickname.trim()) result[key] = nickname.trim();
    }
    return result;
  } catch {
    return {};
  }
}
export function writeDeviceNickname(key: string, nickname: string): Record<string, string> {
  const current = readDeviceNicknames();
  const trimmed = nickname.trim();
  if (trimmed) current[key] = trimmed;
  else delete current[key];
  try { localStorage.setItem(DEVICE_NICKNAMES_KEY, JSON.stringify(current)); } catch { /* 存储不可用时本次会话仍生效 */ }
  return current;
}
// 设备显示名 = 自定义备注 > （同名去重后的）型号标签。备注以 physical_serial 为键。
export function deviceDisplayName(
  device: { serial: string; label: string; physical_serial: string | null },
  nicknames: Record<string, string>,
  fallbackLabel: string,
): string {
  const key = device.physical_serial ?? device.serial;
  return nicknames[key] ?? fallbackLabel;
}
export function relativeTime(seconds: number) {
  if (!seconds) return "使用时间未知";
  const diff = Date.now() / 1000 - seconds;
  if (diff < 60) return "刚刚使用";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前使用`;  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前使用`;
  const days = Math.floor(diff / 86400);
  return days === 1 ? "昨天使用" : `${days} 天前使用`;
}
type AppError = { code: string; message: string; recovery: string };
// 诊断包：预览内容与导出文件内容一致——不多不少。
export type DiagnosticsEvent = { timestamp_ms: number; kind: string; code: string; detail: string };
export type DiagnosticsPreviewData = { generated_at_ms: number; app_version: string; system: string; scrcpy_available: boolean; events: DiagnosticsEvent[] };
type DiagnosticsReceipt = { path: string; events: number; bytes: number };
export type SessionPhase = "idle" | "unauthorized" | "offline" | "paired" | "connecting" | "streaming" | "failed";
// 进程正在运行不等于首帧已到达；未接入端到端探针前后端只会返回 unknown。
export type FirstFrame = "unknown" | "reached";
export type MirrorSession = { phase: SessionPhase; serial: string | null; first_frame: FirstFrame; error: AppError | null };
// 能力探测的结论用 null 表示"未知"，不得默认成"支持"或"不支持"。
type NoticeLevel = "info" | "limitation";
type CapabilityNotice = { code: string; level: NoticeLevel; title: string; detail: string };
export type DeviceCapabilities = {
  serial: string;
  label: string;
  android_release: string | null;
  sdk: number | null;
  mirroring_supported: boolean | null;
  audio_forwarding_supported: boolean | null;
  notices: CapabilityNotice[];
};
// 锁屏与屏幕状态。后端读不到时会返回 unknown，前端必须原样展示“未知”而不是猜。
type KeyguardState = "locked" | "unlocked" | "unknown";
type ScreenState = "awake" | "asleep" | "unknown";
export type DeviceLockReport = {
  keyguard: KeyguardState;
  secure_lock: boolean | null;
  screen: ScreenState;
  explanation: string;
  recovery: string;
};
type SessionOptions = { quality: "smooth" | "balanced" | "sharp"; fullscreen: boolean; always_on_top: boolean; rotation: number; keep_awake: boolean; record: boolean; clipboard_autosync: boolean; audio: boolean; shortcut_mod: string | null; show_touches: boolean; keyboard_uhid: boolean; read_only: boolean; max_fps: number | null; desktop_mode: boolean; desktop_app: string | null; camera_source: boolean };
// 镜像窗口形态由启动参数决定，运行中无法改写：后端「应用新设置」= 结束旧窗口 + 按新设置重开。
type SessionUpdate = { applied: boolean; note: string | null; session: MirrorSession };
// 最近一次会话的录制文件。active 表示此刻进程是否仍在写这个文件。
type Recording = { file_name: string; path: string; active: boolean };

// C4-01 伴侣 App 配对（POC）：桌面作为 TLS 服务端，二维码携带连接信息
// 与服务器证书 SPKI 指纹，伴侣 App 出带校验后建立加密会话。
type PairingOffer = {
  version: number;
  hosts: string[];
  port: number;
  token: string;
  fingerprint: string;
};
type PairingStatus = {
  phase: "idle" | "listening" | "connected";
  // M4-2 常驻通道：true = 常驻监听中，已配对设备可免扫码直连。
  resident: boolean;
  events: string[];
  offer: PairingOffer | null;
};

// 开发者中心自动发现（mDNS）：手机无线调试页广播的服务。
type WirelessServices = { pairing: string[]; connect: string[] };

// 应用级设置（后端持久化到 app-settings.json）：只影响客户端自身行为
// （窗口、图标、断线自动重连），与镜像会话参数（SessionOptions）严格分开。
export type AppSettingsView = { hide_dock_icon: boolean; auto_reconnect: boolean; recording_dir?: string | null };

// 归一化后端返回的通用设置：布尔给默认、recording_dir 透传（null=用系统默认目录）。
function normalizeAppSettings(s: Partial<AppSettingsView>): AppSettingsView {
  return {
    hide_dock_icon: s.hide_dock_icon === true,
    auto_reconnect: s.auto_reconnect !== false,
    recording_dir: typeof s.recording_dir === "string" && s.recording_dir.length > 0 ? s.recording_dir : null,
  };
}
// 平台判断：只有 macOS 提供「隐藏 Dock 图标」。做成纯函数方便测试。
export function isMacPlatform(userAgent: string): boolean {
  return /Macintosh|Mac OS X/.test(userAgent);
}

// 扫码配对（Android 11+ 无线调试「使用二维码配对设备」）：
// 桌面出码 → 手机扫码 → 手机广播配对服务 → 桌面自动 adb pair / connect。
type QrPairingOffer = { payload: string; service_name: string; pairing_code: string };
type QrPairingProgress = { stage: "waiting" | "pairing" | "paired" | "done" | "ended"; detail: string | null };
export const qrPairingStageCopy: Record<string, string> = {
  waiting: "等待手机扫码",
  pairing: "已发现手机，正在配对",
  paired: "配对成功，正在连接",
  done: "配对并连接完成",
  ended: "已取消",
};

/// 二维码载荷格式：MDP2|主机列表(逗号分隔)|端口|一次性配对码|桌面身份 SPKI SHA-256。
/// 伴侣 App 与桌面侧共享同一约定（见 companion_pairing.rs 模块注释）。
/// MDP2 起：指纹对应桌面长期身份（重连免扫码），配对后设备进入互信台账。
export function pairingPayload(offer: Pick<PairingOffer, "hosts" | "port" | "token" | "fingerprint">): string {
  return `MDP2|${offer.hosts.join(",")}|${offer.port}|${offer.token}|${offer.fingerprint}`;
}
/// 已配对伴侣设备（桌面互信台账条目，M4-1）。
export type PairedCompanion = { pairing_id: string; model: string; pubkey_hex: string; added_at: number; last_seen: number };

// 本地权益状态：无账户、无激活服务器，后端验签后回传。edition 只有 free/pro。
export type EntitlementView = { edition: string; key_id: string | null; expires_at: number | null };
export function editionLabel(edition: string | undefined | null) {
  return edition === "pro" ? "专业版" : edition === "free" ? "免费版" : "版本未知";
}
export function isProEdition(edition: string | undefined | null) {
  return edition === "pro";
}
export function expiryText(expiresAt: number | null) {
  if (!expiresAt) return "永久有效";
  return `有效期至 ${new Date(expiresAt * 1000).toLocaleDateString()}`;
}
// 截图结果。后端只回文件名、路径与字节数，不回传任何像素数据。
type Screenshot = { file_name: string; path: string; bytes: number };
// 与截图共用同一回执形状：发送时 path 是手机上的路径，取回时是本机路径。
type TransferReceipt = { file_name: string; path: string; bytes: number };

/** X10-66 通知镜像一期：手机转来的通知（内容只在内存，不落盘）。
 * `key`/`replyable` 由 0.2.5+ 伴侣端提供：只有 replyable=true（通知自带
 * RemoteInput 回复动作）时才显示快捷回复框（X10-69）。 */
export type PhoneNotification = {
  pkg: string;
  app: string;
  title: string;
  text: string;
  posted: number;
  key?: string;
  replyable?: boolean;
};

/** 通知时间展示：当天的只显示时刻，跨天带日期。 */
export function notificationTime(posted: number, now: number = Date.now()): string {
  const date = new Date(posted);
  const pad = (n: number) => String(n).padStart(2, "0");
  const hm = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  const sameDay = new Date(now).toDateString() === date.toDateString();
  return sameDay ? hm : `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${hm}`;
}
// 安装 APK 的回执：summary 是后端把 adb 结论解析后的可读结果。
type ApkInstallReceipt = { file_name: string; bytes: number; summary: string };
const defaultOptions: SessionOptions = { quality: "balanced", fullscreen: false, always_on_top: false, rotation: 0, keep_awake: true, record: false, clipboard_autosync: true, audio: true, shortcut_mod: null, show_touches: false, keyboard_uhid: true, read_only: false, max_fps: null, desktop_mode: false, desktop_app: null, camera_source: false };
export function readOptions(): SessionOptions {
  try {
    const value = JSON.parse(localStorage.getItem("mirrordock.sessionOptions") ?? "null");
    if (value && ["smooth", "balanced", "sharp"].includes(value.quality) && [0,90,180,270].includes(value.rotation) && typeof value.fullscreen === "boolean" && typeof value.always_on_top === "boolean") {
      // 旧版本没有 keep_awake / record / clipboard_autosync / audio 字段：按各自
      // 默认值取向回填（唤醒开、录制关、剪贴板同步开、声音转发开）。
      return {
        quality: value.quality,
        rotation: value.rotation,
        fullscreen: value.fullscreen,
        always_on_top: value.always_on_top,
        keep_awake: typeof value.keep_awake === "boolean" ? value.keep_awake : true,
        record: typeof value.record === "boolean" ? value.record : false,
        clipboard_autosync: typeof value.clipboard_autosync === "boolean" ? value.clipboard_autosync : true,
        audio: typeof value.audio === "boolean" ? value.audio : true,
        shortcut_mod: ["lctrl", "rctrl", "lalt", "ralt", "lsuper", "rsuper"].includes(value.shortcut_mod) ? value.shortcut_mod : null,
        show_touches: typeof value.show_touches === "boolean" ? value.show_touches : false,
        keyboard_uhid: typeof value.keyboard_uhid === "boolean" ? value.keyboard_uhid : true,
        read_only: typeof value.read_only === "boolean" ? value.read_only : false,
        max_fps: [24, 30, 60].includes(value.max_fps) ? value.max_fps : null,
        desktop_mode: typeof value.desktop_mode === "boolean" ? value.desktop_mode : false,
        desktop_app: typeof value.desktop_app === "string" && value.desktop_app.trim() ? value.desktop_app.trim() : null,
        camera_source: typeof value.camera_source === "boolean" ? value.camera_source : false,
      };
    }
  } catch { /* Invalid or unavailable local settings use defaults. */ }
  return defaultOptions;
}

// X10-71：桌面模式是设备级能力——多台设备不会都开桌面模式，「桌面模式开关 +
// 虚拟屏启动的应用」按设备单独保存（localStorage），其余设置仍是全局一份。
// 手机序列号（无线端点）在重连后可能变化，此时按新端点重新设置即可。
export type DeviceApp = { package: string; name: string };
export type DesktopPref = { desktop_mode: boolean; desktop_app: string | null };

// 把某台设备的桌面模式偏好叠加到全局设置上：仅覆盖桌面模式相关字段，并维持
// 与「后置摄像头画面」的互斥（开桌面模式时摄像头源强制关闭）。
export function composeOptionsWithDesktop(options: SessionOptions, pref: DesktopPref | null): SessionOptions {
  if (!pref) return options;
  return {
    ...options,
    desktop_mode: pref.desktop_mode,
    // 桌面模式关闭时不下发 desktop_app，避免遗留包名触发后端白名单校验失败。
    desktop_app: pref.desktop_mode ? pref.desktop_app : null,
    camera_source: pref.desktop_mode ? false : options.camera_source,
  };
}

// X10-71 + X10-72：桌面模式是设备级能力。旧的实现只在「两台以上设备正在镜像」
// 时才渲染设备选择器，单设备 / 未镜像时读写落到全局 options —— 功能存在但用户
// 看不见、也无法在未镜像时按设备预配。这里把候选设备的构建提成纯函数：
//
// 候选来源（去重、按 adb 顺序稳定输出）：
//   1. 当前 adb 设备列表（ready / unauthorized / offline 全部列出，让用户能提前配置）
//   2. 正在镜像或连接的会话设备（可能已从 adb 列表消失）
//   3. 已有桌面模式偏好的设备（历史配置，不能因为设备不在列表里就丢掉入口）
//
// 无线端点（192.168.x.x:port）重连后会变，调用方需按 physical_serial 归并，
// 归并后取当前活跃端点作为 key。
export type DesktopPrefDevice = {
  serial: string;
  label: string;
  state: DeviceState;
  /** 该设备是否正在镜像/连接 */
  streaming: boolean;
  /** 该设备是否已有独立的桌面模式配置（用于列表上的"已配置"标记） */
  configured: boolean;
  /**
   * 桌面模式偏好的稳定键（X10-84）：physical_serial（ro.serialno），USB 与无线一致。
   * 无线端点（ip:port / adb-xxx._adb-tls-connect._tcp）重连后会变，绝不能当 pref 键；
   * 没有 physical_serial（未授权/离线残留）时退回 serial。
   */
  pref_key: string;
};

export function buildDesktopPrefDevices(input: {
  adbDevices: Device[];
  labels?: Record<string, string>;
  sessionSerials?: string[];
  prefs?: Record<string, DesktopPref>;
}): DesktopPrefDevice[] {
  const { adbDevices, labels = {}, sessionSerials = [], prefs = {} } = input;
  // 物理序列号 → 当前端点：同一台手机的 USB/无线端点归并，避免出现两行。
  const byPhysical = new Map<string, Device>();
  const bySerial = new Map<string, Device>();
  for (const device of adbDevices) {
    bySerial.set(device.serial, device);
    const key = device.physical_serial ?? device.serial;
    const existing = byPhysical.get(key);
    // 已存在的通道不覆盖：ready 优先于其它状态，保证列表首行是能连的那条。
    if (!existing || (existing.state !== "ready" && device.state === "ready")) {
      byPhysical.set(key, device);
    }
  }
  // 会话设备但已不在 adb 列表（设备断开、序列号变更）也要给入口，否则用户无法
  // 找到并清理它的桌面模式设置。
  for (const serial of sessionSerials) {
    if (bySerial.has(serial)) continue;
    const key = prefs[serial] ? serial : serial;
    if (byPhysical.has(key)) continue;
    byPhysical.set(key, {
      serial,
      label: labels[serial] ?? serial,
      state: "offline",
      physical_serial: null,
      connections: [],
    });
  }
  // 已有偏好的设备若不在 adb 列表也补进来（配置持久化在本地，设备可能长期未插）。
  // X10-87：pref 键先归一化——同一物理设备的 mDNS 端点/短物理 id 归并为一行，
  // 避免「adb-xxx-..._tcp」和「xxx」并列出现两条（用户实测一台设备显示多条）。
  for (const serial of Object.keys(prefs)) {
    const normalizedKey = normalizeDeviceIdentityKey(serial);
    if (bySerial.has(normalizedKey)) continue;
    if (byPhysical.has(normalizedKey)) continue;
    byPhysical.set(normalizedKey, {
      serial: normalizedKey,
      label: labels[normalizedKey] ?? labels[serial] ?? normalizedKey,
      state: "offline",
      physical_serial: normalizedKey,
      connections: [],
    });
  }
  const sessionSet = new Set(sessionSerials);
  return [...byPhysical.values()].map((device) => {
    // X10-84：pref 键一律用稳定 physical_serial；没有它才退回当前 serial。
    // 无线端点会变，若用 serial 当键，启动时（adb serial）与设置时（归并键）
    // 会对不上，desktop_app 丢失、--start-app 不下发（用户实测白屏根因）。
    const prefKey = device.physical_serial ?? device.serial;
    return {
      serial: device.serial,
      label: labels[device.serial] ?? device.label,
      state: device.state,
      streaming: sessionSet.has(device.serial) || device.connections.some((c) => sessionSet.has(c.serial)),
      configured: Boolean(prefs[prefKey]),
      pref_key: prefKey,
    };
  });
}

export function readDesktopPrefs(): Record<string, DesktopPref> {
  try {
    const value = JSON.parse(localStorage.getItem("mirrordock.desktopPrefs") ?? "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    const prefs: Record<string, DesktopPref> = {};
    for (const [serial, pref] of Object.entries(value as Record<string, unknown>)) {
      if (!pref || typeof pref !== "object") continue;
      const candidate = pref as Record<string, unknown>;
      if (typeof candidate.desktop_mode !== "boolean") continue;
      prefs[serial] = {
        desktop_mode: candidate.desktop_mode,
        desktop_app: typeof candidate.desktop_app === "string" && candidate.desktop_app.trim() ? candidate.desktop_app.trim() : null,
      };
    }
    return prefs;
  } catch {
    return {};
  }
}

// 会话进行中可用的系统级快捷键（B-快捷键）。镜像窗口聚焦时只有全局快捷键能
// 收到按键；CommandOrControl 在 Windows/Linux 是 Ctrl、macOS 是 ⌘。
// 组合可由用户在设置页自定义，默认值与 ToDesk 类工具的习惯一致。
export type ShortcutSettings = { screenshot: string; record: string; rotate: string };
export const defaultShortcuts: ShortcutSettings = {
  screenshot: "CommandOrControl+Alt+S",
  record: "CommandOrControl+Alt+R",
  rotate: "CommandOrControl+Alt+D",
};
const shortcutModifiers = new Set(["CommandOrControl", "Command", "Cmd", "Control", "Ctrl", "Alt", "Option", "Meta", "Super", "Shift"]);
// 合法组合 = 至少一个修饰键 + 一个非修饰键（避免单键全局抢占）。大小写不敏感。
export function isValidShortcut(combo: string): boolean {
  const parts = combo.trim().split("+").map((part) => part.trim()).filter(Boolean);
  if (parts.length < 2) return false;
  const keys = parts.map((part) => part.charAt(0).toUpperCase() + part.slice(1));
  return keys.some((key) => shortcutModifiers.has(key)) && keys.some((key) => !shortcutModifiers.has(key));
}
const SHORTCUT_ACTIONS = ["screenshot", "record", "rotate"] as const;
export function readShortcuts(): ShortcutSettings {
  const fallback = { ...defaultShortcuts };
  try {
    const value = JSON.parse(localStorage.getItem("mirrordock.shortcuts") ?? "null");
    if (value && typeof value === "object") {
      for (const action of SHORTCUT_ACTIONS) {
        const combo = (value as Record<string, unknown>)[action];
        if (typeof combo === "string" && isValidShortcut(combo)) fallback[action] = combo.trim();
      }
    }
  } catch { /* 本地设置不可用时用默认组合。 */ }
  return fallback;
}
export function sessionShortcutKeys(settings: ShortcutSettings = readShortcuts()): string[] {
  return [settings.screenshot, settings.record, settings.rotate];
}

// 截图与录像的文件名都按**本机时间**生成（后端不猜时区），随后由后端按 ASCII 白名单
// 严格校验。只使用数字与短横线，任何路径分隔符都不会出现在这里。
function localTimestamp(now: Date) {
  const pad = (value: number) => String(value).padStart(2, "0");
  const date = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
  return `${date}-${time}`;
}

export function screenshotFileName(now: Date) {
  return `MirrorDock-${localTimestamp(now)}.png`;
}

export function recordingFileName(now: Date) {
  return `MirrorDock-${localTimestamp(now)}.mp4`;
}

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} 字节`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function lockSummary(report: DeviceLockReport) {
  const keyguard =
    report.keyguard === "locked"
      ? report.secure_lock === true
        ? "已锁屏（需要解锁凭据）"
        : "锁屏中"
      : report.keyguard === "unlocked"
        ? "已解锁"
        : "锁屏状态未知";
  const screen = report.screen === "awake" ? "屏幕已点亮" : report.screen === "asleep" ? "屏幕已关闭" : "屏幕状态未知";
  return `${keyguard} · ${screen}`;
}

/** 设备卡上的锁屏便签（X10-33）：短语级，替代原先的整块锁屏状态面板。 */
export function lockTag(report: DeviceLockReport): string {
  const keyguard =
    report.keyguard === "locked"
      ? report.secure_lock === true
        ? "安全锁屏"
        : "已锁屏"
      : report.keyguard === "unlocked"
        ? "已解锁"
        : "锁屏未知";
  const screen = report.screen === "awake" ? "亮屏" : report.screen === "asleep" ? "熄屏" : "屏幕未知";
  return `${keyguard} · ${screen}`;
}

export function errorMessage(error: unknown, fallback: string) {
  if (typeof error === "object" && error !== null && "message" in error && "recovery" in error) {
    const detail = error as AppError;
    return `${detail.message} ${detail.recovery}`;
  }
  return typeof error === "string" ? error : fallback;
}

export function sessionErrorText(session: MirrorSession, fallback: string) {
  return session.error ? `${session.error.message} ${session.error.recovery}` : fallback;
}

export function supportText(value: boolean | null, yes: string, no: string, unknown: string) {
  return value === true ? yes : value === false ? no : unknown;
}

export function capabilitySummary(capabilities: DeviceCapabilities) {
  const system = capabilities.android_release
    ? `Android ${capabilities.android_release}`
    : "系统版本未知";
  const mirror = supportText(
    capabilities.mirroring_supported,
    "画面可以镜像到电脑",
    "系统较旧，画面镜像可能不稳定",
    "画面能否镜像还无法确认",
  );
  const audio = supportText(
    capabilities.audio_forwarding_supported,
    "手机声音会一起传到电脑",
    "手机声音无法传到电脑",
    "手机声音能否转发还无法确认",
  );
  return `${capabilities.label}（${system}）：${mirror}，${audio}。`;
}

export function sessionStatus(session: MirrorSession): string | null {
  switch (session.phase) {
    case "idle":
      return null;
    case "connecting":
      return "正在启动镜像窗口…";
    case "streaming":
      return session.first_frame === "reached"
        ? "镜像正在运行。关闭镜像窗口即可结束本次会话。"
        : "画面正在启动…若几秒后仍未出现，请检查手机屏幕是否亮起并确认授权。";
    case "unauthorized":
      return sessionErrorText(session, "手机尚未允许这台电脑进行调试，请解锁手机后重新允许。");
    case "offline":
      return sessionErrorText(session, "手机当前处于离线状态，请重新插拔数据线或重新连接无线调试。");
    case "paired":
      return "无线设备已配对并连接，可以开始镜像。";
    case "failed":
      return sessionErrorText(session, "镜像会话失败，请重新检查连接后再试。");
  }
}

const stateCopy: Record<DeviceState, { label: string; detail: string }> = {
  ready: { label: "可以开始镜像", detail: "手机已授权这台电脑。" },
  unauthorized: {
    label: "等待手机确认",
    detail: "请解锁手机，然后在“允许 USB 调试吗？”提示中选择允许。",
  },
  offline: {
    label: "连接暂不可用",
    detail: "请拔下数据线后重新连接，保持手机解锁。",
  },
  unknown: {
    label: "需要检查连接",
    detail: "请检查数据线和手机上的 USB 连接模式。",
  },
};

// 左侧导航（参照 ToDesk / UU 远程的分栏布局）。每个面板都常驻渲染、由 CSS
// 控制显隐：切换导航不销毁任何状态（输入框内容、上传结果都不丢）。
type TabKey = "home" | "tools" | "wireless" | "settings" | "help";
const navItems: { key: TabKey; label: string; icon: ReactElement }[] = [
  { key: "home", label: "连接", icon: <NavIcon d="M3.5 5.5A2 2 0 0 1 5.5 3.5h13a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2v-9Zm5 13h7" /> },
  { key: "tools", label: "工具", icon: <NavIcon d="M4 20h16M6 20V9.5L12 4l6 5.5V20M10 20v-5h4v5" /> },
  { key: "wireless", label: "无线", icon: <NavIcon d="M12 19.5h.01M8.5 15.5a5 5 0 0 1 7 0M5 11.5a10 10 0 0 1 14 0M2 7.5a14.5 14.5 0 0 1 20 0" /> },
  { key: "settings", label: "设置", icon: <NavIcon d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm7-3a7 7 0 0 0-.1-1.2l2-1.5-2-3.5-2.4 1a7 7 0 0 0-2-1.2L14 3h-4l-.5 2.6a7 7 0 0 0-2 1.2l-2.4-1-2 3.5 2 1.5A7 7 0 0 0 5 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.5 2.4-1a7 7 0 0 0 2 1.2L10 21h4l.5-2.6a7 7 0 0 0 2-1.2l2.4 1 2-3.5-2-1.5c.07-.4.1-.8.1-1.2Z" /> },
  { key: "help", label: "帮助", icon: <NavIcon d="M12 17h.01M9.1 9a3 3 0 0 1 5.8 1c0 2-3 2.5-3 4M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z" /> },
];

function NavIcon({ d }: { d: string }) {
  return (
    <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={d} />
    </svg>
  );
}

export type TransferGroup = { kind: string; label: string; files: string[] };

/**
 * 按用途把手机发送区的文件分组（X10-75）。
 *
 * 面向的是不懂技术的人：`.apk` 是「装到手机的软件包」，常见文档格式单列，
 * 其余归「其他文件」。分组的意义是让人一眼扫过去就知道"哪些是安装包、哪些是
 * 要看的文档"，而不是面对一列同质的文件名。
 *
 * 纯函数：分组顺序固定（安装包 → 文档 → 其他），组内按名称自然排序，
 * 这样两次刷新之间行不会跳动。空组不返回。
 */
export function groupTransferFiles(names: string[]): TransferGroup[] {
  // 分类要认得 Android 传输产生的重名副本：`foo.apk.1`、`foo.apk (1).1`。
  // 这些本质仍是安装包（用户从微信/QQ 传过来的），若按严格结尾判断会被甩进
  // 「其他文件」，用户一眼扫不出哪些是安装包 —— 那就白分组了。
  const INSTALLER = /\.(apk|apks|xapk)\b/i;
  const DOCUMENT = /\.(pdf|docx?|xlsx?|pptx?|txt|md|csv|rtf|epub|json|log)\b/i;
  const buckets: Record<string, string[]> = { installer: [], document: [], other: [] };
  for (const name of names) {
    if (INSTALLER.test(name)) buckets.installer.push(name);
    else if (DOCUMENT.test(name)) buckets.document.push(name);
    else buckets.other.push(name);
  }
  const byName = (a: string, b: string) => a.localeCompare(b, "zh-Hans-CN");
  return [
    { kind: "installer", label: "安装包", files: buckets.installer.sort(byName) },
    { kind: "document", label: "文档", files: buckets.document.sort(byName) },
    { kind: "other", label: "其他文件", files: buckets.other.sort(byName) },
  ].filter((group) => group.files.length > 0);
}

function App() {
  const [tab, setTab] = useState<TabKey>("home");
  const [check, setCheck] = useState<AdbCheck | null>(null);
  const [isChecking, setIsChecking] = useState(true);
  const [isLaunching, setIsLaunching] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [launchError, setLaunchError] = useState<string | null>(null);
  const [wirelessExpanded, setWirelessExpanded] = useState(false);
  const [pairEndpoint, setPairEndpoint] = useState("");
  const [connectEndpoint, setConnectEndpoint] = useState("");
  const [pairingCode, setPairingCode] = useState("");
  const [wirelessMessage, setWirelessMessage] = useState<string | null>(null);
  const [wirelessBusy, setWirelessBusy] = useState(false);
  // 开发者中心自动发现（mDNS）：自动填地址，配对码仍由用户从手机屏幕读取。
  const [discovering, setDiscovering] = useState(false);
  const [discoverMessage, setDiscoverMessage] = useState<string | null>(null);
  // 扫码配对：桌面出码，手机原生「使用二维码配对设备」扫码后自动完成配对连接。
  const [qrOffer, setQrOffer] = useState<QrPairingOffer | null>(null);
  const [qrImage, setQrImage] = useState<string | null>(null);
  const [qrProgress, setQrProgress] = useState<QrPairingProgress | null>(null);
  const [qrBusy, setQrBusy] = useState(false);
  const [qrError, setQrError] = useState<string | null>(null);
  // 全局快捷键组合：用户可在设置页修改，本机持久化。
  const [shortcuts, setShortcuts] = useState<ShortcutSettings>(readShortcuts);
  const [trustedDevices, setTrustedDevices] = useState<TrustedWirelessDevice[]>([]);
  const [recentMessage, setRecentMessage] = useState<string | null>(null);
  const [selectedSerial, setSelectedSerial] = useState<string | null>(null);
  const [capabilities, setCapabilities] = useState<DeviceCapabilities | null>(null);
  // 锁屏状态按设备记录：多台并发时每张设备卡都要显示各自的锁屏便签（X10-33）。
  const [lockReports, setLockReports] = useState<Record<string, DeviceLockReport>>({});
  const [lockError, setLockError] = useState<string | null>(null);
  const [lockBusy, setLockBusy] = useState(false);
  const [screenshot, setScreenshot] = useState<Screenshot | null>(null);
  const [screenshotError, setScreenshotError] = useState<string | null>(null);
  const [screenshotBusy, setScreenshotBusy] = useState(false);
  const [recording, setRecording] = useState<Recording | null>(null);
  const [recordingError, setRecordingError] = useState<string | null>(null);
  const [recordingBusy, setRecordingBusy] = useState(false);
  // 文件传输：发送与取回共用一处状态。设备文件列表按需加载，不随会话轮询。
  const [transferBusy, setTransferBusy] = useState(false);
  const [transferMessage, setTransferMessage] = useState<string | null>(null);
  const [transferError, setTransferError] = useState<string | null>(null);
  // X10-75：取回状态按**文件**记录，不再只有一个 lastTransfer。
  // 旧实现只有一个 lastTransfer，导致：①同时/连续取回多个文件时只认最后一次，
  // 界面却一直显示同一个「取回」按钮，用户不知道哪个已经取回；②底部提示
  // 「已保存到这台电脑的下载/MirrorDock 文件夹」会让人以为**全部**都保存了。
  // 现在每个文件有自己的状态，操作列据此切换成「已取回 ✓」而不是重复的按钮。
  const [fetchedFiles, setFetchedFiles] = useState<Record<string, TransferReceipt>>({});
  // 正在取回的文件名（用于行内 loading，只让该行转圈而不是整表转圈）。
  const [fetchingName, setFetchingName] = useState<string | null>(null);
  // X10-75（impeccable 评审）：失败必须是**行内**状态，不能只是面板底部一行红字 ——
  // 8 行列表里那行字扫不出来，而且失败后没有恢复动作会让人卡死。
  const [fetchErrors, setFetchErrors] = useState<Record<string, string>>({});
  // 安装 APK：选中的安装包路径只存在内存里（不写 localStorage，不入日志）。
  const [apkPath, setApkPath] = useState<string | null>(null);
  const [apkBusy, setApkBusy] = useState(false);
  const [apkReceipt, setApkReceipt] = useState<ApkInstallReceipt | null>(null);
  const [apkMessage, setApkMessage] = useState<string | null>(null);
  const [apkError, setApkError] = useState<string | null>(null);
  // 诊断包：默认不生成、不落盘；预览后由用户显式导出。
  const [diagnostics, setDiagnostics] = useState<DiagnosticsPreviewData | null>(null);
  const [diagnosticsBusy, setDiagnosticsBusy] = useState(false);
  const [diagnosticsMessage, setDiagnosticsMessage] = useState<string | null>(null);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const [deviceFiles, setDeviceFiles] = useState<string[] | null>(null);
  // X10-60：伴侣端推送「发送区有新文件」的时间戳（null = 无新事件）。
  const [filesChangedHint, setFilesChangedHint] = useState<number | null>(null);
  // X10-60：伴侣端报来的崩溃堆栈（仅内存展示，不落盘、不上传；点「知道了」即散）。
  const [companionCrash, setCompanionCrash] = useState<string | null>(null);
  const [companionCrashExpanded, setCompanionCrashExpanded] = useState(false);
  // X10-66 通知镜像：手机转来的通知（最新在前，最多 50 条，仅内存）。
  const [phoneNotifications, setPhoneNotifications] = useState<PhoneNotification[]>([]);
  // X10-69 快捷回复：每条通知的草稿与发送状态（按通知 key 索引，仅内存）。
  const [notifReplyDrafts, setNotifReplyDrafts] = useState<Record<string, string>>({});
  const [notifReplyBusy, setNotifReplyBusy] = useState<string | null>(null);
  const [notifReplyNotice, setNotifReplyNotice] = useState<{ key: string; text: string; error: boolean } | null>(null);

  // 发送快捷回复：经后端命令下发到手机伴侣端（常驻通道），由它填进原通知。
  async function sendNotificationReply(item: PhoneNotification & { key: string }) {
    const text = (notifReplyDrafts[item.key] ?? "").trim();
    if (!text || notifReplyBusy) return;
    setNotifReplyBusy(item.key);
    setNotifReplyNotice(null);
    try {
      const sent = await invoke<boolean>("companion_notification_reply", { key: item.key, text });
      if (sent) {
        setNotifReplyDrafts((prev) => {
          const next = { ...prev };
          delete next[item.key];
          return next;
        });
        setNotifReplyNotice({ key: item.key, text: "回复已发送到手机。", error: false });
      } else {
        setNotifReplyNotice({ key: item.key, text: "手机端连接不在线（需要手机伴侣 App 的常驻连接保持连接），稍后再试。", error: true });
      }
    } catch (error) {
      setNotifReplyNotice({ key: item.key, text: errorMessage(error, "回复发送失败，请稍后再试。"), error: true });
    } finally {
      setNotifReplyBusy(null);
    }
  }
  // 版本与授权：读取失败时按「版本未知」呈现，不阻断镜像主流程。
  const [entitlement, setEntitlement] = useState<EntitlementView | null>(null);
  const [licenseInput, setLicenseInput] = useState("");
  const [licenseBusy, setLicenseBusy] = useState(false);
  const [licenseMessage, setLicenseMessage] = useState<string | null>(null);
  const [licenseError, setLicenseError] = useState<string | null>(null);
  // 品牌引导：用户手动选择优先于按设备 label 自动猜测；null 表示尚未选择。
  const [guideKey, setGuideKey] = useState<string | null>(null);
  // 伴侣 App 配对（POC）：仅在前端展示，token 不写入任何持久化记录。
  const [pairingOffer, setPairingOffer] = useState<PairingOffer | null>(null);
  const [pairingStatus, setPairingStatus] = useState<PairingStatus | null>(null);
  // M4-2 常驻通道：本地开关镜像（真实状态以后端 status().resident 为准）。
  const [residentActive, setResidentActive] = useState(false);
  const [residentPort, setResidentPort] = useState<number | null>(null);
  const [residentBusy, setResidentBusy] = useState(false);
  const [residentMessage, setResidentMessage] = useState<string | null>(null);
  // 开启成功后的如实提醒（X10-64）：端口只在扫码那一刻交给手机，已断开的手机
  // 收不到更新——这类提示不是错误，用独立状态、与失败消息分开呈现。
  const [residentNotice, setResidentNotice] = useState<string | null>(null);
  const [pairingQr, setPairingQr] = useState<string | null>(null);
  const [pairingBusy, setPairingBusy] = useState(false);
  const [pairingError, setPairingError] = useState<string | null>(null);
  const [pairedDevices, setPairedDevices] = useState<PairedCompanion[]>([]);
  const [sessions, setSessions] = useState<MirrorSession[]>([]);
  const [sessionError, setSessionError] = useState<string | null>(null);
  const [applyingOptions, setApplyingOptions] = useState(false);
  const [applyNotice, setApplyNotice] = useState<string | null>(null);
  // X10-27 里程碑 2：多会话时设置可选择应用到哪台设备；null = 主会话（单会话默认）。
  const [settingsTargetSerial, setSettingsTargetSerial] = useState<string | null>(null);
  // X10-68 设备收藏：置顶显示，持久化到本机（与最近记录同生命周期，本机存储）。
  const [favoriteDevices, setFavoriteDevices] = useState<string[]>(() => {
    try {
      const raw = JSON.parse(localStorage.getItem("mirrordock.favoriteDevices") ?? "[]");
      return Array.isArray(raw) ? raw.filter((item): item is string => typeof item === "string") : [];
    } catch {
      return [];
    }
  });
  // X10-88：设备自定义备注（physical_serial → 备注名）。避免同型号手机显示一样。
  const [deviceNicknames, setDeviceNicknames] = useState<Record<string, string>>(() => readDeviceNicknames());
  // 正在编辑备注的设备键 + 草稿；null 表示没有打开编辑框。
  const [nicknameEditing, setNicknameEditing] = useState<string | null>(null);
  const [nicknameDraft, setNicknameDraft] = useState("");
  // 按设备结束镜像（X10-27 复测反馈）：记录正在结束的会话归属，
  // 让每台设备自己的「结束」按钮显示各自的进行中状态，而不是全局一把抓。
  const [stoppingSerial, setStoppingSerial] = useState<string | null>(null);
  // 取消授权的两段式确认：第一次点击进入待确认状态，再点才真正执行（X10-32）。
  const [revokeArmedSerial, setRevokeArmedSerial] = useState<string | null>(null);
  const [revoking, setRevoking] = useState(false);
  const [revokeNotice, setRevokeNotice] = useState<{ text: string; error: boolean } | null>(null);
  // 「关于」板块的应用版本（tauri.conf.json 的 version，随包分发）。
  const [appVersion, setAppVersion] = useState<string>("");
  // macOS 宿主输入源被自动托管时的提示（X10-39）：第三方输入法会吞掉镜像输入
  // 的原始键码，后端已临时切到系统输入源，会话结束后自动恢复。
  const [hostImNotice, setHostImNotice] = useState<string | null>(null);
  const [reconnectNotice, setReconnectNotice] = useState<string | null>(null);
  // 取消授权成功后立即把这台设备从列表隐藏（X10-33）：调试开关已关，adb 列表
  // 通常几秒内自己消失，隐藏让它不闪一下「离线」。设备重新就绪（重新授权）即自动恢复显示。
  // 隐藏清单持久化到本机存储（X10-35）：离线设备的取消授权指令往往送不到手机，
  // adb 列表里的残影不会自己消失，重启客户端也不能再冒出来。
  const [hiddenRevokedSerials, setHiddenRevokedSerials] = useState<string[]>(() => {
    try {
      const raw = JSON.parse(localStorage.getItem("mirrordock.revokedSerials") ?? "[]");
      return Array.isArray(raw) ? raw.filter((item): item is string => typeof item === "string") : [];
    } catch {
      return [];
    }
  });
  const hideRevokedSerial = (serial: string) => {
    setHiddenRevokedSerials((prev) => {
      if (prev.includes(serial)) return prev;
      const next = [...prev, serial];
      try { localStorage.setItem("mirrordock.revokedSerials", JSON.stringify(next)); } catch { /* 存储不可用时仅本次会话生效 */ }
      return next;
    });
  };
  // 启动中的会话归属：多张设备卡同时可见时，「正在启动…」只出现在点下的那张卡上。
  const [launchingSerial, setLaunchingSerial] = useState<string | null>(null);
  // 拖拽安装 APK：拖入时显示全屏提示；安装结果以右下角浮层反馈（任何页签可见）。
  const [apkDragOver, setApkDragOver] = useState(false);
  const [dropInstallNotice, setDropInstallNotice] = useState<{ kind: "info" | "error"; text: string } | null>(null);
  // X10-27 并发多设备：后端按设备维护会话表；这里保留「主会话」视图供既有
  // 单会话 UI（顶栏、状态提示、设置应用）使用，各设备自己的状态按 serial 查询。
  const session = sessions.find((item) => item.phase === "streaming" || item.phase === "connecting") ?? sessions[0] ?? null;
  const sessionActive = sessions.some((item) => item.phase === "connecting" || item.phase === "streaming");
  const sessionFor = (serial: string | null) =>
    serial ? sessions.find((item) => item.serial === serial) ?? null : null;
  const activeSessionCount = sessions.filter((item) => item.phase === "connecting" || item.phase === "streaming").length;
  // 有明确归属（serial 非空）的进行中会话：供「正在镜像的设备」逐台列出与单独结束。
  const activeSessionList = sessions.filter(
    (item): item is MirrorSession & { serial: string } =>
      (item.phase === "connecting" || item.phase === "streaming") && item.serial != null
  );
  // 授权状态读取失败（null）按免费版呈现：录制开关禁用并给出激活指引。
  const proEdition = isProEdition(entitlement?.edition);
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const current = await invoke<MirrorSession[]>("mirror_sessions");
        if (!disposed) { setSessions(current); setSessionError(null); }
      } catch { if (!disposed) setSessionError("无法更新会话状态，请重新打开应用后检查。"); }
      try {
        const captured = await invoke<Recording | null>("current_recording");
        if (!disposed) setRecording(captured);
      } catch { /* 录制信息是附加信息，读不到不影响会话状态本身的展示。 */ }
      if (!disposed) timer = setTimeout(() => void poll(), 1000);
    }
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
  }, []);
  const [options, setOptions] = useState<SessionOptions>(readOptions);
  const [settingsNotice, setSettingsNotice] = useState<string | null>(null);
  const [pinPadActive, setPinPadActive] = useState(false);
  const sessionSerial = session?.serial ?? null;
  // 密码页守护：会话进行中时周期探测「安全表面（密码输入页）」。
  // 真机定案：密码页是 Android 安全表面，截屏与镜像同时被拒（镜像黑屏）——
  // 不可绕过也不应绕过。检测到时在锁屏面板明确告知用户「请在手机上解锁」，
  // 密码页退出后提示自动消失。探测只读状态与截屏字节数，像素不落盘、不回传。
  useEffect(() => {
    if (!sessionActive || !sessionSerial) {
      setPinPadActive(false);
      return;
    }
    const serial = sessionSerial;
    let disposed = false;
    const timer = window.setInterval(async () => {
      try {
        const probe = await invoke<{ active: boolean }>("probe_pin_pad_state", { serial });
        if (!disposed) setPinPadActive(probe.active);
      } catch {
        // 读不到（设备离线等）保持现状，不闪烁。
      }
    }, 8000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [sessionActive, sessionSerial]);
  // 通用设置：开机自启（autostart 插件持久化）与 macOS 隐藏 Dock（后端持久化）。
  const [appSettings, setAppSettings] = useState<AppSettingsView>({ hide_dock_icon: false, auto_reconnect: true });
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [generalNotice, setGeneralNotice] = useState<string | null>(null);
  const isMac = isMacPlatform(navigator.userAgent);

  // 挂载时读取一次通用设置。两者都允许静默失败：读不到就用默认值，
  // 不该因为一个偏好设置读不出来而影响连接与镜像。
  useEffect(() => {
    let disposed = false;
    void (async () => {
      try {
        const settings = await invoke<Partial<AppSettingsView>>("get_app_settings");
        if (!disposed) setAppSettings(normalizeAppSettings(settings));
      } catch { if (!disposed) setAppSettings(normalizeAppSettings({})); }
      try {
        const enabled = await invoke<unknown>("plugin:autostart|is_enabled");
        if (!disposed) setAutostartEnabled(enabled === true);
      } catch { /* 非 Tauri 环境（浏览器/测试）没有该命令，保持关闭 */ }
      try {
        const version = await getVersion();
        if (!disposed) setAppVersion(version);
      } catch { /* 非 Tauri 环境读不到版本，留空即可 */ }
    })();
    return () => { disposed = true; };
  }, []);

  async function toggleAutostart(enabled: boolean) {
    const previous = autostartEnabled;
    setAutostartEnabled(enabled);
    setGeneralNotice(null);
    try {
      await invoke(enabled ? "plugin:autostart|enable" : "plugin:autostart|disable");
      setGeneralNotice(enabled ? "已开启开机自动启动。" : "已关闭开机自动启动。");
    } catch (error) {
      setAutostartEnabled(previous);
      setGeneralNotice(errorMessage(error, "无法修改开机自动启动。"));
    }
  }

  async function toggleHideDockIcon(hide: boolean) {
    const previous = appSettings;
    setAppSettings({ ...appSettings, hide_dock_icon: hide });
    setGeneralNotice(null);
    try {
      const saved = await invoke<Partial<AppSettingsView>>("set_app_settings", { settings: { ...appSettings, hide_dock_icon: hide } });
      setAppSettings(normalizeAppSettings(saved));
      setGeneralNotice(hide ? "已隐藏 Dock 图标，从屏幕顶部菜单栏图标使用 MirrorDock。" : "已恢复 Dock 图标。");
    } catch (error) {
      setAppSettings(previous);
      setGeneralNotice(errorMessage(error, "无法修改 Dock 图标设置。"));
    }
  }

  async function toggleAutoReconnect(enabled: boolean) {
    const previous = appSettings;
    setAppSettings({ ...appSettings, auto_reconnect: enabled });
    setGeneralNotice(null);
    try {
      const saved = await invoke<Partial<AppSettingsView>>("set_app_settings", { settings: { ...appSettings, auto_reconnect: enabled } });
      setAppSettings(normalizeAppSettings(saved));
      setGeneralNotice(enabled ? "已开启断线自动重连：无线掉线或数据线被拔掉后，会等待连接恢复并自动重建镜像（最多 15 分钟）。" : "已关闭自动重连：断开后需要手动重新连接。");
    } catch (error) {
      setAppSettings(previous);
      setGeneralNotice(errorMessage(error, "无法修改自动重连设置。"));
    }
  }

  // X10-95：选择录像保存文件夹。不选（取消）保持现状；选了立即持久化到后端。
  async function chooseRecordingDir() {
    setGeneralNotice(null);
    let picked: string | null = null;
    try {
      const result = await openFilePicker({ directory: true, multiple: false, title: "选择录像保存文件夹" });
      picked = typeof result === "string" ? result : null;
    } catch (error) {
      setGeneralNotice(errorMessage(error, "无法打开文件夹选择。"));
      return;
    }
    if (!picked) return; // 用户取消
    const previous = appSettings;
    setAppSettings({ ...appSettings, recording_dir: picked });
    try {
      const saved = await invoke<Partial<AppSettingsView>>("set_app_settings", { settings: { ...appSettings, recording_dir: picked } });
      setAppSettings(normalizeAppSettings(saved));
      setGeneralNotice(`录像将保存到：${picked}`);
    } catch (error) {
      setAppSettings(previous);
      setGeneralNotice(errorMessage(error, "无法使用这个录像保存位置。"));
    }
  }

  // 恢复默认录像目录（视频目录/MirrorDock）。
  async function resetRecordingDir() {
    const previous = appSettings;
    setAppSettings({ ...appSettings, recording_dir: null });
    setGeneralNotice(null);
    try {
      const saved = await invoke<Partial<AppSettingsView>>("set_app_settings", { settings: { ...appSettings, recording_dir: null } });
      setAppSettings(normalizeAppSettings(saved));
      setGeneralNotice("已恢复默认录像保存位置（本机视频目录下的 MirrorDock 文件夹）。");
    } catch (error) {
      setAppSettings(previous);
      setGeneralNotice(errorMessage(error, "无法恢复默认录像位置。"));
    }
  }
  // 检查更新（X10-47）：check → 发现新版则下载安装（验签由后端 updater 完成）→ 重启。
  const [updateState, setUpdateState] = useState<"idle" | "checking" | "installing">("idle");
  const [updateMessage, setUpdateMessage] = useState<string | null>(null);
  async function checkForUpdates() {
    setUpdateMessage(null);
    setUpdateState("checking");
    try {
      const update = await checkForUpdate();
      if (!update) {
        setUpdateMessage("当前已是最新版本。");
        setUpdateState("idle");
        return;
      }
      setUpdateState("installing");
      setUpdateMessage(`发现新版本 ${update.version}，正在下载并安装…`);
      await update.downloadAndInstall();
      setUpdateMessage("更新已安装，应用即将重启…");
      await invoke("restart_app");
    } catch (error) {
      setUpdateMessage(errorMessage(error, "检查或安装更新失败，请稍后再试，也可以到 GitHub 仓库的 Releases 页手动下载。"));
      setUpdateState("idle");
    }
  }

  // 在手机上打开「实体键盘」设置页（X10-46）：UHID 打字依赖手机端启用的布局。
  async function openKeyboardSettings() {    if (!readySerial) return;
    try {
      await invoke("open_keyboard_settings", { serial: readySerial });
      setSettingsNotice("已在手机上打开「实体键盘」设置：请确认 scrcpy 键盘已启用「英语（美国）」布局，镜像窗口里按 MOD+k 也能打开这个页面。");
    } catch (error) {
      setSettingsNotice(errorMessage(error, "无法打开手机的键盘设置（设备可能未连接）。"));
    }
  }

  // 桌面模式「虚拟屏启动的应用」候选（X10-53；X10-71 起带应用名并按设备缓存）：
  // 桌面模式相关设置出现在界面上时拉取一次。拉取失败不阻塞——用户仍可手动填包名。
  const [deviceApps, setDeviceApps] = useState<Record<string, DeviceApp[]>>({});
  const deviceAppsInflight = useRef<Set<string>>(new Set());
  async function ensureDeviceApps(serial?: string | null) {
    if (!serial || deviceApps[serial] || deviceAppsInflight.current.has(serial)) return;
    deviceAppsInflight.current.add(serial);
    try {
      const apps = await invoke<DeviceApp[]>("list_device_apps", { serial });
      setDeviceApps((prev) => ({ ...prev, [serial]: apps }));
    } catch {
      setDeviceApps((prev) => ({ ...prev, [serial]: [] }));
    } finally {
      deviceAppsInflight.current.delete(serial);
    }
  }

  // X10-71：按设备保存桌面模式偏好；只影响这台设备，其他设备与全局默认不动。
  // X10-90：数据源从前端 localStorage 迁到后端 app data（托盘等任意入口都能读），
  // localStorage 只作迁移来源，迁移成功后清除。
  const [desktopPrefs, setDesktopPrefs] = useState<Record<string, DesktopPref>>({});
  // X10-72：桌面模式模块的编辑目标独立于「镜像窗口」那张卡的目标，理由见
  // settingsTargetSerial 的重置 effect 注释——未插线的设备也要能提前配好。
  const [desktopTargetSerial, setDesktopTargetSerial] = useState<string | null>(null);
  function updateDesktopPref(serial: string, patch: Partial<DesktopPref>) {
    setDesktopPrefs((prev) => {
      const base = prev[serial] ?? { desktop_mode: options.desktop_mode, desktop_app: options.desktop_app };
      const next = { ...base, ...patch };
      const nextPrefs = { ...prev, [serial]: next };
      // X10-90：写后端持久化；失败如实提示，不假装保存成功。
      invoke("set_desktop_prefs", { prefs: nextPrefs }).catch((error: unknown) => {
        setSettingsNotice(errorMessage(error, "桌面模式配置无法保存，本次会话仍可使用。"));
      });
      return nextPrefs;
    });
  }

  // X10-90：挂载时把桌面模式偏好从 localStorage 迁到后端，再从后端拉取。
  // 一次性（空依赖）：后端是唯一持久层；迁移成功后清掉 localStorage，避免下次重复迁。
  // localStorage 里保留的数据结构 = Record<key, {desktop_mode, desktop_app}>（readDesktopPrefs
  // 已做校验），直接透传给后端即可；后端写盘时会再做一次键归一化。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const legacy = readDesktopPrefs();
      const hasLegacy = Object.keys(legacy).length > 0;
      if (hasLegacy) {
        try {
          await invoke("set_desktop_prefs", { prefs: legacy });
          try { localStorage.removeItem("mirrordock.desktopPrefs"); } catch { /* 存储不可用则下次再迁 */ }
        } catch (error) {
          // 迁移失败：本次仍用 localStorage 的值兜底，不清 localStorage（下次启动重迁）。
          if (!cancelled) {
            setDesktopPrefs(legacy);
            setSettingsNotice(errorMessage(error, "桌面模式配置迁移失败，本次沿用本机原有配置。"));
          }
          return;
        }
      }
      try {
        const prefs = await invoke<Record<string, DesktopPref>>("get_desktop_prefs");
        if (!cancelled) setDesktopPrefs(prefs);
      } catch (error) {
        if (!cancelled) {
          setSettingsNotice(errorMessage(error, "无法读取桌面模式配置。"));
        }
      }
    })();
    return () => { cancelled = true; };
  }, []);

  // X10-84 + X10-87：pref 键统一迁移到稳定 physical_serial。两个来源：
  //  ① 设备在线时反查：无线端点/adb serial → physical_serial（desktopPrefKeyFor）；
  //  ② 离线归一化：adb-<physical>-..._tcp 端点直接提取物理 id（normalizeDeviceIdentityKey）。
  // 同一物理设备的多个历史键（端点/ip/型号名）合并为一条，值保留「最近设置」——
  // 由于无法判断新旧，保留目标键已有值，仅清掉重复旧键，避免列表出现多行（用户实测
  // 一台设备显示 5 条桌面模式记录）。
  useEffect(() => {
    const devices = check?.devices;
    setDesktopPrefs((prev) => {
      let changed = false;
      const next: Record<string, DesktopPref> = { ...prev };
      for (const key of Object.keys(prev)) {
        // 设备在线时优先反查真实 physical；离线则用归一化规则。
        const stable = (devices && devices.length > 0)
          ? desktopPrefKeyFor(devices, key)
          : normalizeDeviceIdentityKey(key);
        if (stable && stable !== key) {
          if (!(stable in next)) next[stable] = prev[key];
          delete next[key];
          changed = true;
        }
      }
      if (changed) {
        // X10-90：键归一化后的整表回写后端。
        invoke("set_desktop_prefs", { prefs: next }).catch((error: unknown) => {
          setSettingsNotice(errorMessage(error, "桌面模式配置无法保存，本次会话仍可使用。"));
        });
      }
      return changed ? next : prev;
    });
    // 设备列表每次变化都重跑一次：设备上线时能反查出更多旧端点 → 继续归并。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [check?.devices]);

  // 恢复默认设置时，各设备的桌面模式偏好一并清空——「默认」应意味着全部回到出厂值。
  function resetAllOptions() {
    updateOptions(defaultOptions);
    setDesktopPrefs(() => {
      // X10-90：清空后端持久层（localStorage 已在迁移后清除，无需再动）。
      invoke("set_desktop_prefs", { prefs: {} }).catch((error: unknown) => {
        setSettingsNotice(errorMessage(error, "桌面模式配置无法清空，本次会话仍可使用默认设置。"));
      });
      return {};
    });
  }

  function updateOptions(next: SessionOptions) {
    setOptions(next);
    // 设置一旦被改动，上一次「已应用 / 无需重启」的结论就不再适用，先清掉避免误导。
    setApplyNotice(null);
    try { localStorage.setItem("mirrordock.sessionOptions", JSON.stringify(next)); }
    catch { setSettingsNotice("本机设置无法保存，本次会话仍可使用这些选项。"); }
  }

  // X10-68：收藏/取消收藏设备。只影响本机显示排序，不动连接与授权。
  function toggleFavoriteDevice(serial: string) {
    setFavoriteDevices((prev) => {
      const next = prev.includes(serial) ? prev.filter((item) => item !== serial) : [...prev, serial];
      try { localStorage.setItem("mirrordock.favoriteDevices", JSON.stringify(next)); }
      catch { /* 保存失败只影响下次启动时的置顶，本次会话内仍然生效。 */ }
      return next;
    });
  }

  // 目标设备的会话结束后，设置目标回退到主会话，避免打到已结束的设备上。
  // 注意：只回退「镜像窗口」那张卡的目标（settingsTargetSerial）。桌面模式按设备
  // 的选择走独立的 desktopTargetSerial —— 用户要能在设备没插上时提前配好它，
  // 若跟着会话结束一起被清掉，这个模块就又变成"只能边连边配"。
  useEffect(() => {
    if (settingsTargetSerial && !activeSessionList.some((item) => item.serial === settingsTargetSerial)) {
      setSettingsTargetSerial(null);
    }
  }, [activeSessionList, settingsTargetSerial]);

  // 会话进行中应用新设置：镜像窗口会按新参数重新打开，画面会短暂中断。
  async function applyOptionsUpdate(next: SessionOptions) {
    updateOptions(next);
    setApplyingOptions(true);
    setApplyNotice(null);
    const targetSerial = settingsTargetSerial ?? session?.serial ?? null;
    // X10-71：桌面模式开关与虚拟屏应用按设备叠加——改其他设置时不会把这台
    // 设备的桌面模式偏好冲掉，反之亦然。X10-84：键统一为 physical_serial。
    const prefKey = desktopPrefKeyFor(check?.devices, targetSerial);
    const effective = composeOptionsWithDesktop(next, prefKey ? desktopPrefs[prefKey] ?? null : null);
    try {
      const result = await invoke<SessionUpdate>("update_session_options", {
        options: effective,
        // 只有开启录制时才生成文件名；后端在未开启录制时会忽略它。
        recordFileName: effective.record ? recordingFileName(new Date()) : null,
        // X10-27：多会话时作用于选中的设备；未选择时作用于主会话（真正持有镜像进程的设备优先）。
        serial: targetSerial,
      });
      setSessions((prev) => {
        if (result.session.serial == null) return prev;
        const next2 = prev.filter((item) => item.serial !== result.session.serial);
        return [...next2, result.session].sort((a, b) => (a.serial ?? "").localeCompare(b.serial ?? ""));
      });
      const targetLabel = targetSerial
        ? check?.devices.find((device) => device.serial === targetSerial)?.label ?? targetSerial
        : null;
      const prefix = targetLabel ? `「${targetLabel}」` : "";
      setApplyNotice(
        result.applied
          ? `${prefix}新设置已生效：镜像窗口已按新设置重新打开。`
          : result.note ?? "设置与当前会话一致，未重启镜像窗口。",
      );
    } catch (error) {
      setApplyNotice(errorMessage(error, "无法应用新设置。"));
    } finally {
      setApplyingOptions(false);
    }
  }

  async function applySessionOptions() {
    await applyOptionsUpdate(options);
  }

  // 全局快捷键处理器需要读到最新的 options / 设备 / 会话状态；用 ref 镜像，
  // 避免注册进快捷键的闭包拿到过期值。
  const shortcutsRef = useRef({ options, readySerial: null as string | null, proEdition, sessionActive });
  shortcutsRef.current = { options, readySerial: null, proEdition, sessionActive };

  // 会话进行中的系统级快捷键：截图 / 录制开关 / 轮换显示方向，组合可在设置页修改。
  // 仅在真实 Tauri 环境注册；浏览器 / 测试环境没有 __TAURI_INTERNALS__，直接跳过。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    if (!sessionActive) return;
    let cancelled = false;
    const toggleRecord = () => {
      const current = shortcutsRef.current;
      if (!current.proEdition) {
        setApplyNotice("录制是专业版功能，激活后即可使用。");
        return;
      }
      // X10-92：快捷键直接触发双通道录制启停，不再改动 record 设置项、不重启显示窗口。
      if (current.sessionActive) void toggleRecording();
      else setApplyNotice("请先开始镜像，再使用录制快捷键。");
    };
    const rotate = () => {
      const current = shortcutsRef.current;
      const next = { ...current.options, rotation: (current.options.rotation + 90) % 360 };
      if (current.sessionActive) void applyOptionsUpdate(next);
      else updateOptions(next);
    };
    const actions: [string, () => void][] = [
      [shortcuts.screenshot, () => { const serial = shortcutsRef.current.readySerial; if (serial) void captureScreen(serial); }],
      [shortcuts.record, toggleRecord],
      [shortcuts.rotate, rotate],
    ];
    void (async () => {
      try {
        for (const [key, action] of actions) {
          if (cancelled) return;
          await register(key, (event) => {
            if (event.state === "Pressed") action();
          });
        }
      } catch {
        // 全局快捷键注册失败（如被系统占用）不阻断镜像；主窗口内的操作始终可用。
        setApplyNotice(null);
      }
    })();
    return () => { cancelled = true; void unregisterAll().catch(() => { /* 会话已结束时注销失败无需处理 */ }); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionActive, shortcuts]);

  async function refreshDevices() {
    setIsChecking(true);
    try {
      setCheck(await invoke<AdbCheck>("check_adb_devices"));
    } catch {
      setCheck({
        adb_available: false,
        scrcpy_available: false,
        devices: [],
        diagnostic: "无法读取连接状态。请关闭后重新打开 MirrorDock。",
      });
    } finally {
      setIsChecking(false);
    }
  }

  useEffect(() => {
    try { setSelectedSerial(localStorage.getItem("mirrordock.lastDeviceSerial")); } catch { /* Storage may be unavailable. */ }
    void refreshDevices();
    void refreshTrustedDevices();
    void refreshRecentDevices();
    void refreshEntitlement();
  }, []);

  // X10-84：设备列表由后端 watcher 事件驱动（devices-changed），不再前端高频轮询。
  // 后端每 2s 探一次、仅在设备集合变化时 emit；首次挂载仍主动拉一次保证首屏有数据。
  // 手动「重新检查」入口保留。adb 调用已在后端带超时兜底（X10-84），卡死不再拖死 UI。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    const pull = () => {
      invoke<AdbCheck>("check_adb_devices", { silent: true })
        .then((next) => { if (!disposed) setCheck(next); })
        .catch(() => { /* 失败保持上一次结果，不闪烁、不打断 */ });
    };
    pull(); // 首屏数据
    void listen("devices-changed", pull)
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);

  async function refreshEntitlement() {
    try {
      setEntitlement(await invoke<EntitlementView>("entitlement_status"));
      setLicenseError(null);
    } catch (error) {
      setEntitlement(null);
      setLicenseError(errorMessage(error, "无法读取授权状态，功能按免费版呈现。"));
    }
  }

  // 激活离线完成：许可证只发往本地后端验签，密钥原文不会被写入任何日志。
  async function activateLicense() {
    setLicenseBusy(true);
    setLicenseMessage(null);
    setLicenseError(null);
    try {
      const view = await invoke<EntitlementView>("entitlement_activate", { licenseKey: licenseInput.trim() });
      setEntitlement(view);
      setLicenseInput("");
      setLicenseMessage("专业版已激活：MP4 录制现已可用。授权保存在本机。");
    } catch (error) {
      setLicenseError(errorMessage(error, "激活失败，请检查许可证后重试。"));
    } finally {
      setLicenseBusy(false);
    }
  }

  // 撤销只删除本机的授权状态文件，不影响许可证本身（可重新激活同一密钥）。
  async function deactivateLicense() {
    setLicenseBusy(true);
    setLicenseMessage(null);
    setLicenseError(null);
    try {
      const view = await invoke<EntitlementView>("entitlement_deactivate");
      setEntitlement(view);
      setLicenseMessage("已撤销本机的专业版授权。录制功能回到免费版状态。");
    } catch (error) {
      setLicenseError(errorMessage(error, "撤销授权失败，请重试。"));
    } finally {
      setLicenseBusy(false);
    }
  }

  // X10-85：首页不再展示最近设备列表，但数据仍在后端记录（供无线重连）。
  // refreshRecentDevices 仅用于启动镜像后让后端完成记录写入，UI 不消费返回值。
  async function refreshRecentDevices() {
    try {
      await invoke<RecentDevice[]>("list_recent_devices");
    } catch (error) {
      setRecentMessage(errorMessage(error, "无法读取本机最近使用的设备记录。"));
    }
  }

  // 客户端侧取消授权（X10-32）：关手机调试开关 + 断开全部无线连接 + 清本机记录，
  // 并打开手机的开发者选项引导本人完成「撤销 USB 调试授权」（清除授权记录
  // 只能手机端做，这是 Android 的安全设计）。成功后这台设备立即从列表移除（X10-33）。
  async function revokeDeviceAccess(serial: string) {
    setRevoking(true);
    setRevokeArmedSerial(null);
    setRevokeNotice(null);
    try {
      const receipt = await invoke<{ steps: string[] }>("revoke_device_access", { serial });
      setRevokeNotice({ text: receipt.steps.join(" "), error: false });
      hideRevokedSerial(serial);
      await refreshDevices();
      await refreshRecentDevices();
    } catch (error) {
      setRevokeNotice({ text: errorMessage(error, "无法完成取消授权，请稍后重试。"), error: true });
    } finally {
      setRevoking(false);
    }
  }

  // 无线设备的连接端口每次重新开启无线调试都可能变化，因此这里不承诺一定能连上。
  async function reconnectRecentDevice(endpoint: string) {
    setRecentMessage(null);
    try {
      await invoke("connect_wireless_device", { endpoint });
      setRecentMessage("已发送连接请求。若手机重启过无线调试，端口可能已变化，需重新配对。");
      await refreshDevices();
    } catch (error) {
      setRecentMessage(errorMessage(error, "这条记录无法直接连接，请在「无线」页重新配对。"));
    }
  }

  async function refreshTrustedDevices() {
    try {
      setTrustedDevices(await invoke<TrustedWirelessDevice[]>("list_trusted_wireless_devices"));
    } catch (error) {
      setTrustedDevices([]);
      setWirelessMessage(errorMessage(error, "无法读取本机已保存的无线设备列表。"));
    }
  }

  async function startMirroring(serial: string) {
    setSelectedSerial(serial);
    setIsLaunching(true);
    setLaunchingSerial(serial);
    setLaunchError(null);
    setApplyNotice(null);
    try {
      // X10-71：这台设备自己的桌面模式偏好（若有）叠加在全局设置之上。
      // X10-84：pref 键统一为 physical_serial——serial 是无线端点会变，直接当键会查空。
      const prefKey = desktopPrefKeyFor(check?.devices, serial);
      const effective = composeOptionsWithDesktop(options, prefKey ? desktopPrefs[prefKey] ?? null : null);
      await invoke("start_mirroring", {
        serial,
        options: effective,
        // 只有开启录制时才生成文件名；后端在未开启录制时会忽略它。
        recordFileName: effective.record ? recordingFileName(new Date()) : null,
      });
      try { localStorage.setItem("mirrordock.lastDeviceSerial", serial); } catch { setSettingsNotice("无法保存最近设备，本次连接不受影响。"); }
      // 启动成功后端才记入最近设备，这里同步刷新以反映新的排序。
      void refreshRecentDevices();
    } catch (error) {
      setLaunchError(errorMessage(error, "无法启动镜像窗口。请重新检查连接后再试。"));
    } finally {
      setIsLaunching(false);
      setLaunchingSerial(null);
    }
  }

  // 结束镜像：默认结束全部会话；传入 serial 时只结束该设备的会话（X10-27）。
  // stoppingSerial 让「这台设备的结束按钮」单独显示进行中，其它设备不受影响。
  async function stopMirroring(serial?: string) {
    setIsStopping(true);
    setStoppingSerial(serial ?? null);
    setLaunchError(null);
    try {
      await invoke("stop_mirroring", { serial: serial ?? null });
      // 会话已结束，上一次「新设置已生效」的提示不再有意义。
      setApplyNotice(null);
    } catch (error) {
      setLaunchError(errorMessage(error, "无法结束镜像会话，请手动关闭镜像窗口。"));
    } finally {
      setIsStopping(false);
      setStoppingSerial(null);
    }
  }

  // 开发者中心自动发现：手机无线调试的配对页 / 主页会通过 mDNS 广播地址。
  // Android 不提供配对二维码，6 位配对码仍需用户从手机屏幕读取后手动填写。
  async function discoverWirelessEndpoints() {
    setDiscovering(true);
    setDiscoverMessage(null);
    try {
      const found = await invoke<WirelessServices>("discover_pairing_services");
      const filled: string[] = [];
      if (found.pairing.length > 0) { setPairEndpoint(found.pairing[0]); filled.push("配对地址"); }
      if (found.connect.length > 0) { setConnectEndpoint(found.connect[0]); filled.push("连接地址"); }
      if (filled.length === 0) {
        setDiscoverMessage("未发现服务。让手机停在「使用配对码配对设备」或无线调试主页面再试。");
      } else {
        setDiscoverMessage(`已自动填入${filled.join("与")}，配对码以手机屏幕为准。`);
      }
    } catch (error) {
      setDiscoverMessage(errorMessage(error, "自动发现不可用。"));
    } finally {
      setDiscovering(false);
    }
  }

  async function refreshLockReport(serial: string) {
    try {
      const report = await invoke<DeviceLockReport>("device_lock_report", { serial });
      setLockReports((prev) => ({ ...prev, [serial]: report }));
    } catch {
      // 锁屏便签读不到就不显示该设备的便签，不打断其它状态展示。
    }
  }

  // 只点亮屏幕：不解锁、不输入任何凭据。安全锁屏仍需你本人在手机上解锁。
  async function wakeDevice(serial: string) {
    setLockBusy(true);
    setLockError(null);
    try {
      await invoke("wake_device", { serial });
      await refreshLockReport(serial);
    } catch (error) {
      setLockError(errorMessage(error, "无法点亮手机屏幕。"));
    } finally {
      setLockBusy(false);
    }
  }

  // 截图：把手机当前画面保存到这台电脑的「图片 / MirrorDock」。
  // 受保护页面（支付、密码输入等）由 Android 自行屏蔽，截出来是黑屏——这是系统行为，
  // 不是故障，MirrorDock 也不会尝试绕过它。
  async function captureScreen(serial: string) {
    setScreenshotBusy(true);
    setScreenshotError(null);
    try {
      const saved = await invoke<Screenshot>("capture_screenshot", {
        serial,
        fileName: screenshotFileName(new Date()),
      });
      setScreenshot(saved);
    } catch (error) {
      setScreenshot(null);
      setScreenshotError(errorMessage(error, "无法保存截图。"));
    } finally {
      setScreenshotBusy(false);
    }
  }

  // 撤销：删除刚刚保存的那张截图。只删这一个文件，不动其它内容。
  async function undoCapture() {
    if (!screenshot) return;
    setScreenshotBusy(true);
    setScreenshotError(null);
    try {
      await invoke("delete_screenshot", { fileName: screenshot.file_name });
      setScreenshot(null);
    } catch (error) {
      setScreenshotError(errorMessage(error, "无法删除这张截图。"));
    } finally {
      setScreenshotBusy(false);
    }
  }

  async function revealCapture() {
    if (!screenshot) return;
    setScreenshotError(null);
    try {
      await revealItemInDir(screenshot.path);
    } catch (error) {
      setScreenshotError(errorMessage(error, "无法打开截图所在的文件夹。"));
    }
  }

  // X10-92（方案甲）：开始/结束录制。走独立录制通道（start_recording/stop_recording），
  // 显示窗口全程不重启。何时录、录多久完全由用户决定——这与设置项「启用屏幕录制功能」
  // （仅作能力开关）解耦。文件名沿用既有时间戳格式，与旧行为一致。
  async function toggleRecording() {
    if (!proEdition) {
      setRecordingError("录制是专业版功能，激活后即可使用。");
      return;
    }
    setRecordingBusy(true);
    setRecordingError(null);
    try {
      if (recording?.active) {
        const stopped = await invoke<Recording | null>("stop_recording", {});
        if (stopped) setRecording(stopped);
        else setRecording(null);
      } else {
        const stamp = new Date().toISOString().replace(/[-:T]/g, "").slice(0, 14);
        const started = await invoke<Recording>("start_recording", {
          fileName: `mirrordock-record-${stamp}.mp4`,
        });
        setRecording(started);
      }
    } catch (error) {
      setRecordingError(errorMessage(error, recording?.active ? "无法结束录制。" : "无法开始录制。"));
    } finally {
      setRecordingBusy(false);
    }
  }

  // 撤销：删除这段录像。正在录制中的文件会被后端拒绝，界面如实展示原因。
  async function removeRecording() {
    if (!recording) return;
    setRecordingBusy(true);
    setRecordingError(null);
    try {
      await invoke("delete_recording", { fileName: recording.file_name });
      setRecording(null);
    } catch (error) {
      setRecordingError(errorMessage(error, "无法删除这段录像。"));
    } finally {
      setRecordingBusy(false);
    }
  }

  async function revealRecording() {
    if (!recording) return;
    setRecordingError(null);
    try {
      await revealItemInDir(recording.path);
    } catch (error) {
      setRecordingError(errorMessage(error, "无法打开录像所在的文件夹。"));
    }
  }

  // 发送文件：由用户通过系统文件选择器明确挑选，MirrorDock 不替用户选。
  // 文件内容直接经 adb 传到手机，不经过前端、不入日志。
  async function previewDiagnostics() {
    setDiagnosticsBusy(true);
    setDiagnosticsError(null);
    setDiagnosticsMessage(null);
    try {
      setDiagnostics(await invoke<DiagnosticsPreviewData>("diagnostics_preview"));
    } catch (error) {
      setDiagnosticsError(errorMessage(error, "无法生成诊断预览。"));
    } finally {
      setDiagnosticsBusy(false);
    }
  }

  async function exportDiagnostics() {
    if (!diagnostics) return;
    setDiagnosticsBusy(true);
    setDiagnosticsError(null);
    setDiagnosticsMessage(null);
    try {
      // 系统保存对话框：取消选择不算错误。
      const path = await saveFilePicker({
        defaultPath: `MirrorDock-诊断-${localTimestamp(new Date())}.json`,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      const receipt = await invoke<DiagnosticsReceipt>("export_diagnostics", { path });
      setDiagnosticsMessage(`诊断包已保存：${receipt.path}（${receipt.events} 条事件，${formatBytes(receipt.bytes)}）`);
    } catch (error) {
      setDiagnosticsError(errorMessage(error, "诊断包导出失败。"));
    } finally {
      setDiagnosticsBusy(false);
    }
  }

  // 伴侣 App 配对：开始后每 2 秒轮询状态展示事件流；二维码只在前端内存生成。
  async function beginPairing() {
    setPairingBusy(true);
    setPairingError(null);
    setPairingQr(null);
    try {
      const offer = await invoke<PairingOffer>("companion_begin_pairing");
      setPairingOffer(offer);
      void loadPairedDevices();
      setPairingStatus({ phase: "listening", resident: false, events: [], offer });
      try {
        setPairingQr(await QRCode.toDataURL(pairingPayload(offer), { width: 220, margin: 1 }));
      } catch {
        // 渲染环境不支持 canvas 时退化为只显示配对码，可手动输入。
      }
    } catch (error) {
      setPairingError(errorMessage(error, "无法开始配对。"));
    } finally {
      setPairingBusy(false);
    }
  }

  async function stopPairing() {
    setPairingBusy(true);
    try {
      await invoke("companion_end_pairing");
      setPairingOffer(null);
      setPairingStatus(null);
      setPairingQr(null);
    } catch (error) {
      setPairingError(errorMessage(error, "结束配对失败。"));
    } finally {
      setPairingBusy(false);
    }
  }

  // 已配对设备列表：进页面加载一次；配对/移除后手动刷新。
  async function loadPairedDevices() {
    try {
      setPairedDevices(await invoke<PairedCompanion[]>("companion_paired_devices"));
    } catch { /* 台账读不到就保持现有显示，不挡主流程。 */ }
  }

  // M4-2 常驻通道开关。开启后伴侣 App 里「连接上次配对的电脑」可免扫码直连。
  async function startResident() {
    setResidentBusy(true);
    setResidentMessage(null);
    setResidentNotice(null);
    try {
      const port = await invoke<number>("companion_begin_resident");
      setResidentActive(true);
      setResidentPort(port);
      // 如实告知边界（X10-64）：端口只在扫码配对的那一刻交给手机，桌面端无法
      // 主动通知已经断开的手机。此前在关闭状态下配对过的设备必须重新扫一次码，
      // 否则「连接上次配对的电脑」会一直显示没有可直连的电脑。
      setResidentNotice(
        `常驻通道已开启（端口 ${port}）。端口在扫码配对时才会交给手机：手机上若提示「没有可直连的电脑」，请在手机上重新扫一次码。`
      );
    } catch (error) {
      setResidentMessage(errorMessage(error, "常驻通道开启失败。"));
    } finally {
      setResidentBusy(false);
    }
  }

  async function stopResident() {
    setResidentBusy(true);
    setResidentMessage(null);
    setResidentNotice(null);
    try {
      await invoke("companion_end_resident");
      setResidentActive(false);
      setResidentPort(null);
    } catch (error) {
      setResidentMessage(errorMessage(error, "常驻通道关闭失败。"));
    } finally {
      setResidentBusy(false);
    }
  }

  // 挂载时同步一次常驻状态（应用重启后仍显示真实的监听状态）。
  useEffect(() => {
    invoke<PairingStatus>("companion_pairing_status")
      .then((status) => setResidentActive(status.resident === true))
      .catch(() => { /* 状态读不到保持默认，不打断页面。 */ });
    return () => {};
  }, []);

  async function unpairDevice(pairingId: string) {
    try {
      setPairedDevices(await invoke<PairedCompanion[]>("companion_unpair_device", { pairingId }));
    } catch (error) {
      setPairingError(errorMessage(error, "移除配对设备失败。"));
    }
  }

  useEffect(() => {
    void loadPairedDevices();
    return () => {};
  }, []);

  useEffect(() => {
    if (!pairingOffer && !residentActive) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const status = await invoke<PairingStatus>("companion_pairing_status");
        if (!disposed) setPairingStatus(status);
      } catch { /* 配对状态是附加信息，读不到保持上次结果。 */ }
      if (!disposed) timer = setTimeout(() => void poll(), 2000);
    }
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
  }, [pairingOffer, residentActive]);

  async function sendFileTo(serial: string) {
    setTransferMessage(null);
    setTransferError(null);
    let picked: string | string[] | null;
    try {
      picked = await openFilePicker({ multiple: false, title: "选择要发送到手机的文件" });
    } catch (error) {
      setTransferError(errorMessage(error, "无法打开文件选择器。"));
      return;
    }
    // 用户取消选择不算错误，界面回到原样即可。
    if (typeof picked !== "string" || picked.length === 0) return;
    setTransferBusy(true);
    try {
      const receipt = await invoke<TransferReceipt>("send_file_to_device", { serial, localPath: picked });
      // X10-75：说清是**哪个文件**发到了哪里，不写"已发送"这种含糊的话。
      setTransferMessage(`已把「${receipt.file_name}」发送到手机（手机「下载 / MirrorDock」）。`);
      // 发送成功后设备目录内容已变化，让下一次列表请求重新拉取。
      setDeviceFiles(null);
    } catch (error) {
      setTransferError(errorMessage(error, "文件没有传到手机上。"));
    } finally {
      setTransferBusy(false);
    }
  }

  async function refreshDeviceFiles(serial: string) {
    setTransferBusy(true);
    setTransferError(null);
    try {
      setDeviceFiles(await invoke<string[]>("list_device_files", { serial }));
    } catch (error) {
      setTransferError(errorMessage(error, "无法读取手机上的文件列表。"));
      // 读取失败时不要留下"上次的结果"冒充当前状态——那会让用户以为
      // 手机上的文件还在。置空让它回到"尚未读取"的诚实状态。
      setDeviceFiles(null);
    } finally {
      setTransferBusy(false);
    }
  }

  // 选择要安装的 APK：只挑 .apk，路径留在内存里，不写任何持久化记录。
  async function pickApk() {
    setApkMessage(null);
    setApkError(null);
    let picked: string | string[] | null;
    try {
      picked = await openFilePicker({
        multiple: false,
        title: "选择要安装到手机的 APK",
        filters: [{ name: "Android 安装包", extensions: ["apk"] }],
      });
    } catch (error) {
      setApkError(errorMessage(error, "无法打开文件选择器。"));
      return;
    }
    // 用户取消选择不算错误，界面回到原样即可。
    if (typeof picked !== "string" || picked.length === 0) return;
    setApkPath(picked);
    setApkReceipt(null);
  }

  // 一键安装：后端用 adb 直接安装，失败原因由后端解析成可照做的文案。
  async function installApk(serial: string) {
    if (!apkPath) return;
    setApkBusy(true);
    setApkMessage(null);
    setApkError(null);
    setApkReceipt(null);
    try {
      const receipt = await invoke<ApkInstallReceipt>("install_apk_to_device", { serial, apkPath });
      setApkReceipt(receipt);
      setApkMessage(`${receipt.summary}手机上可能出现「安装未知应用」等确认，需要你本人同意。`);
    } catch (error) {
      setApkError(errorMessage(error, "安装没有完成。"));
    } finally {
      setApkBusy(false);
    }
  }

  // 取回文件：保存到本机「下载 / MirrorDock」，同名时后端自动顺延序号。
  // X10-75：状态记到**该文件**上（fetchedFiles）；提示语也只说这一个文件，
  // 不写"已保存到…文件夹"（会被读成全部都存好了）。失败记在 fetchErrors，
  // 由该行显示原因并提供「重试」，而不是只在面板底部留一行红字。
  async function fetchDeviceFile(serial: string, fileName: string) {
    setTransferMessage(null);
    setTransferError(null);
    setFetchingName(fileName);
    setFetchErrors((prev) => {
      if (!(fileName in prev)) return prev;
      const next = { ...prev };
      delete next[fileName];
      return next;
    });
    try {
      const receipt = await invoke<TransferReceipt>("fetch_file_from_device", { serial, fileName });
      setFetchedFiles((prev) => ({ ...prev, [fileName]: receipt }));
      setTransferMessage(`已取回「${fileName}」，保存在这台电脑的「下载 / MirrorDock」。`);
      // 文件已安全落在本机，刷新列表以便看到最新状态。
      setDeviceFiles(await invoke<string[]>("list_device_files", { serial }));
    } catch (error) {
      setFetchErrors((prev) => ({ ...prev, [fileName]: errorMessage(error, "没有取回成功。") }));
    } finally {
      setFetchingName(null);
    }
  }

  // 删除前确认（X10-75）：「删除」在已取回态下有两种可能含义（删手机原件？删电脑副本？），
  // 用户点之前无从判断。确认必须点名文件并说清只删哪一份 —— 用项目既有的
  // window.confirm 风格，不引入新的弹窗依赖。
  function confirmDeleteDeviceFile(fileName: string) {
    const ok = window.confirm(
      `删除「${fileName}」？\n\n会删掉手机上的这个文件。电脑里已取回的副本不受影响。`,
    );
    if (ok) void deleteDeviceFile(transferDeviceSerial ?? "", fileName);
  }

  // 打开取回目录（列表级工具条用）。目录由后端约定，这里用最近一次取回的路径定位；
  // 一个都没取回过时退回到该目录本身。
  async function openFetchedFolder() {
    const entries = Object.values(fetchedFiles);
    if (entries.length > 0) {
      const latest = entries[entries.length - 1];
      try {
        await revealItemInDir(latest.path);
        return;
      } catch (error) {
        setTransferError(errorMessage(error, "无法打开文件夹。"));
        return;
      }
    }
    setTransferMessage("还没有取回过文件。取回后就能从这里直接打开了。");
  }

  // 删除手机发送区里的一个文件（X10-60）：只删「下载 / MirrorDock」内的这个文件。
  async function deleteDeviceFile(serial: string, fileName: string) {
    setTransferMessage(null);
    setTransferError(null);
    try {
      await invoke("delete_device_file", { serial, fileName });
      setDeviceFiles(await invoke<string[]>("list_device_files", { serial }));
      // 删掉了就撤掉它的"已取回"标记——文件已不在手机上。
      setFetchedFiles((prev) => {
        if (!(fileName in prev)) return prev;
        const next = { ...prev };
        delete next[fileName];
        return next;
      });
      setTransferMessage(`已从手机删除「${fileName}」。`);
    } catch (error) {
      setTransferError(errorMessage(error, "文件没有从手机上删除。"));
    }
  }


  // 修改快捷键组合：立即持久化；会话进行中会按新组合重新注册（见注册 effect）。
  function updateShortcuts(next: ShortcutSettings) {
    setShortcuts(next);
    try { localStorage.setItem("mirrordock.shortcuts", JSON.stringify(next)); } catch { /* 保存失败只影响下次启动的默认值。 */ }
  }

  // 扫码配对：桌面出码，手机「使用二维码配对设备」扫码后由后端自动 pair + connect。
  async function beginQrPairing() {
    setQrBusy(true);
    setQrError(null);
    setQrProgress(null);
    setQrImage(null);
    try {
      const offer = await invoke<QrPairingOffer>("begin_qr_pairing");
      setQrOffer(offer);
      try {
        setQrImage(await QRCode.toDataURL(offer.payload, { width: 220, margin: 1 }));
      } catch { /* 渲染环境不支持 canvas 时退化为文字提示。 */ }
    } catch (error) {
      setQrError(errorMessage(error, "无法生成配对二维码。"));
    } finally {
      setQrBusy(false);
    }
  }

  async function stopQrPairing() {
    if (!qrOffer) return;
    try {
      await invoke("end_qr_pairing", { serviceName: qrOffer.service_name });
    } catch { /* 状态清理失败不影响界面复位。 */ }
    setQrOffer(null);
    setQrImage(null);
    setQrProgress(null);
  }

  useEffect(() => {
    if (!qrOffer) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const startedAt = Date.now();
    async function poll() {
      if (!qrOffer || disposed) return;
      try {
        const progress = await invoke<QrPairingProgress>("qr_pairing_progress", {
          serviceName: qrOffer.service_name,
          pairingCode: qrOffer.pairing_code,
        });
        if (disposed) return;
        setQrProgress(progress);
        if (progress.stage === "done") {
          void Promise.all([refreshDevices(), refreshTrustedDevices()]);
          setQrOffer(null);
          setQrImage(null);
          return;
        }
        if (progress.stage === "ended" || Date.now() - startedAt > 150_000) {
          void invoke("end_qr_pairing", { serviceName: qrOffer.service_name }).catch(() => undefined);
          setQrOffer(null);
          setQrImage(null);
          setQrError(progress.stage === "ended" ? null : "等待超时：手机没有扫码或网络阻止了 mDNS（公共 Wi-Fi 常见）。请改用配对码配对。");
          return;
        }
      } catch { /* 单次轮询失败不终止等待。 */ }
      if (!disposed) timer = setTimeout(() => void poll(), 1500);
    }
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [qrOffer]);

  async function pairAndConnect() {
    setWirelessBusy(true);
    setWirelessMessage(null);
    try {
      await invoke("pair_wireless_device", { endpoint: pairEndpoint, pairingCode });
      setPairingCode("");
      await invoke("connect_wireless_device", { endpoint: connectEndpoint });
      setPairingCode("");
      setWirelessMessage("配对并连接完成。正在更新设备列表。");
      await Promise.all([refreshDevices(), refreshTrustedDevices()]);
    } catch (error) {
      setWirelessMessage(errorMessage(error, "无线连接未完成。请重新检查手机上的无线调试页面。"));
    } finally {
      setPairingCode("");
      setWirelessBusy(false);
    }
  }

  async function reconnect(endpoint: string) {
    setWirelessBusy(true);
    setWirelessMessage(null);
    try {
      await invoke("connect_wireless_device", { endpoint });
      setWirelessMessage("已发送连接请求，正在更新设备列表。");
      await refreshDevices();
    } catch (error) {
      setWirelessMessage(errorMessage(error, "无法重新连接该设备。"));
    } finally {
      setWirelessBusy(false);
    }
  }

  async function forgetDevice(endpoint: string) {
    setWirelessBusy(true);
    setWirelessMessage(null);
    try {
      await invoke("forget_trusted_wireless_device", { endpoint });
      setWirelessMessage("已从 MirrorDock 的本机列表移除，并断开当前连接。");
      await Promise.all([refreshDevices(), refreshTrustedDevices()]);
    } catch (error) {
      setWirelessMessage(errorMessage(error, "无法移除该设备。"));
    } finally {
      setWirelessBusy(false);
    }
  }

  const readyDevices = check?.devices.filter((device) => device.state === "ready") ?? [];
  const readyDevice = readyDevices.find((device) => device.serial === selectedSerial) ?? readyDevices[0];
  // 同型号多台设备重名时追加 -1、-2 后缀，便于区分（基于全部设备统一编号）。
  const baseLabels = buildDisplayLabels(check?.devices ?? []);
  // X10-88：自定义备注覆盖型号标签——有备注的设备全端显示备注名。
  const displayLabels = (() => {
    const merged: Record<string, string> = { ...baseLabels };
    for (const device of check?.devices ?? []) {
      const key = device.physical_serial ?? device.serial;
      if (deviceNicknames[key]) merged[device.serial] = deviceNicknames[key];
    }
    return merged;
  })();
  const scrcpyReady = check?.scrcpy_available ?? false;
  const readySerial = readyDevice?.serial ?? null;
  // 快捷键处理器读取的最新 serial。
  shortcutsRef.current.readySerial = readySerial;

  // X10-74 文件传输的目标设备：**不依赖镜像会话**。
  // 优先已就绪设备；没有就绪设备但有会话设备（正在镜像/连接）也可用 ——
  // 两者都拿不到才为空，此时面板如实提示去连接页。
  // 取回/发送走 adb 通道，与 scrcpy 会话无关，所以不该被会话状态门禁挡住。
  const transferDeviceSerial = readyDevice?.serial ?? sessionSerial;
  // X10-75：按用途分组展示。分组是纯函数结果，列表为空时自然为空数组。
  const transferGroups = groupTransferFiles(deviceFiles ?? []);
  // 已取回数量（用于列表底部的计数说明）。
  const fetchedCount = Object.keys(fetchedFiles).length;

  // 设备换了就丢弃旧列表：否则会出现"这是上一台手机的文件"这种误导。
  useEffect(() => {
    setDeviceFiles(null);
    setTransferMessage(null);
    setTransferError(null);
    // 取回记录也属于"那台设备的状态"，换设备后一并清掉。
    setFetchedFiles({});
  }, [readyDevice?.serial]);

  // 打开「工具」页时自动加载手机文件列表（首次也加载）。
  // 旧实现只在用户点按钮后才有列表，导致"手机发了文件但客户端一直看不到"——
  // 用户不点按钮就永远看不到。现在让"进入页面"本身就完成读取。
  useEffect(() => {
    if (tab !== "tools") return;
    if (deviceFiles !== null || transferBusy) return;
    if (!transferDeviceSerial) return;
    void refreshDeviceFiles(transferDeviceSerial);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, transferDeviceSerial]);

  // X10-72（修 X10-71 的可见性缺陷）：桌面模式设置的归属设备现在**始终**是一个
  // 真实设备 —— 显式选择 > 会话设备 > 当前就绪设备 > 唯一已配对设备。旧逻辑在
  // 「单设备 / 未镜像」时把它留空，于是读写落到全局 options，用户看到的就像全局
  // 开关。现在模块内明确显示归属设备，无设备时才回落全局默认并如实说明。
  const desktopPrefDevices = buildDesktopPrefDevices({
    adbDevices: check?.devices ?? [],
    labels: displayLabels,
    sessionSerials: sessions.map((item) => item.serial).filter((s): s is string => Boolean(s)),
    prefs: desktopPrefs,
  });
  const settingsDesktopTarget = desktopTargetSerial ?? sessionSerial ?? readySerial ?? null;
  // X10-84：读取这台设备的 pref 时统一换算成稳定 physical_serial 键。
  const settingsDesktopPrefKey = desktopPrefKeyFor(check?.devices, settingsDesktopTarget);
  const targetDesktopPref = settingsDesktopPrefKey ? desktopPrefs[settingsDesktopPrefKey] ?? null : null;
  // 有设备归属时，缺省值取全局 options（首次进入该设备的编辑态），此后一律以
  // 该设备自己的 pref 为准 —— 打开这台设备的桌面模式不会连带改到别的设备。
  const shownDesktopMode = targetDesktopPref ? targetDesktopPref.desktop_mode : options.desktop_mode;
  const shownDesktopApp = targetDesktopPref ? targetDesktopPref.desktop_app : options.desktop_app;
  // 当前编辑归属设备的展示名：让「这几项属于哪台设备」在界面上直接可读。
  const settingsDesktopTargetLabel = settingsDesktopTarget
    ? displayLabels[settingsDesktopTarget] ?? settingsDesktopTarget
    : null;
  useEffect(() => {
    if (shownDesktopMode) void ensureDeviceApps(settingsDesktopTarget ?? readySerial);
  }, [shownDesktopMode, settingsDesktopTarget, readySerial]);
  // 已填包名能对上候选列表时，把应用名展示在说明文字里（小白用户看得懂名字）。
  const desktopAppsForTarget = deviceApps[settingsDesktopTarget ?? readySerial ?? ""] ?? [];
  const currentDesktopAppName = shownDesktopApp
    ? desktopAppsForTarget.find((app) => app.package === shownDesktopApp)?.name ?? null
    : null;
  // 拖拽安装的目标设备：拖放回调是长生命周期闭包，读 ref 拿最新就绪设备，
  // 避免注册后设备插拔导致指向过期。
  const dragTargetRef = useRef<{ serial: string; label: string } | null>(null);
  dragTargetRef.current = readyDevice
    ? { serial: readyDevice.serial, label: displayLabels[readyDevice.serial] ?? readyDevice.label }
    : null;
  // 拖拽路由（X10-63）：APK → 安装到当前就绪手机；其他文件 → 发送到该手机的
  // 发送区（下载/MirrorDock）。混拖时各走各路，分别如实汇报。
  function handleDroppedFiles(paths: string[]) {
    const apks = paths.filter((path) => path.toLowerCase().endsWith(".apk"));
    const others = paths.filter((path) => !path.toLowerCase().endsWith(".apk"));
    if (apks.length > 0) void installDroppedApk(apks);
    if (others.length > 0) void sendDroppedFiles(others);
  }
  // 拖入非 APK 文件 → 逐个发送到手机发送区；结果走右下角浮层反馈（拖拽可发生在任何页签）。
  async function sendDroppedFiles(paths: string[]) {
    const target = dragTargetRef.current;
    if (!target) {
      setDropInstallNotice({ kind: "error", text: "请先在「连接」页连接手机，再拖拽发送文件。" });
      return;
    }
    const names = paths.map((path) => path.split(/[\\/]/).pop() ?? path);
    setDropInstallNotice({ kind: "info", text: `正在把 ${names[0]}${paths.length > 1 ? ` 等 ${paths.length} 个文件` : ""} 发送到 ${target.label} 的发送区…` });
    let sent = 0;
    const failures: string[] = [];
    for (let i = 0; i < paths.length; i++) {
      try {
        await invoke("send_file_to_device", { serial: target.serial, localPath: paths[i] });
        sent += 1;
      } catch (error) {
        failures.push(`${names[i]}：${errorMessage(error, "发送失败")}`);
      }
    }
    if (failures.length > 0) {
      setDropInstallNotice({ kind: "error", text: `发送到手机发送区：${sent}/${paths.length} 成功。${failures.join(" ")}` });
    } else {
      setDropInstallNotice({
        kind: "info",
        text: `已把 ${sent} 个文件发送到 ${target.label} 的发送区（手机「下载 / MirrorDock」）。`,
      });
    }
  }
  // 拖入的文件里挑出 APK 安装包安装到当前就绪手机；多个时只装第一个并说明，
  // 不静默批量安装。结果走右下角浮层反馈（拖拽可发生在任何页签）。
  async function installDroppedApk(apks: string[]) {
    const target = dragTargetRef.current;
    if (!target) {
      setDropInstallNotice({ kind: "error", text: "请先在「连接」页连接手机，再拖拽安装 APK。" });
      return;
    }
    const fileName = apks[0].split(/[\\/]/).pop() ?? apks[0];
    setDropInstallNotice({ kind: "info", text: `正在把 ${fileName} 安装到 ${target.label}…` });
    try {
      const receipt = await invoke<ApkInstallReceipt>("install_apk_to_device", { serial: target.serial, apkPath: apks[0] });
      setDropInstallNotice({
        kind: "info",
        text: `${receipt.summary}已交给 ${target.label} 安装。手机上可能出现「安装未知应用」等确认，需要你本人同意。${apks.length > 1 ? `（拖入了 ${apks.length} 个 APK，只安装了第一个）` : ""}`,
      });
    } catch (error) {
      setDropInstallNotice({ kind: "error", text: errorMessage(error, "安装没有完成。") });
    }
  }
  // 拖拽安装监听：仅真实 Tauri 环境有拖放事件；浏览器/测试环境直接跳过。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setApkDragOver(true);
        } else if (event.payload.type === "leave") {
          setApkDragOver(false);
        } else if (event.payload.type === "drop") {
          setApkDragOver(false);
          void handleDroppedFiles(event.payload.paths);
        }
      })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // 宿主输入源托管提示（X10-39）：仅真实 Tauri 环境有事件通道。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen("host-input-source-switched", () => {
      setHostImNotice(
        "当前输入法会导致镜像窗口打字无效。已临时切换到 ABC 布局，镜像结束后会自动恢复你原来的输入法。",
      );
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);
  // 断线自动重连进度（X10-45 无线 / X10-59 USB）：中断 → 等待 → 成功恢复 / 放弃。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<{ status: "waiting" | "succeeded" | "gave_up"; endpoint: string; kind?: "wireless" | "usb" }>(
      "wireless-reconnect-status",
      (event) => {
        const status = event.payload?.status;
        const usb = event.payload?.kind === "usb";
        if (status === "waiting") {
          setReconnectNotice(usb
            ? "数据线已断开，正在等待重新插入，插回后会自动恢复镜像（最多等 15 分钟）。"
            : "无线连接已断开，正在等待手机重新上线，回网后会自动恢复镜像（最多等 15 分钟）。");
        } else if (status === "succeeded") {
          setReconnectNotice(usb ? "数据线已重新连接，镜像会话已自动恢复。" : "手机已回网，镜像会话已自动恢复。");
        } else {
          setReconnectNotice(usb
            ? "等待重新插回数据线超时，已停止自动重连。需要时请手动重新连接。"
            : "等待手机回网超时，已停止自动重连。需要时请手动重新连接。");
        }
      },
    )
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);
  // 镜像会话意外中断（X10-59）：镜像窗口是独立进程，掉线时窗口消失但应用还在，
  // 这里给用户一句明确解释；若自动重连接管，后续 waiting 事件会覆盖本提示。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<{ serial: string; unexpected: boolean }>("mirror-session-ended", (event) => {
      if (event.payload?.unexpected) {
        setReconnectNotice("镜像连接已中断（设备连接断开）。");
      }
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);
  // X10-80：桌面模式下虚拟屏启动的应用没有真正落地（常见于部分游戏的投屏
  // 兼容性限制，真机实证见 lib.rs X10-80 注释）→ 白屏的根因提示与替代路径，
  // 不让用户对着一面白屏自己猜。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<{ serial: string; package: string }>("desktop-app-missing", (event) => {
      const pkg = event.payload?.package ?? "";
      setReconnectNotice(
        `应用（${pkg}）未能在虚拟屏上启动，桌面模式窗口可能保持空白。建议关闭该设备的「桌面模式」改用普通镜像，或换用兼容的应用（部分游戏/应用有虚拟屏限制）。`,
      );
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);
  // X10-60：伴侣端发送区有新文件 → 提示 + 已打开的列表自动刷新。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen("companion-files-changed", () => {
      if (!disposed) setFilesChangedHint(Date.now());
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);
  // 收到新文件事件后的实际动作：**无条件加载列表**，不再要求用户先点过一次按钮。
  // 旧实现要求 `deviceFiles !== null` 才刷新，而 deviceFiles 只在手动点按钮后
  // 才赋值 —— 于是形成死锁：不点按钮就永远看不到新文件（用户实际报的现象）。
  // 文件传输不依赖镜像会话，所以这里也不看 sessionActive。
  useEffect(() => {
    if (filesChangedHint === null) return;
    setTransferMessage("手机发送区有新文件到达。");
    // 切到工具页并自动刷新：新文件是用户等着要看的东西，不该要求他先点按钮。
    setTab("tools");
    if (transferDeviceSerial && !transferBusy) {
      void refreshDeviceFiles(transferDeviceSerial);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filesChangedHint]);
  // X10-60：伴侣端崩溃堆栈上报（本地点对点，仅界面展示）。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<{ stack: string }>("companion-crash-report", (event) => {
      if (!disposed && event.payload?.stack) setCompanionCrash(event.payload.stack);
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);
  // X10-66 通知镜像一期：手机新通知实时进面板。只在内存展示（最近 50 条），
  // 不落盘；转发开关与隐私边界都在手机端（默认关、可随时关）。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<PhoneNotification>("companion-notification", (event) => {
      const item = event.payload;
      if (!disposed && item && typeof item.pkg === "string") {
        setPhoneNotifications((prev) => [item, ...prev].slice(0, 50));
      }
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);
  // 选中设备变化时重新探测能力信息。只读取设备信息，不启动镜像。
  // 探测结果用于设置页「转发手机声音」开关的一致性；能力说明文案在帮助中心。
  useEffect(() => {
    if (!readySerial) {
      setCapabilities(null);
      return;
    }
    let disposed = false;
    setCapabilities(null);
    invoke<DeviceCapabilities>("probe_device_capabilities", { serial: readySerial })
      .then((value) => { if (!disposed) setCapabilities(value); })
      .catch(() => { /* 读不到就按未知处理，不阻塞连接 */ });
    return () => { disposed = true; };
  }, [readySerial]);
  // 每张就绪设备卡的锁屏便签（X10-33）：按就绪设备集合轮询，5 秒一刷；
  // 某台读不到就不更新它的便签，不影响其它设备。
  const readySerialsKey = readyDevices.map((device) => device.serial).join(",");
  useEffect(() => {
    const serials = readySerialsKey ? readySerialsKey.split(",") : [];
    if (serials.length === 0) return;
    let disposed = false;
    for (const serial of serials) void refreshLockReport(serial);
    const timer = window.setInterval(() => {
      for (const serial of serials) {
        if (disposed) break;
        invoke<DeviceLockReport>("device_lock_report", { serial })
          .then((value) => { if (!disposed) setLockReports((prev) => ({ ...prev, [serial]: value })); })
          .catch(() => { /* 保持上一次结果 */ });
      }
    }, 8000);
    return () => { disposed = true; window.clearInterval(timer); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [readySerialsKey]);
  // 探测确认这台手机不支持系统音频转发（Android < 11）时，把「转发手机声音」
  // 如实关掉并禁用：scrcpy 在这类设备上会自动禁用音频，界面必须与之一致，
  // 而不是留一个开了也不生效的开关。
  const audioUnsupported = capabilities?.audio_forwarding_supported === false;
  useEffect(() => {
    if (audioUnsupported && options.audio) {
      updateOptions({ ...options, audio: false });
    }
    // 只在探测结论变化时介入；options 变化由开关本身处理。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [audioUnsupported]);
  const statusMessage = session ? sessionStatus(session) : null;
  const statusRole = session && ["unauthorized", "offline", "failed"].includes(session.phase) ? "alert" : "status";
  // 按当前可见设备的 label 猜品牌；猜不出就是 null，界面如实显示「未检测到」。
  const detectedGuide = detectBrand((check?.devices ?? []).map((device) => device.label));
  const shownGuide: BrandGuide | null =
    brandGuides.find((guide) => guide.key === (guideKey ?? detectedGuide?.key)) ?? null;
  // 会话当前属于哪台设备（含显示名），用于顶栏与按钮文案说清「镜像运行中」的主语。
  const sessionDevice = sessionSerial
    ? (check?.devices ?? []).find((device) => device.serial === sessionSerial)
    : undefined;
  const sessionDeviceLabel = sessionDevice
    ? (displayLabels[sessionDevice.serial] ?? sessionDevice.label)
    : sessionSerial ?? null;
  // 顶栏状态：一句话说清“现在这台电脑能不能用手机”。
  const topbarStatus = isChecking
    ? "正在检查连接…"
    : sessionActive
      ? activeSessionCount > 1
        ? `${activeSessionCount} 台设备镜像中`
        : sessionDeviceLabel
          ? `镜像运行中：${sessionDeviceLabel}`
          : "镜像运行中"
      : readyDevice
        ? readyDevice.label
        : check?.devices.length
          ? "手机需要授权"
          : "未连接手机";

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand" aria-label="MirrorDock">
          <span className="brand-mark" aria-hidden="true">M</span>
          <span className="brand-name">MirrorDock</span>
        </div>
        <nav className="side-nav" aria-label="主导航">
          {navItems.map((item) => (
            <button
              key={item.key}
              type="button"
              className={tab === item.key ? "nav-item active" : "nav-item"}
              aria-current={tab === item.key ? "page" : undefined}
              onClick={() => setTab(item.key)}
            >
              {item.icon}
              <span>{item.label}</span>
            </button>
          ))}
        </nav>
        <div className="sidebar-foot">
          <span className="edition-pill">{editionLabel(entitlement?.edition)}</span>
          <span className="local-pill">仅在本机连接</span>
        </div>
      </aside>

      <div className="main-area">
        <header className="topbar">
          <div className="device-chip">
            <span className={`status-dot ${readyDevice ? "ready" : sessionActive ? "ready" : "idle"}`} aria-hidden="true" />
            <span>{topbarStatus}</span>
          </div>
          {/* 单会话时设备卡片上已有唯一的「结束镜像」；顶栏只在多台同停时才出现。 */}
          {activeSessionCount > 1 && (
            <button className="secondary-button" type="button" onClick={() => void stopMirroring()} disabled={isStopping}>
              {isStopping ? "正在结束…" : `结束全部（${activeSessionCount} 台）`}
            </button>
          )}
        </header>

        {/* 拖拽安装反馈浮层：右下角，任何页签都能看到结果。 */}
        {dropInstallNotice && (
          <div className={`drop-toast ${dropInstallNotice.kind === "error" ? "drop-toast-error" : ""}`} role="status">
            <span>{dropInstallNotice.text}</span>
            <button className="text-button" type="button" onClick={() => setDropInstallNotice(null)}>知道了</button>
          </div>
        )}
        {/* 拖入文件时的全屏提示：APK 装到手机，其他文件进发送区。 */}
        {apkDragOver && (
          <div className="drop-overlay" aria-hidden="true">
            <div className="drop-overlay-card">
              <strong>松开鼠标，传文件到手机</strong>
              <p>{dragTargetRef.current ? `目标：${dragTargetRef.current.label}（APK 安装，其他文件进入发送区）` : "请先连接一台手机"}</p>
            </div>
          </div>
        )}

        <div className="content">
          {/* -- 连接（主页） ------------------------------------------------ */}
          <section className={`tab-panel ${tab === "home" ? "" : "panel-hidden"}`} aria-label="连接">
            <div className="page-head">
              <h1>在电脑上安心使用手机</h1>
              <p className="intro">用数据线或同一 Wi-Fi 连接手机，画面只出现在这台电脑上。</p>
            </div>

            <section className="connection-card" aria-live="polite">
              <div className="connection-heading">
                <div>
                  <p className="eyebrow">
                    {sessionActive
                      ? activeSessionCount > 1
                        ? `镜像运行中：${activeSessionCount} 台设备`
                        : sessionDeviceLabel
                          ? `镜像运行中：${sessionDeviceLabel}`
                          : "镜像运行中"
                      : "第一步：连接手机"}
                  </p>
                  <h2>
                    {isChecking
                      ? "正在检查 USB 连接…"
                      : sessionActive
                        ? activeSessionCount > 1
                          ? "多台设备同时镜像中"
                          : "镜像进行中"
                        : readyDevice
                          ? "手机已准备就绪"
                          : "等待连接手机"}
                  </h2>
                </div>
                <button className="secondary-button" type="button" onClick={() => void refreshDevices()} disabled={isChecking}>
                  {isChecking ? "检查中…" : "重新检查"}
                </button>
              </div>

              {/* 状态/错误提示集中在一处，纵向紧凑排列；容器为空时不占位。 */}
              <div className="status-stack">
                {check?.diagnostic && <p className="diagnostic">{check.diagnostic}</p>}
                {launchError && <p className="diagnostic">{launchError}</p>}
                {sessionError && <p className="diagnostic" role="alert">{sessionError}</p>}
                {statusMessage && <p className="diagnostic" role={statusRole}>{statusMessage}</p>}
                {revokeNotice && (
                  <div className={`notice-dismissable ${revokeNotice.error ? "notice-error" : ""}`} role={revokeNotice.error ? "alert" : "status"}>
                    <p className="diagnostic">{revokeNotice.text}</p>
                    <button
                      className="notice-close"
                      type="button"
                      aria-label="关闭这条通知"
                      title="关闭"
                      onClick={() => setRevokeNotice(null)}
                    >
                      ×
                    </button>
                  </div>
                )}
                {hostImNotice && (
                  <div className="notice-dismissable" role="status">
                    <p className="diagnostic">{hostImNotice}</p>
                    <button
                      className="notice-close"
                      type="button"
                      aria-label="关闭这条通知"
                      title="关闭"
                      onClick={() => setHostImNotice(null)}
                    >
                      ×
                    </button>
                  </div>
                )}
                {lockError && <p className="diagnostic" role="alert">{lockError}</p>}
                {reconnectNotice && (
                  <p className="diagnostic" role="status">
                    {reconnectNotice}
                    <button type="button" className="dismiss-button" aria-label="关闭这条通知" title="关闭" onClick={() => setReconnectNotice(null)}>×</button>
                  </p>
                )}
                {companionCrash && (
                  <div className="diagnostic" role="status">
                    <p>伴侣 App 报来一份崩溃记录（经本机连接点对点送达，未上传云端）：</p>
                    {companionCrashExpanded && (
                      <pre style={{ maxHeight: 180, overflow: "auto", whiteSpace: "pre-wrap", fontSize: 12 }}>{companionCrash}</pre>
                    )}
                    <span>
                      <button type="button" className="text-button" onClick={() => setCompanionCrashExpanded((v) => !v)}>
                        {companionCrashExpanded ? "收起详情" : "查看详情"}
                      </button>
                      <button type="button" className="dismiss-button" aria-label="关闭崩溃记录" title="知道了" onClick={() => { setCompanionCrash(null); setCompanionCrashExpanded(false); }}>×</button>
                    </span>
                  </div>
                )}
                {sessionActive && pinPadActive && (
                  <p className="diagnostic" role="status">🔒 此画面受系统安全保护，无法镜像。请在手机上直接输入密码解锁，解锁后画面自动恢复。</p>
                )}
                {sessionActive && activeSessionList.some((item) => lockReports[item.serial]?.screen === "asleep") && (
                  <p className="diagnostic" role="status">有手机的屏幕已关闭：在镜像窗口上点右键即可直接点亮屏幕（scrcpy 内置手势），无需回到本窗口。</p>
                )}
                {settingsNotice && <p className="diagnostic">{settingsNotice}</p>}
              </div>

              {/* 设备卡片列表：一台设备一张卡，状态即操作——每个动作全页只出现一次。
                  镜像中 →「结束镜像」；就绪 →「开始镜像」；待授权/离线 → 文字说明。
                  任何状态都有「取消授权」按钮（X10-33）；取消成功后该设备立即从列表移除，
                  重新就绪（用户重新授权）时自动恢复显示。
                  会话仍在但设备已从列表消失（如无线瞬断）时，补一张兜底卡保留结束入口。 */}
              {!isChecking && check?.devices && check.devices.length > 0 && (
                <div className="device-cards" aria-label="已连接的设备">
                  {check.devices
                    .filter((device) => device.state === "ready" || !hiddenRevokedSerials.includes(device.serial))
                    .map((device) => {
                    const badge = connectionLabel(device);
                    const lockStamp = device.state === "ready" ? lockReports[device.serial] : undefined;
                    const own = sessionFor(device.serial);
                    const owned = own?.phase === "connecting" || own?.phase === "streaming";
                    const favorite = favoriteDevices.includes(device.serial);
                    return (
                      <div className={`device-card ${owned ? "device-card-active" : ""}`} key={device.serial}>
                        <span className={`status-dot ${owned ? "ready" : device.state}`} aria-hidden="true" />
                        <div>
                          <strong>
                            {favorite && <span className="fav-star" aria-hidden="true">★ </span>}
                            {displayLabels[device.serial] ?? device.label}
                            {badge && <span className="conn-badge inline">{badge}</span>}
                            {lockStamp && <span className="conn-badge inline lock-badge">{lockTag(lockStamp)}</span>}
                            {/* X10-88：自定义备注编辑（铅笔按钮）。备注以 physical_serial 为键。 */}
                            <button
                              type="button"
                              className="nickname-edit-btn"
                              aria-label={`备注 ${displayLabels[device.serial] ?? device.label}`}
                              title="自定义备注名（区分同型号设备）"
                              onClick={() => {
                                const key = device.physical_serial ?? device.serial;
                                setNicknameEditing(key);
                                setNicknameDraft(deviceNicknames[key] ?? "");
                              }}
                            >
                              ✎
                            </button>
                          </strong>
                          {nicknameEditing === (device.physical_serial ?? device.serial) && (
                            <div className="nickname-editor">
                              <input
                                type="text"
                                value={nicknameDraft}
                                placeholder={device.label}
                                aria-label="设备备注名"
                                onChange={(e) => setNicknameDraft(e.target.value)}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter") {
                                    const key = device.physical_serial ?? device.serial;
                                    setDeviceNicknames(writeDeviceNickname(key, nicknameDraft));
                                    setNicknameEditing(null);
                                  } else if (e.key === "Escape") {
                                    setNicknameEditing(null);
                                  }
                                }}
                              />
                              <button
                                type="button"
                                className="text-button"
                                onClick={() => {
                                  const key = device.physical_serial ?? device.serial;
                                  setDeviceNicknames(writeDeviceNickname(key, nicknameDraft));
                                  setNicknameEditing(null);
                                }}
                              >
                                保存
                              </button>
                              {deviceNicknames[device.physical_serial ?? device.serial] && (
                                <button
                                  type="button"
                                  className="text-button danger"
                                  onClick={() => {
                                    const key = device.physical_serial ?? device.serial;
                                    setDeviceNicknames(writeDeviceNickname(key, ""));
                                    setNicknameEditing(null);
                                  }}
                                >
                                  清除
                                </button>
                              )}
                            </div>
                          )}
                          <p>
                            {owned
                              ? own.phase === "connecting"
                                ? "画面正在启动…若几秒后未出现，请看手机屏幕是否亮起并确认授权。"
                                : "镜像进行中。直接关闭电脑上的镜像窗口也可以结束。"
                              : stateCopy[device.state].detail}
                          </p>
                        </div>
                        <span className="device-card-actions">
                          {/* X10-68：收藏置顶开关（只影响最近设备列表排序）。 */}
                          <button
                            className="text-button fav-toggle"
                            type="button"
                            aria-label={favorite ? `取消收藏 ${displayLabels[device.serial] ?? device.label}` : `收藏 ${displayLabels[device.serial] ?? device.label}`}
                            title={favorite ? "取消收藏" : "收藏（最近列表置顶）"}
                            onClick={() => toggleFavoriteDevice(device.serial)}
                          >
                            {favorite ? "★" : "☆"}
                          </button>
                          {device.state === "ready" && (
                            <>
                              <button
                                className="secondary-button"
                                type="button"
                                disabled={lockBusy}
                                onClick={() => { setSelectedSerial(device.serial); void wakeDevice(device.serial); }}
                              >
                                {lockBusy && selectedSerial === device.serial ? "正在唤醒…" : "屏幕唤醒"}
                              </button>
                              {owned ? (
                                <button className="secondary-button danger-stop" type="button" disabled={isStopping} onClick={() => void stopMirroring(device.serial)}>
                                  {stoppingSerial === device.serial ? "正在结束…" : "结束镜像"}
                                </button>
                              ) : (
                                <button className="primary-button" type="button" disabled={!scrcpyReady || isLaunching || isStopping} onClick={() => void startMirroring(device.serial)}>
                                  {launchingSerial === device.serial ? "正在启动…" : scrcpyReady ? "开始镜像" : "镜像引擎准备中"}
                                </button>
                              )}
                            </>
                          )}
                          {!owned && device.state !== "ready" && looksLikeWirelessEndpoint(device.serial) && device.state === "offline" && (
                            <button className="secondary-button" type="button" disabled={wirelessBusy} onClick={() => void reconnectRecentDevice(device.serial)}>重新连接</button>
                          )}
                          {!owned && device.state !== "ready" && (
                            <span className="status-label">{stateCopy[device.state].label}</span>
                          )}
                          {!owned && (revokeArmedSerial === device.serial ? (
                            <>
                              <button className="secondary-button danger-stop" type="button" disabled={revoking} onClick={() => void revokeDeviceAccess(device.serial)}>
                                {revoking ? "正在执行…" : "确认取消"}
                              </button>
                              <button className="secondary-button" type="button" disabled={revoking} onClick={() => setRevokeArmedSerial(null)}>
                                算了
                              </button>
                            </>
                          ) : (
                            <button
                              className="secondary-button revoke-button"
                              type="button"
                              title="关闭手机调试开关、断开本机连接并清理记录；最后一步需在手机上点「撤销 USB 调试授权」"
                              onClick={() => setRevokeArmedSerial(device.serial)}
                            >
                              取消授权
                            </button>
                          ))}
                        </span>
                      </div>
                    );
                  })}
                  {activeSessionList
                    .filter((item) => !check.devices.some((device) => device.serial === item.serial))
                    .map((item) => (
                      <div className="device-card device-card-active" key={item.serial}>
                        <span className="status-dot ready" aria-hidden="true" />
                        <div>
                          <strong>{displayLabels[item.serial] ?? item.serial}</strong>
                          <p>镜像会话仍在进行，设备暂时未出现在连接列表。</p>
                        </div>
                        <span className="device-card-actions">
                          <button className="secondary-button danger-stop" type="button" disabled={isStopping} onClick={() => void stopMirroring(item.serial)}>
                            {stoppingSerial === item.serial ? "正在结束…" : "结束镜像"}
                          </button>
                        </span>
                      </div>
                    ))}
                </div>
              )}

              {(!isChecking && !readyDevice) && (
                <ol className="setup-steps">
                  <li><span>1</span><div><strong>使用可传输数据的数据线连接手机</strong><p>如果手机弹出 USB 用途选择，请选择“文件传输”。</p></div></li>
                  <li><span>2</span><div><strong>在手机上开启“USB 调试”</strong><p>这是 Android 提供的安全授权，用于将画面显示到这台电脑。</p></div></li>
                  <li><span>3</span><div><strong>解锁手机并允许这台电脑</strong><p>在“允许 USB 调试吗？”中选择允许。你可以随时在手机设置中撤销。</p></div></li>
                </ol>
              )}

              {(!isChecking && !readyDevice) && (
                <div className="capability-panel brand-guide" aria-live="polite">
                  <strong>按品牌查看开启步骤</strong>
                  <p className="capability-pending">
                    {detectedGuide
                      ? `检测到「${detectedGuide.name}」，已选中；不符可手动切换。`
                      : "选择手机品牌，查看开启步骤。"}
                  </p>
                  <div className="device-picker" aria-label="选择手机品牌">
                    {brandGuides.map((guide) => (
                      <button
                        className={shownGuide?.key === guide.key ? "device-choice selected" : "device-choice"}
                        type="button"
                        key={guide.key}
                        onClick={() => setGuideKey(guide.key)}
                      >
                        {guide.name}
                      </button>
                    ))}
                  </div>
                  {shownGuide && (
                    <>
                      <ol className="setup-steps brand-steps">
                        <li><span>1</span><div><strong>打开开发者选项</strong><p>{shownGuide.openDeveloperOptions}</p></div></li>
                        <li><span>2</span><div><strong>开启 USB 调试</strong><p>{shownGuide.usbDebugging}</p></div></li>
                        <li><span>3</span><div><strong>（可选）无线调试</strong><p>{shownGuide.wireless}</p></div></li>
                      </ol>
                      {shownGuide.notes.length > 0 && (
                        <ul className="brand-notes">
                          {shownGuide.notes.map((note) => <li key={note}>{note}</li>)}
                        </ul>
                      )}
                    </>
                  )}
                  <p className="capability-pending">菜单名称因机型而异，以手机实际设置为准。开启后点「重新检查」。</p>
                </div>
              )}

              {/* X10-85：首页不再展示「最近使用过的设备」列表（用户拍板）。最近设备
                  数据仍在后端记录（供无线重连等内部逻辑），仅移除 UI 展示。 */}
              {recentMessage && (
                <div className="recent-devices" aria-label="设备记录操作反馈">
                  <p className="recent-note" role="status">{recentMessage}</p>
                </div>
              )}
            </section>

          </section>

          {/* -- 工具 -------------------------------------------------------- */}
          <section className={`tab-panel ${tab === "tools" ? "" : "panel-hidden"}`} aria-label="工具">
            <div className="page-head">
              <h1>工具</h1>
              <p className="intro">截图、录像与文件传输都直接走数据线，内容只保存在这台电脑上。</p>
            </div>
            {readyDevice ? (
              <div className="panel-grid">
                <div className="capability-panel screenshot-panel" aria-live="polite">
                  <strong>截图</strong>
                  <p className="capability-pending">保存到本机「图片 / MirrorDock」。受保护页面（支付、密码）会截成黑屏，是系统限制，不是故障。</p>
                  <button className="secondary-button" type="button" disabled={screenshotBusy} onClick={() => void captureScreen(readyDevice.serial)}>
                    {screenshotBusy ? "正在处理…" : "截取当前画面"}
                  </button>
                  {screenshot && (
                    <div className="screenshot-result">
                      <p className="capability-summary">{screenshot.file_name} · {formatBytes(screenshot.bytes)}</p>
                      <p className="screenshot-path">{screenshot.path}</p>
                      <span>
                        <button className="text-button" type="button" onClick={() => void revealCapture()}>在文件夹中显示</button>
                        <button className="text-button danger" type="button" disabled={screenshotBusy} onClick={() => void undoCapture()}>删除这张截图</button>
                      </span>
                    </div>
                  )}
                  {screenshotError && <p className="capability-pending" role="alert">{screenshotError}</p>}
                </div>
                <div className="capability-panel recording-panel" aria-live="polite">
                  <strong>录像</strong>
                  {/* X10-92：手动启停，不重启镜像窗口。 */}
                  <p>
                    <button
                      className="primary-button"
                      type="button"
                      disabled={recordingBusy || !proEdition}
                      onClick={() => void toggleRecording()}
                    >
                      {recording?.active ? "结束录制" : "开始录制"}
                    </button>
                  </p>
                  {recording ? (
                    <div className="screenshot-result">
                      <p className="capability-summary">
                        {recording.active ? "正在录制：" : "已结束录制："}{recording.file_name}
                      </p>
                      <p className="screenshot-path">{recording.path}</p>
                      <span>
                        <button className="text-button" type="button" onClick={() => void revealRecording()}>在文件夹中显示</button>
                        <button className="text-button danger" type="button" disabled={recordingBusy || recording.active} onClick={() => void removeRecording()}>删除这段录像</button>
                      </span>
                      {recording.active && <p className="screenshot-path">录像正在写入，点「结束录制」后定型；录制中无法删除。镜像窗口不会因录制而中断。</p>}
                    </div>
                  ) : (
                    <p className="capability-pending">镜像开启后随时点「开始录制」；何时录、录多久由你决定，画面不会中断。文件保存在「视频 / MirrorDock」，结束录制即定型可播放。</p>
                  )}
                  {recordingError && <p className="capability-pending" role="alert">{recordingError}</p>}
                </div>
              </div>
            ) : (
              <div className="capability-panel">
                <strong>先连接手机</strong>
                <p className="capability-pending">截图与录像需要先在「连接」页连接并授权手机。文件传输不依赖镜像，见下方「文件传输」面板。</p>
              </div>
            )}

            {/* 文件传输（X10-74 / X10-75 重设计）。
                移出 {readyDevice ? ...} 门禁：取回/发送走 adb 通道，与镜像会话无关。
                X10-75 的设计修正（纯视觉 + 状态表达，不动传输逻辑）：
                - 操作列**固定宽度并右对齐**（旧实现文件名多长决定按钮位置，视觉上散乱）；
                - 每行按自己的取回结果显示「已取回 ✓」而不是重复的「取回到电脑」；
                - 按文件类型分组（安装包 / 文档 / 其他），小白用户更容易找；
                - 提示语只说**这一个文件**，不说"已保存到文件夹"（会被读成全部都存好了）。 */}
            <div className="capability-panel transfer-panel" aria-live="polite" style={{ marginTop: 16 }}>
              <div className="transfer-head">
                <div>
                  <strong>文件传输</strong>
                  <p className="capability-pending">
                    手机「下载 / MirrorDock」与这台电脑之间互传，只经过你自己的数据线或局域网。
                  </p>
                </div>
                <div className="transfer-actions">
                  <button className="secondary-button" type="button" disabled={transferBusy} onClick={() => void sendFileTo(transferDeviceSerial ?? "")}>
                    {transferBusy ? "正在处理…" : "发送到手机"}
                  </button>
                  <button className="ghost-button" type="button" disabled={transferBusy || fetchingName !== null} onClick={() => transferDeviceSerial && void refreshDeviceFiles(transferDeviceSerial)}>
                    刷新
                  </button>
                </div>
              </div>
              {deviceFiles !== null && deviceFiles.length > 0 ? (
                <>
                  {/* 「打开文件夹」上移到列表级：8 行里 8 个「在文件夹中显示」指向同一个
                      目录，是纯冗余，也是操作列宽度不稳定的元凶。这里一次解决三件事：
                      列宽恒定、噪音少 7 个元素、"文件去哪了"有了全局答案。 */}
                  <div className="transfer-toolbar">
                    <span>取回后保存在 <span className="transfer-toolbar-dest">下载 / MirrorDock</span></span>
                    <button className="ghost-button" type="button" onClick={() => void openFetchedFolder()}>
                      打开文件夹
                    </button>
                  </div>
                  {transferGroups.map((group) => (
                    <section key={group.kind} className="transfer-group">
                      <h4 className="transfer-group-title">
                        {group.label}
                        {group.kind === "installer" && (
                          <span className="transfer-group-hint">装到手机上的程序文件</span>
                        )}
                        <span className="transfer-group-count">{group.files.length}</span>
                      </h4>
                      {/* 列表级 grid + 行 display:contents → 操作列全列表对齐 */}
                      <ul className="transfer-list">
                        {group.files.map((name) => {
                          const receipt = fetchedFiles[name];
                          const fetching = fetchingName === name;
                          const failure = fetchErrors[name];
                          return (
                            <li key={name} className="transfer-file-row">
                              <div className={`transfer-file-cell${receipt ? " done" : ""}${failure ? " failed" : ""}`}>
                                <span className="transfer-name">
                                  <span className="transfer-file-name" title={name}>{name}</span>
                                  {failure && <span className="transfer-row-error">{failure}</span>}
                                </span>
                              </div>
                              <div className="transfer-row-actions">
                                {fetching ? (
                                  <span className="transfer-row-state busy">正在取回…</span>
                                ) : receipt ? (
                                  <span className="transfer-row-state ok">已取回</span>
                                ) : failure ? (
                                  <button className="text-button" type="button" onClick={() => void fetchDeviceFile(transferDeviceSerial ?? "", name)}>重试</button>
                                ) : (
                                  <button className="text-button" type="button" disabled={fetchingName !== null} onClick={() => void fetchDeviceFile(transferDeviceSerial ?? "", name)}>取回到电脑</button>
                                )}
                                <button className="text-button danger" type="button" disabled={fetchingName !== null} onClick={() => void confirmDeleteDeviceFile(name)}>删除</button>
                              </div>
                            </li>
                          );
                        })}
                      </ul>
                    </section>
                  ))}
                  {/* 计数即可，不再重复"取回不会删除原件"这类防御性解释（删除处已有确认）。 */}
                  <p className="transfer-summary">
                    共 {deviceFiles.length} 个文件
                    {fetchedCount > 0 && `，${fetchedCount} 个已取回`}
                  </p>
                </>
              ) : (
                <p className="capability-pending">
                  {transferBusy || fetchingName
                    ? "正在读取手机上的文件…"
                    : deviceFiles
                      ? "手机的「下载 / MirrorDock」当前没有文件。手机上发送文件到这里后，刷新即可看到。"
                      : "尚未读取。点「刷新」查看手机「下载 / MirrorDock」里的文件。"}
                </p>
              )}
              {transferMessage && <p className="apply-notice" role="status">{transferMessage}</p>}
              {transferError && <p className="capability-pending" role="alert">{transferError}</p>}
            </div>

            {/* X10-74：APK 安装需要已就绪设备（要下发到设备），留在原门禁内。 */}
            {readyDevice && (
              <div className="capability-panel apk-panel" aria-live="polite" style={{ marginTop: 16 }}>
                  <strong>安装 APK</strong>
                  <p className="capability-pending">选择本机的 .apk 安装包，一键装到手机（覆盖安装、保留应用数据）。</p>
                  <span>
                    <button className="secondary-button" type="button" disabled={apkBusy} onClick={() => void pickApk()}>
                      {apkPath ? "重新选择安装包" : "选择 APK 安装包"}
                    </button>
                    <button className="primary-button" type="button" disabled={apkBusy || !apkPath} onClick={() => void installApk(readyDevice.serial)}>
                      {apkBusy ? "正在安装…" : "安装到手机"}
                    </button>
                  </span>
                  {apkPath && <p className="screenshot-path">{apkPath}</p>}
                  {apkReceipt && (
                    <div className="screenshot-result">
                      <p className="capability-summary">{apkReceipt.file_name} · {formatBytes(apkReceipt.bytes)} · {apkReceipt.summary}</p>
                    </div>
                  )}
                  {apkMessage && <p className="apply-notice" role="status">{apkMessage}</p>}
                  {apkError && <p className="capability-pending" role="alert">{apkError}</p>}
                  <p className="capability-pending">安装由手机系统完成，需你在手机上确认（如「安装未知应用」）；安装包只在电脑与手机之间传输。</p>
              </div>
            )}

            {/* X10-66 通知镜像一期：手机转来的通知（伴侣通道，不依赖 USB/ADB 镜像）。
                只在内存展示最近 50 条，不落盘；转发开关在手机端（默认关）。 */}
            <div className="capability-panel notify-panel" aria-live="polite" style={{ marginTop: 16 }}>
              <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <strong>手机通知</strong>
                {phoneNotifications.length > 0 && (
                  <button className="text-button" type="button" onClick={() => setPhoneNotifications([])}>
                    清空
                  </button>
                )}
              </span>
              {phoneNotifications.length === 0 ? (
                <p className="capability-pending">
                  在手机伴侣 App 打开「通知镜像」并授予读取通知权限后，新通知会实时出现在这里（需要手机与电脑的常驻连接在线）。通知只在本窗口临时展示，不保存、不经过任何云端。
                </p>
              ) : (
                <ul className="transfer-file-list">
                  {phoneNotifications.map((item, index) => (
                    <li key={`${item.posted}-${item.pkg}-${index}`} className="notif-item">
                      <span className="transfer-file-name">
                        <strong style={{ marginRight: 6 }}>[{item.app || item.pkg}]</strong>
                        {item.title}
                        {item.text ? `：${item.text}` : ""}
                      </span>
                      <span style={{ marginLeft: "auto", whiteSpace: "nowrap" }}>{notificationTime(item.posted)}</span>
                      {/* X10-69 快捷回复：仅通知自带 RemoteInput 回复动作时展示。 */}
                      {item.replyable && item.key && (
                        <div className="notif-reply">
                          <input
                            value={notifReplyDrafts[item.key] ?? ""}
                            placeholder={`回复 ${item.app || item.pkg}…`}
                            maxLength={500}
                            disabled={notifReplyBusy === item.key}
                            onChange={e => setNotifReplyDrafts(prev => ({ ...prev, [item.key as string]: e.target.value }))}
                            onKeyDown={e => {
                              if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                                void sendNotificationReply({ ...item, key: item.key as string });
                              }
                            }}
                          />
                          <button
                            className="secondary-button"
                            type="button"
                            disabled={notifReplyBusy === item.key || !(notifReplyDrafts[item.key] ?? "").trim()}
                            onClick={() => void sendNotificationReply({ ...item, key: item.key as string })}
                          >
                            {notifReplyBusy === item.key ? "发送中…" : "发送"}
                          </button>
                        </div>
                      )}
                      {notifReplyNotice && notifReplyNotice.key === item.key && (
                        <p className={notifReplyNotice.error ? "capability-pending" : "apply-notice"} role="status">
                          {notifReplyNotice.text}
                        </p>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </section>

          {/* -- 无线 -------------------------------------------------------- */}
          <section className={`tab-panel ${tab === "wireless" ? "" : "panel-hidden"}`} aria-label="无线">
            <div className="page-head">
              <h1>无线连接</h1>
              <p className="intro">同一 Wi-Fi 下连接 Android 11+；推荐扫码配对，全程免输入。</p>
            </div>
            <section className="connection-card" aria-live="polite">
              <div className="wireless-heading">
                <div>
                  <p className="eyebrow">推荐</p>
                  <h2>扫码配对</h2>
                  <p>手机在「无线调试」里点「使用二维码配对设备」，扫下面的二维码即可，配对与连接自动完成。</p>
                </div>
              </div>
              {!qrOffer ? (
                <span>
                  <button className="primary-button" type="button" disabled={qrBusy} onClick={() => void beginQrPairing()}>
                    {qrBusy ? "正在生成…" : "生成配对二维码"}
                  </button>
                </span>
              ) : (
                <div className="screenshot-result">
                  {qrImage && <img src={qrImage} alt="无线调试配对二维码" width={220} height={220} />}
                  <p className="capability-summary">
                    {qrProgress ? qrPairingStageCopy[qrProgress.stage] ?? qrProgress.stage : "等待手机扫码"}
                    {qrProgress?.detail ? ` · ${qrProgress.detail}` : ""}
                  </p>
                  <p className="capability-pending">手机路径：设置 → 开发者选项 → 无线调试 → 使用二维码配对设备。二维码一次有效，可随时取消。</p>
                  <button className="secondary-button" type="button" onClick={() => void stopQrPairing()}>取消</button>
                </div>
              )}
              {qrError && <p className="capability-pending" role="alert">{qrError}</p>}
            </section>
            <section className="connection-card" aria-labelledby="wireless-title">
              <div className="wireless-heading">
                <div>
                  <p className="eyebrow">手动方式</p>
                  <h2 id="wireless-title">配对码配对</h2>
                  <p>扫码不便时，填手机屏幕上的地址和 6 位配对码。配对码不会保存。</p>
                </div>
                <button className="secondary-button" type="button" onClick={() => setWirelessExpanded((value) => !value)}>
                  {wirelessExpanded ? "收起" : "手动配对"}
                </button>
              </div>

              {wirelessExpanded && <div className="wireless-content">
                <ol className="wireless-steps">
                  <li>在手机的“开发者选项”中打开“无线调试”，并确认手机和电脑在同一 Wi-Fi。</li>
                  <li>选择“使用配对码配对设备”，填写手机显示的配对地址和 6 位配对码。</li>
                  <li>回到无线调试主页面，填写“IP 地址和端口”中的连接地址；它可能与配对地址不同。</li>
                </ol>
                <span>
                  <button className="secondary-button" type="button" disabled={discovering} onClick={() => void discoverWirelessEndpoints()}>
                    {discovering ? "正在搜索…" : "自动发现地址（mDNS）"}
                  </button>
                </span>
                {discoverMessage && <p className="apply-notice" role="status">{discoverMessage}</p>}
                <div className="wireless-form">
                  <label>配对地址<input value={pairEndpoint} onChange={(event) => setPairEndpoint(event.target.value)} placeholder="例如 192.168.1.20:37123" autoComplete="off" /></label>
                  <label>6 位配对码<input value={pairingCode} onChange={(event) => setPairingCode(event.target.value.replace(/\D/g, "").slice(0, 6))} inputMode="numeric" placeholder="不会保存" autoComplete="one-time-code" /></label>
                  <label>连接地址<input value={connectEndpoint} onChange={(event) => setConnectEndpoint(event.target.value)} placeholder="例如 192.168.1.20:41839" autoComplete="off" /></label>
                  <button className="primary-button" type="button" onClick={() => void pairAndConnect()} disabled={wirelessBusy}>{wirelessBusy ? "正在连接…" : "配对并连接"}</button>
                </div>
                <p className="capability-pending">也可以停在「使用配对码配对设备」页面点「自动发现地址」，地址会自动填入，6 位配对码仍需人工读取。</p>
              </div>}

              {wirelessMessage && <p className="diagnostic">{wirelessMessage}</p>}
              {trustedDevices.length > 0 && <div className="trusted-devices" aria-label="本机已保存的无线设备">
                <strong>本机已保存的无线设备</strong>
                {trustedDevices.map((device) => <div className="trusted-device" key={device.endpoint}>
                  <code>{device.endpoint}</code>
                  <span>
                    <button className="text-button" type="button" disabled={wirelessBusy} onClick={() => void reconnect(device.endpoint)}>重新连接</button>
                    <button className="text-button danger" type="button" disabled={wirelessBusy} onClick={() => void forgetDevice(device.endpoint)}>忘记</button>
                  </span>
                </div>)}
              </div>}
            </section>

            <section className="connection-card" aria-live="polite">
              <div className="wireless-heading">
                <div>
                  <p className="eyebrow">伴侣 App（实验）</p>
                  <h2>扫码配对</h2>
                  <p>用伴侣 App 扫码，与电脑建立加密助手通道并互相记住身份（桌面端与手机端都保存互信凭据，之后重连无需再扫码）。注意：这条通道与镜像连接相互独立——手机要出现在连接列表里，请用数据线连接，或在上方「无线」区完成配对；已配对过的手机在扫码成功后会自动尝试回连。</p>
                </div>
              </div>
              <span>
                <button className="secondary-button" type="button" disabled={pairingBusy} onClick={() => void beginPairing()}>
                  {pairingBusy && !pairingOffer ? "正在准备…" : pairingOffer ? "重新生成配对" : "开始配对"}
                </button>
                <button className="secondary-button" type="button" disabled={pairingBusy || !pairingOffer} onClick={() => void stopPairing()}>
                  结束配对
                </button>
              </span>
              {/* X10-64：常驻端口只在扫码那一刻交给手机，顺序反了就会走进
                  「没有可直连的电脑」的死胡同——在扫码入口旁边先说清顺序。 */}
              {!residentActive && (
                <p className="capability-pending">提示：想让手机用一键免扫码重连，请先在下方开启「常驻通道」，再让手机扫码——通道端口只在扫码配对时交给手机。</p>
              )}
              {pairingOffer && (
                <div className="screenshot-result">
                  <p className="capability-summary">
                    配对状态：{pairingStatus?.phase === "connected" ? "伴侣已连接" : pairingStatus?.phase === "listening" ? "等待伴侣扫码" : "未开始"}
                    {" · 一次性配对码 "}
                    <code>{pairingOffer.token}</code>
                    {pairingStatus?.phase === "connected" && (
                      <span className="capability-pending">（这是伴侣助手通道；镜像连接请在设备卡片上操作）</span>
                    )}
                  </p>
                  {pairingQr
                    ? <img src={pairingQr} alt="伴侣 App 配对二维码" width={220} height={220} />
                    : <p className="capability-pending">二维码渲染不可用时，可在伴侣 App 中手动输入上方 16 位配对码与本机地址（{pairingOffer.hosts[0]}:{pairingOffer.port}）。</p>}
                  {pairingStatus && pairingStatus.events.length > 0 && (
                    <ul className="transfer-file-list">
                      {pairingStatus.events.slice(-8).reverse().map((event) => (
                        <li key={event}><span className="transfer-file-name">{event}</span></li>
                      ))}
                    </ul>
                  )}
                </div>
              )}
              {pairingError && <p className="capability-pending" role="alert">{pairingError}</p>}
              {pairedDevices.length > 0 && <div className="trusted-devices" aria-label="已配对的伴侣设备">
                <strong>已配对的伴侣设备</strong>
                {pairedDevices.map((device) => <div className="trusted-device" key={device.pairing_id}>
                  <code>{device.model}</code>
                  <span>
                    <button className="text-button danger" type="button" onClick={() => void unpairDevice(device.pairing_id)}>移除互信</button>
                  </span>
                </div>)}
                <p className="capability-pending">移除后，该设备再次连接需要重新扫码配对。</p>
              </div>}

              {/* M4-2 常驻通道：开启后已配对设备在伴侣 App 内一键免扫码直连。 */}
              <div className="trusted-devices" aria-label="常驻通道">
                <strong>常驻通道（免扫码重连）</strong>
                <p className="capability-pending">开启后，已配对的伴侣设备打开 App 里的「连接上次配对的电脑」，即可直接建立互信会话并尝试自动回连镜像通道，无需重新扫码。要注意先后顺序：通道端口是在扫码配对的那一刻交给手机的，电脑只会把端口变化告诉当前连着的手机——所以请先开启常驻通道、再在手机上扫码；如果手机是在关闭状态下配过对，需要在手机上重新扫一次码（电脑这边不用改）。关闭常驻通道后，已配对的手机会如实显示「没有可直连的电脑」。</p>
                <span>
                  <button className="secondary-button" type="button" disabled={residentBusy || residentActive} onClick={() => void startResident()}>
                    开启常驻通道
                  </button>
                  <button className="secondary-button" type="button" disabled={residentBusy || !residentActive} onClick={() => void stopResident()}>
                    关闭常驻通道
                  </button>
                </span>
                {residentActive && <p className="capability-summary">常驻监听中{residentPort ? `（端口 ${residentPort}）` : ""} · 等待伴侣设备连接。</p>}
                {residentNotice && <p className="capability-summary">{residentNotice}</p>}
                {residentMessage && <p className="capability-pending" role="alert">{residentMessage}</p>}
                {residentActive && pairingStatus && pairingStatus.events.length > 0 && (
                  <ul className="transfer-file-list">
                    {pairingStatus.events.slice(-8).reverse().map((event) => (
                      <li key={event}><span className="transfer-file-name">{event}</span></li>
                    ))}
                  </ul>
                )}
              </div>
            </section>
          </section>

          {/* -- 设置 -------------------------------------------------------- */}
          <section className={`tab-panel ${tab === "settings" ? "" : "panel-hidden"}`} aria-label="设置">
            <div className="page-head">
              <h1>设置</h1>
              <p className="intro">镜像窗口、快捷键与授权。</p>
            </div>

            <section className="settings-card">
              <header className="settings-card-head">
                <div><h2>通用</h2><p>窗口关闭行为、断线自动重连、菜单栏/托盘与开机启动。</p></div>
              </header>
              <div className="settings-rows">
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">开机自动启动 MirrorDock</span>
                    <span className="setting-desc">登录本机后自动运行，无需手动打开。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="开机自动启动" checked={autostartEnabled} onChange={e => void toggleAutostart(e.target.checked)} /></label>
                </div>
                {isMac && (
                  <div className="setting-row">
                    <div className="setting-info">
                      <span className="setting-name">隐藏 Dock 图标</span>
                      <span className="setting-desc">只保留屏幕顶部菜单栏图标，从菜单栏回到主窗口。</span>
                    </div>
                    <label className="setting-toggle"><input type="checkbox" aria-label="隐藏 Dock 图标" checked={appSettings.hide_dock_icon} onChange={e => void toggleHideDockIcon(e.target.checked)} /></label>
                  </div>
                )}
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">断线自动重连</span>
                    <span className="setting-desc">镜像意外断开时自动等待连接恢复并重建镜像（最多 15 分钟）：无线掉线等手机回网，数据线被拔掉等重新插线。手动关闭镜像窗口不会触发。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="断线自动重连" checked={appSettings.auto_reconnect} onChange={e => void toggleAutoReconnect(e.target.checked)} /></label>
                </div>
                <p className="setting-note">点窗口关闭按钮 = 最小化到菜单栏/托盘，不会结束镜像会话。菜单栏/托盘里可以：连接/断开、开始/结束屏幕录制、屏幕唤醒、手机截图。</p>
                <p className="setting-note">镜像画面黑屏（手机熄屏）时，在镜像窗口上点右键即可直接点亮屏幕，不必回到本窗口。</p>
                {generalNotice && <p className="setting-note apply-notice" role="status">{generalNotice}</p>}
              </div>
            </section>

            <section className="settings-card">
              <header className="settings-card-head">
                <div><h2>镜像窗口</h2><p>{sessionActive ? "会话中修改需重启镜像窗口" : "开始镜像时生效"}</p></div>
                <div className="settings-actions">
                  {sessionActive && (
                    <button type="button" className="secondary-button" disabled={applyingOptions} onClick={() => void applySessionOptions()}>
                      {applyingOptions ? "正在应用…" : "应用并重启镜像窗口"}
                    </button>
                  )}
                  <button type="button" className="secondary-button" onClick={resetAllOptions}>恢复默认设置</button>
                </div>
              </header>
              <div className="settings-rows">
                {sessionActive && activeSessionList.length > 1 && (
                  <div className="setting-row">
                    <div className="setting-info">
                      <span className="setting-name">应用到哪台设备</span>
                      <span className="setting-desc">多台设备镜像中：改动只作用于这里选中的一台，其他设备不受影响。</span>
                    </div>
                    <AppSelect
                      ariaLabel="设置应用目标设备"
                      value={settingsTargetSerial ?? session?.serial ?? ""}
                      options={activeSessionList.map((item) => ({
                        value: item.serial,
                        label: check?.devices.find((device) => device.serial === item.serial)?.label ?? item.serial,
                      }))}
                      onChange={(next) => setSettingsTargetSerial(next)}
                    />
                  </div>
                )}
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">画质</span>
                    <span className="setting-desc">分辨率与码率越高越清晰，对电脑与手机性能要求也越高。</span>
                  </div>
                  <AppSelect
                    ariaLabel="画质"
                    value={options.quality}
                    options={[
                      { value: "smooth", label: "流畅 · 1024 / 2 Mbps" },
                      { value: "balanced", label: "均衡 · 1920 / 8 Mbps" },
                      { value: "sharp", label: "清晰 · 2560 / 16 Mbps" },
                    ]}
                    onChange={(next) => updateOptions({...options, quality: next as SessionOptions["quality"]})}
                  />
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">帧率上限</span>
                    <span className="setting-desc">默认跟随设备帧率。老手机发烫卡顿时选 30，长会话省电选 24。</span>
                  </div>
                  <AppSelect
                    ariaLabel="帧率上限"
                    value={String(options.max_fps ?? 0)}
                    options={[
                      { value: "0", label: "跟随设备（不限）" },
                      { value: "60", label: "最高 60 帧" },
                      { value: "30", label: "最高 30 帧" },
                      { value: "24", label: "最高 24 帧（最省电）" },
                    ]}
                    onChange={(next) => updateOptions({...options, max_fps: Number(next) === 0 ? null : Number(next)})}
                  />
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">使用手机后置摄像头画面</span>
                    <span className="setting-desc">把手机摄像头当作电脑上的摄像头画面（网课、会议场景）。仅在你显式开启时使用摄像头，且不采集任何麦克风声音；与桌面模式互斥，更改后重启会话生效。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="使用手机后置摄像头画面" checked={options.camera_source} onChange={e => updateOptions({ ...options, camera_source: e.target.checked, desktop_mode: e.target.checked ? false : options.desktop_mode })} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">显示方向</span>
                    <span className="setting-desc">「自动」跟随手机旋转，打开横屏游戏会自动转为横屏。</span>
                  </div>
                  <AppSelect
                    ariaLabel="显示方向"
                    value={String(options.rotation)}
                    options={[
                      { value: "0", label: "自动（跟随手机）" },
                      { value: "90", label: "锁定 90°" },
                      { value: "180", label: "锁定 180°" },
                      { value: "270", label: "锁定 270°" },
                    ]}
                    onChange={(next) => updateOptions({...options, rotation: Number(next)})}
                  />
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">全屏启动</span>
                    <span className="setting-desc">镜像窗口直接铺满整个屏幕。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="全屏启动" checked={options.fullscreen} onChange={e => updateOptions({...options, fullscreen: e.target.checked})} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">窗口置顶</span>
                    <span className="setting-desc">镜像窗口始终浮在其他窗口上方。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="窗口置顶" checked={options.always_on_top} onChange={e => updateOptions({...options, always_on_top: e.target.checked})} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">会话期间保持手机唤醒</span>
                    <span className="setting-desc">镜像进行中手机不会自动熄屏，也不会变暗，锁屏页有充足时间输入解锁密码。USB 与无线均生效（无线依靠 scrcpy 保持唤醒，不会伪造充电状态，状态栏不会显示充电中）；屏幕即将变暗时也会自动恢复亮度；会话结束即恢复原设置。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="会话期间保持手机唤醒" checked={options.keep_awake} onChange={e => updateOptions({...options, keep_awake: e.target.checked})} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">启用视频录制</span>
                    <span className="setting-desc">勾选则开启录制功能；何时开始、何时结束由你在「工具」页或快捷键（{shortcuts.record}）决定，镜像画面不会中断。MP4 保存在本机视频目录。{!proEdition && "专业版功能，在下方「版本与授权」激活后可用"}</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="启用视频录制" checked={options.record} disabled={!proEdition} onChange={e => updateOptions({...options, record: e.target.checked})} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">录像保存位置</span>
                    <span className="setting-desc">{appSettings.recording_dir ? `当前：${appSettings.recording_dir}` : "默认保存在本机视频目录下的 MirrorDock 文件夹。"}</span>
                  </div>
                  <div className="setting-actions">
                    <button className="text-button" type="button" aria-label="选择录像保存文件夹" onClick={() => void chooseRecordingDir()}>选择文件夹</button>
                    {appSettings.recording_dir && <button className="text-button" type="button" aria-label="恢复默认录像位置" onClick={() => void resetRecordingDir()}>恢复默认</button>}
                  </div>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">双向同步剪贴板</span>
                    <span className="setting-desc">在电脑和手机之间直接复制粘贴。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="双向同步剪贴板" checked={options.clipboard_autosync} onChange={e => updateOptions({...options, clipboard_autosync: e.target.checked})} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">键盘直输（手机不弹全屏键盘）</span>
                    <span className="setting-desc">开启后手机把电脑当作外接键盘：点输入框不再弹出全屏软键盘，只在屏幕底部留一条小候选栏，直接用电脑键盘打字即可。关闭则由电脑把组好的文字直接注入手机，手机软键盘照常弹出。更改后重启会话生效。打字没反应或字符全错时，通常是手机端「实体键盘」布局未启用「英语（美国）」——点右侧按钮到手机上确认。</span>
                  </div>
                  <div className="settings-actions">
                    <button type="button" className="secondary-button" disabled={!readySerial} aria-label="打开手机的实体键盘设置" onClick={() => void openKeyboardSettings()}>手机键盘设置</button>
                    <label className="setting-toggle"><input type="checkbox" aria-label="键盘直输（手机不弹全屏键盘）" checked={options.keyboard_uhid} onChange={e => updateOptions({...options, keyboard_uhid: e.target.checked})} /></label>
                  </div>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">转发手机播放的声音</span>
                    <span className="setting-desc">{audioUnsupported ? "这台手机不支持系统音频转发，已自动关闭。" : "声音在电脑播放、手机静音（Android 11+）。"}</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="转发手机播放的声音" checked={options.audio} disabled={audioUnsupported} onChange={e => updateOptions({...options, audio: e.target.checked})} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">显示触摸点</span>
                    <span className="setting-desc">画面上显示点按位置，适合演示与录屏。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="显示触摸点" checked={options.show_touches} onChange={e => updateOptions({...options, show_touches: e.target.checked})} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">只读模式</span>
                    <span className="setting-desc">电脑键鼠只看不控，避免误操作手机。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="只读模式" checked={options.read_only} onChange={e => updateOptions({...options, read_only: e.target.checked})} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">镜像窗口快捷键修饰键</span>
                    <span className="setting-desc">窗口内组合键的修饰键，如 +H 主屏幕、+B 返回、+O 熄屏（镜像继续）。</span>
                  </div>
                  <AppSelect
                    ariaLabel="镜像窗口快捷键修饰键"
                    value={options.shortcut_mod ?? ""}
                    options={[
                      { value: "", label: "默认（左 Alt / 左 Super）" },
                      { value: "lctrl", label: "左 Ctrl" },
                      { value: "rctrl", label: "右 Ctrl" },
                      { value: "lalt", label: "左 Alt" },
                      { value: "ralt", label: "右 Alt" },
                      { value: "lsuper", label: "左 Super（Win / ⌘）" },
                      { value: "rsuper", label: "右 Super" },
                    ]}
                    onChange={(next) => updateOptions({...options, shortcut_mod: next || null})}
                  />
                </div>
                <p className="setting-note">窗口内建快捷键：+H 主屏幕、+B 返回、+S 最近任务、+N 通知栏、+P 电源、+O 熄屏（镜像继续）、+↑/↓ 音量、+F 全屏、+Q 退出。</p>
                <p className="setting-note">受保护内容（支付、密码页）系统会屏蔽为黑屏；会话进行中的全局快捷键（无需切回本窗口）可在下方「全局快捷键」中自定义。</p>
                {sessionActive && <p className="setting-note">镜像窗口形态在启动时确定，运行中修改需重启窗口，画面会短暂中断。</p>}
                {applyNotice && <p className="setting-note apply-notice" role="status">{applyNotice}</p>}
              </div>
            </section>

            {/* X10-72：桌面模式是设备级能力，独立成卡。旧实现把它塞在「镜像窗口」
                全局设置里，只在多会话时给一个设备下拉，单设备场景下读写落到全局 —
                用户看到的就是一个"全局开关"，找不到按设备设置的地方。现在：
                ① 顶部设备选择器列出**所有**已知设备（含未镜像、未就绪、已有历史配置的）
                ② 明确显示"下面两项属于哪台设备"
                ③ 设备列表每行直接显示该设备当前的桌面模式状态，点行即切换编辑对象
                ④ 只有在一台设备都没有时，才回落全局默认并如实写明。 */}
            <section className="settings-card">
              <header className="settings-card-head">
                <div>
                  <h2>按设备设置 · 桌面模式</h2>
                  <p>桌面模式与虚拟屏启动的应用按设备单独保存，各设备互不影响。</p>
                </div>
              </header>
              <div className="settings-rows">
                {desktopPrefDevices.length > 0 ? (
                  <>
                    <div className="setting-row">
                      <div className="setting-info">
                        <span className="setting-name">设置哪台设备</span>
                        <span className="setting-desc">
                          下面两项只作用于这里选中的设备。已连接的手机会排在前面；没有插上的、但以前设置过的设备也会列出，方便随时调整。
                        </span>
                      </div>
                      <AppSelect
                        ariaLabel="桌面模式设置目标设备"
                        value={settingsDesktopPrefKey ?? ""}
                        options={desktopPrefDevices.map((device) => ({
                          value: device.pref_key,
                          label: `${device.label}（${device.state === "ready" ? "已就绪" : device.state === "unauthorized" ? "未授权" : device.state === "offline" ? "离线" : "未知"}${device.streaming ? " · 镜像中" : ""}${device.configured ? " · 已设置桌面模式" : ""}）`,
                        }))}
                        onChange={(next) => setDesktopTargetSerial(next || null)}
                      />
                    </div>
                    <div className="device-pref-list" role="list" aria-label="各设备桌面模式状态">
                      {desktopPrefDevices.map((device) => {
                        const pref = desktopPrefs[device.pref_key];
                        const on = pref ? pref.desktop_mode : options.desktop_mode;
                        return (
                          <button
                            type="button"
                            role="listitem"
                            key={device.pref_key}
                            className={`device-pref-item${device.pref_key === settingsDesktopPrefKey ? " active" : ""}`}
                            onClick={() => setDesktopTargetSerial(device.pref_key)}
                          >
                            <span className="device-pref-name">
                              {device.label}
                              {device.streaming && <em className="device-pref-badge live">镜像中</em>}
                              {!device.configured && <em className="device-pref-badge">未单独设置</em>}
                            </span>
                            <span className={`device-pref-state${on ? " on" : ""}`}>{on ? "桌面模式 开" : "桌面模式 关"}</span>
                          </button>
                        );
                      })}
                    </div>
                    <p className="setting-note" role="status">
                      下面两项正在编辑：<strong>{settingsDesktopTargetLabel ?? "（未选择设备）"}</strong>
                    </p>
                  </>
                ) : (
                  <p className="setting-note">
                    当前没有检测到任何设备。下面两项会作为<strong>全局默认</strong>保存 —— 新接入的设备在没有单独设置之前都按它启动。插上手机并完成调试授权后，这里会变成按设备单独保存。
                  </p>
                )}
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">桌面模式（独立虚拟屏幕）</span>
                    <span className="setting-desc">
                      不再镜像手机现有屏幕，而是在手机上创建一块独立虚拟屏幕：电脑上全屏看视频、写笔记，手机上回微信也不打断画面。需要 Android 10+，手机端会弹出「显示在其他应用上层」的确认。与摄像头画面互斥，更改后重启会话生效。
                      {settingsDesktopTarget ? `仅对「${settingsDesktopTargetLabel}」这台设备生效。` : "当前保存为全局默认。"}
                    </span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="桌面模式（独立虚拟屏幕）" checked={shownDesktopMode} onChange={e => {
                    const checked = e.target.checked;
                    if (settingsDesktopTarget) {
                      // 这台设备自己的桌面模式偏好，不影响其他设备与全局默认。
                      // X10-84：写入键统一为 physical_serial。
                      if (settingsDesktopPrefKey) updateDesktopPref(settingsDesktopPrefKey, { desktop_mode: checked });
                      if (checked && options.camera_source) updateOptions({ ...options, camera_source: false });
                    } else {
                      updateOptions({ ...options, desktop_mode: checked, camera_source: checked ? false : options.camera_source });
                    }
                  }} /></label>
                </div>
                {shownDesktopMode && (
                  <div className="setting-row">
                    <div className="setting-info">
                      <span className="setting-name">虚拟屏启动的应用（可选）</span>
                      <span className="setting-desc">
                        实测部分机型（如小米/MIUI）的桌面不在虚拟屏上显示（会得到白屏/黑屏）；下拉按应用名称选择（例如「浏览器」），也可以直接填包名如 com.android.browser。留空则显示系统桌面。
                        {currentDesktopAppName && `当前已选：${currentDesktopAppName}。`}
                        {settingsDesktopTarget ? `仅对「${settingsDesktopTargetLabel}」这台设备生效。` : "当前保存为全局默认。"}
                      </span>
                    </div>
                    <input
                      className="setting-control"
                      aria-label="虚拟屏启动的应用"
                      list="device-app-packages"
                      placeholder="com.android.browser"
                      value={shownDesktopApp ?? ""}
                      onChange={e => {
                        const pkg = e.target.value.trim();
                        const next = pkg ? pkg : null;
                        if (settingsDesktopTarget && settingsDesktopPrefKey) updateDesktopPref(settingsDesktopPrefKey, { desktop_mode: true, desktop_app: next });
                        else updateOptions({ ...options, desktop_app: next });
                      }}
                    />
                  </div>
                )}
                {shownDesktopMode && (
                  <datalist id="device-app-packages">
                    {desktopAppsForTarget.slice(0, 500).map(app => (
                      <option key={app.package} value={app.package}>{app.name === app.package ? undefined : app.name}</option>
                    ))}
                  </datalist>
                )}
              </div>
            </section>

            <section className="settings-card">
              <header className="settings-card-head">
                <div><h2>全局快捷键</h2><p>镜像运行中全局生效，无需切回本窗口（Windows 用 Ctrl，macOS 用 ⌘）。</p></div>
                <div className="settings-actions">
                  <button type="button" className="secondary-button" onClick={() => updateShortcuts({ ...defaultShortcuts })}>恢复默认</button>
                </div>
              </header>
              <div className="shortcut-grid">
                <label className="shortcut-row"><span>截图</span><input value={shortcuts.screenshot} onChange={e => updateShortcuts({ ...shortcuts, screenshot: e.target.value })} placeholder="CommandOrControl+Alt+S" autoComplete="off" spellCheck={false} /></label>
                <label className="shortcut-row"><span>录制开关</span><input value={shortcuts.record} onChange={e => updateShortcuts({ ...shortcuts, record: e.target.value })} placeholder="CommandOrControl+Alt+R" autoComplete="off" spellCheck={false} /></label>
                <label className="shortcut-row"><span>轮换方向</span><input value={shortcuts.rotate} onChange={e => updateShortcuts({ ...shortcuts, rotate: e.target.value })} placeholder="CommandOrControl+Alt+D" autoComplete="off" spellCheck={false} /></label>
              </div>
              {(!isValidShortcut(shortcuts.screenshot) || !isValidShortcut(shortcuts.record) || !isValidShortcut(shortcuts.rotate)) && (
                <p className="setting-note" role="alert">格式无效：至少一个修饰键（Ctrl/Alt/⌘ 等）加一个普通键。无效的组合不会生效。</p>
              )}
              <p className="setting-note">格式：修饰键+按键，如 Ctrl+Alt+S。修改立即保存；镜像运行中会自动改用新组合，无需重启。</p>
            </section>

            <section className="settings-card">
              <header className="settings-card-head">
                <div><h2>版本与授权</h2><p>当前版本：{editionLabel(entitlement?.edition)}{proEdition && entitlement?.key_id ? `（许可证 ${entitlement.key_id}，${expiryText(entitlement.expires_at)}）` : ""}</p></div>
              </header>
              <div className="settings-rows">
                {proEdition ? (
                  <>
                    <p className="setting-note">专业版已激活：MP4 录制可用。授权状态保存在本机，激活与使用都不需要联网账号。</p>
                    <div className="settings-actions license-action">
                      <button type="button" className="secondary-button" disabled={licenseBusy} onClick={() => void deactivateLicense()}>
                        {licenseBusy ? "正在处理…" : "撤销本机授权"}
                      </button>
                    </div>
                  </>
                ) : (
                  <>
                    <div className="license-input-row">
                      <label className="shortcut-row"><span>许可证</span><input value={licenseInput} onChange={e => setLicenseInput(e.target.value)} placeholder="MD1-XXXXXX-XXXXXX-…" autoComplete="off" spellCheck={false} /></label>
                      <button type="button" className="primary-button" disabled={licenseBusy || !licenseInput.trim()} onClick={() => void activateLicense()}>
                        {licenseBusy ? "正在激活…" : "激活专业版"}
                      </button>
                    </div>
                    <p className="setting-note">免费版含全部镜像、截图与传输功能；专业版解锁 MP4 录制。离线激活，只保存在本机。</p>
                  </>
                )}
                {licenseMessage && <p className="setting-note" role="status">{licenseMessage}</p>}
                {licenseError && <p className="diagnostic" role="alert">{licenseError}</p>}
              </div>
            </section>

            {/* 关于：版本、开发者、许可与项目链接（X10-41）。 */}
            <section className="settings-card">
              <header className="settings-card-head">
                <div><h2>关于 MirrorDock</h2><p>版本、开源许可与项目链接。</p></div>
              </header>
              <div className="settings-rows">
                <p className="setting-note">当前版本：{appVersion || "—"}（测试版）</p>
                <p className="setting-note">开发者：MirrorDock 项目（g-star1024），个人开源项目，欢迎在仓库提 Issue 反馈问题。</p>
                <p className="setting-note">开源许可：Apache-2.0。镜像引擎基于 scrcpy（Apache-2.0）；全部第三方组件的版权声明见安装目录内的 THIRD_PARTY_NOTICES 文件。</p>
                <p className="setting-note">隐私承诺：本地优先，画面与文件只经过你的数据线或局域网，不经过任何服务器。</p>
                <p className="setting-note">检查更新：发现新版本后自动下载安装（更新包经数字签名校验），安装完成后重启应用即完成升级。</p>
                {updateMessage && <p className="setting-note apply-notice" role="status">{updateMessage}</p>}
                <div className="settings-actions">
                  <button type="button" className="secondary-button" onClick={() => void openUrl("https://g-star1024.github.io/MirrorDock/")}>
                    打开官网
                  </button>
                  <button type="button" className="secondary-button" onClick={() => void openUrl("https://github.com/g-star1024/MirrorDock")}>
                    GitHub 仓库
                  </button>
                  <button type="button" className="secondary-button" disabled={updateState !== "idle"} onClick={() => void checkForUpdates()}>
                    {updateState === "idle" ? "检查更新" : updateState === "checking" ? "正在检查…" : "正在更新…"}
                  </button>
                </div>
              </div>
            </section>
          </section>

          {/* -- 帮助 -------------------------------------------------------- */}
          <section className={`tab-panel ${tab === "help" ? "" : "panel-hidden"}`} aria-label="帮助">
            <div className="page-head">
              <h1>帮助与诊断</h1>
              <p className="intro">常用问题都在下面的帮助主题里；联系支持时先在这里预览诊断内容，确认无误后再导出。</p>
            </div>

            {/* 内置帮助文档：随应用分发、离线可读。 */}
            <div className="help-list">
              {helpArticles.map((article) => (
                <div className="capability-panel help-article" key={article.id} aria-live="polite">
                  <strong>{article.title}</strong>
                  {article.paragraphs.map((paragraph, index) => (
                    <p className="capability-pending" key={index}>{paragraph}</p>
                  ))}
                </div>
              ))}
            </div>

            <section className="connection-card" aria-live="polite">
              <strong>诊断包</strong>
              <p className="capability-pending">仅含版本、系统与最近操作结果（已抹去序列号、配对码与路径）。先预览，确认后才保存；不会自动上传任何内容。</p>
              <span>
                <button className="secondary-button" type="button" disabled={diagnosticsBusy} onClick={() => void previewDiagnostics()}>
                  {diagnosticsBusy ? "正在处理…" : diagnostics ? "刷新预览" : "预览诊断内容"}
                </button>
                <button className="secondary-button" type="button" disabled={diagnosticsBusy || !diagnostics} onClick={() => void exportDiagnostics()}>
                  导出为文件
                </button>
              </span>
              {diagnostics && (
                <div className="screenshot-result">
                  <p className="capability-summary">{diagnostics.app_version} · {diagnostics.system} · 镜像引擎{diagnostics.scrcpy_available ? "可用" : "不可用"} · {diagnostics.events.length} 条最近事件</p>
                  {diagnostics.events.length > 0 && (
                    <ul className="transfer-file-list">
                      {diagnostics.events.slice().reverse().map((event) => (
                        <li key={`${event.timestamp_ms}-${event.kind}-${event.code}`}>
                          <span className="transfer-file-name">{new Date(event.timestamp_ms).toLocaleString()} · {event.kind} · {event.code} · {event.detail}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              )}
              {diagnosticsMessage && <p className="apply-notice" role="status">{diagnosticsMessage}</p>}
              {diagnosticsError && <p className="capability-pending" role="alert">{diagnosticsError}</p>}
            </section>
            <aside className="privacy-note">
              <strong>隐私承诺</strong>
              <p>本地优先：数据只经过你的数据线或局域网，不经过任何服务器。</p>
            </aside>
          </section>
        </div>
      </div>
    </div>
  );
}

export default App;
