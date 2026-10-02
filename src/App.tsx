import { useEffect, useRef, useState, type ReactElement } from "react";
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
export function relativeTime(seconds: number) {
  if (!seconds) return "使用时间未知";
  const diff = Date.now() / 1000 - seconds;
  if (diff < 60) return "刚刚使用";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前使用`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前使用`;
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
export type AppSettingsView = { hide_dock_icon: boolean; auto_reconnect: boolean };
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

/** X10-66 通知镜像一期：手机转来的通知（内容只在内存，不落盘）。 */
export type PhoneNotification = { pkg: string; app: string; title: string; text: string; posted: number };

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
  const [recentDevices, setRecentDevices] = useState<RecentDevice[]>([]);
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
  const [lastTransfer, setLastTransfer] = useState<TransferReceipt | null>(null);
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
    }, 5000);
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
        if (!disposed) setAppSettings({ hide_dock_icon: settings.hide_dock_icon === true, auto_reconnect: settings.auto_reconnect !== false });
      } catch { if (!disposed) setAppSettings({ hide_dock_icon: false, auto_reconnect: true }); }
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
      setAppSettings({ hide_dock_icon: saved.hide_dock_icon === true, auto_reconnect: saved.auto_reconnect !== false });
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
      setAppSettings({ hide_dock_icon: saved.hide_dock_icon === true, auto_reconnect: saved.auto_reconnect !== false });
      setGeneralNotice(enabled ? "已开启断线自动重连：无线掉线或数据线被拔掉后，会等待连接恢复并自动重建镜像（最多 15 分钟）。" : "已关闭自动重连：断开后需要手动重新连接。");
    } catch (error) {
      setAppSettings(previous);
      setGeneralNotice(errorMessage(error, "无法修改自动重连设置。"));
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

  // 桌面模式「虚拟屏启动的应用」候选（X10-53）：首次勾选时拉取一次第三方应用
  // 包名，供输入框联想。拉取失败不阻塞——用户仍可手动填写包名。
  const [deviceApps, setDeviceApps] = useState<string[] | null>(null);
  async function loadDeviceApps(serial: string) {
    try {
      const apps = await invoke<string[]>("list_device_apps", { serial });
      setDeviceApps(apps);
    } catch {
      setDeviceApps([]);
    }
  }

  function updateOptions(next: SessionOptions) {
    setOptions(next);
    // 设置一旦被改动，上一次「已应用 / 无需重启」的结论就不再适用，先清掉避免误导。
    setApplyNotice(null);
    try { localStorage.setItem("mirrordock.sessionOptions", JSON.stringify(next)); }
    catch { setSettingsNotice("本机设置无法保存，本次会话仍可使用这些选项。"); }
  }

  // 会话进行中应用新设置：镜像窗口会按新参数重新打开，画面会短暂中断。
  async function applyOptionsUpdate(next: SessionOptions) {
    updateOptions(next);
    setApplyingOptions(true);
    setApplyNotice(null);
    try {
      const result = await invoke<SessionUpdate>("update_session_options", {
        options: next,
        // 只有开启录制时才生成文件名；后端在未开启录制时会忽略它。
        recordFileName: next.record ? recordingFileName(new Date()) : null,
        // X10-27：设置作用于主会话（真正持有镜像进程的设备优先）；后端兼容不传。
        serial: session?.serial ?? null,
      });
      setSessions((prev) => {
        if (result.session.serial == null) return prev;
        const next2 = prev.filter((item) => item.serial !== result.session.serial);
        return [...next2, result.session].sort((a, b) => (a.serial ?? "").localeCompare(b.serial ?? ""));
      });
      setApplyNotice(result.applied ? "新设置已生效：镜像窗口已按新设置重新打开。" : result.note ?? "设置与当前会话一致，未重启镜像窗口。");
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
      const next = { ...current.options, record: !current.options.record };
      if (current.sessionActive) void applyOptionsUpdate(next);
      else updateOptions(next);
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

  // 设备列表自动轮询：无线设备（尤其同 Wi-Fi 自动重连、或手机端已配对的连接回连）
  // 常在本客户端的显式连接流程之外出现，不轮询就只有手动点「重新检查」才看得到。
  // 走 silent=true，只刷新界面、不写诊断日志。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    const timer = window.setInterval(async () => {
      try {
        const next = await invoke<AdbCheck>("check_adb_devices", { silent: true });
        if (!disposed) setCheck(next);
      } catch {
        // 轮询失败保持上一次结果，不闪烁、不打断。
      }
    }, 5000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
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

  async function refreshRecentDevices() {
    try {
      setRecentDevices(await invoke<RecentDevice[]>("list_recent_devices"));
    } catch (error) {
      setRecentDevices([]);
      setRecentMessage(errorMessage(error, "无法读取本机最近使用的设备记录。"));
    }
  }

  // 只移除这条本地记录：不断开连接、不忘记无线配对、不撤销手机上的调试授权。
  async function forgetRecentDevice(serial: string) {
    setRecentMessage(null);
    try {
      setRecentDevices(await invoke<RecentDevice[]>("forget_recent_device", { serial }));
      setRecentMessage("已从本机的最近使用记录中移除。");
    } catch (error) {
      setRecentMessage(errorMessage(error, "无法移除这条记录。"));
    }
  }

  // 一键清空全部最近使用记录（物理删除，落盘空列表）。同样只影响本地记录本身。
  async function clearRecentDevices() {
    setRecentMessage(null);
    try {
      await invoke("clear_recent_devices");
      setRecentDevices([]);
      setRecentMessage("已清空全部最近使用记录。之后再次镜像会重新记入用过的设备。");
    } catch (error) {
      setRecentMessage(errorMessage(error, "无法清空最近设备记录。"));
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
      await invoke("start_mirroring", {
        serial,
        options,
        // 只有开启录制时才生成文件名；后端在未开启录制时会忽略它。
        recordFileName: options.record ? recordingFileName(new Date()) : null,
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
    setLastTransfer(null);
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
      setLastTransfer(receipt);
      setTransferMessage("已发送到手机的「下载 / MirrorDock」文件夹。");
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
  async function fetchDeviceFile(serial: string, fileName: string) {
    setTransferBusy(true);
    setTransferMessage(null);
    setTransferError(null);
    setLastTransfer(null);
    try {
      const receipt = await invoke<TransferReceipt>("fetch_file_from_device", { serial, fileName });
      setLastTransfer(receipt);
      setTransferMessage("已保存到这台电脑的「下载 / MirrorDock」文件夹。");
      // 文件已安全落在本机，顺带问一下是否清理手机上的原件（X10-60 发送区清理）。
      setDeviceFiles(await invoke<string[]>("list_device_files", { serial }));
    } catch (error) {
      setTransferError(errorMessage(error, "文件没有从手机取回。"));
    } finally {
      setTransferBusy(false);
    }
  }

  // 删除手机发送区里的一个文件（X10-60）：只删「下载 / MirrorDock」内的这个文件。
  async function deleteDeviceFile(serial: string, fileName: string) {
    setTransferBusy(true);
    setTransferMessage(null);
    setTransferError(null);
    try {
      await invoke("delete_device_file", { serial, fileName });
      setDeviceFiles(await invoke<string[]>("list_device_files", { serial }));
      setTransferMessage(`已从手机删除：${fileName}`);
    } catch (error) {
      setTransferError(errorMessage(error, "文件没有从手机上删除。"));
    } finally {
      setTransferBusy(false);
    }
  }

  async function revealTransfer() {
    if (!lastTransfer) return;
    try {
      await revealItemInDir(lastTransfer.path);
    } catch (error) {
      setTransferError(errorMessage(error, "无法打开文件所在的文件夹。"));
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
  const displayLabels = buildDisplayLabels(check?.devices ?? []);
  const scrcpyReady = check?.scrcpy_available ?? false;
  const readySerial = readyDevice?.serial ?? null;
  // 快捷键处理器读取的最新 serial。
  shortcutsRef.current.readySerial = readySerial;
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
  // 收到新文件事件后的实际动作：列表开着就刷新；没开着只留提示，用户打开列表时自然会看到。
  useEffect(() => {
    if (filesChangedHint === null) return;
    setTransferMessage("手机发送区有新文件到达。");
    if (readyDevice && deviceFiles !== null && !transferBusy) {
      void refreshDeviceFiles(readyDevice.serial);
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
    }, 5000);
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
                    return (
                      <div className={`device-card ${owned ? "device-card-active" : ""}`} key={device.serial}>
                        <span className={`status-dot ${owned ? "ready" : device.state}`} aria-hidden="true" />
                        <div>
                          <strong>
                            {displayLabels[device.serial] ?? device.label}
                            {badge && <span className="conn-badge inline">{badge}</span>}
                            {lockStamp && <span className="conn-badge inline lock-badge">{lockTag(lockStamp)}</span>}
                          </strong>
                          <p>
                            {owned
                              ? own.phase === "connecting"
                                ? "画面正在启动…若几秒后未出现，请看手机屏幕是否亮起并确认授权。"
                                : "镜像进行中。直接关闭电脑上的镜像窗口也可以结束。"
                              : stateCopy[device.state].detail}
                          </p>
                        </div>
                        <span className="device-card-actions">
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

              {/* 列表清空后仍要显示反馈，否则移除最后一条记录会静默消失，用户不知道操作是否生效。 */}
              {(recentDevices.length > 0 || recentMessage) && (
                <div className="recent-devices" aria-label="最近使用过的设备">
                  {recentDevices.length > 0 && (
                    <div className="recent-head">
                      <strong>最近使用过的设备</strong>
                      <button className="text-button danger" type="button" onClick={() => void clearRecentDevices()}>
                        清空记录
                      </button>
                    </div>
                  )}
                  {recentDevices.map((device) => {
                    const connected = findConnectedDevice(check?.devices, device.serial);
                    return (
                      <div className="recent-device" key={device.serial}>
                        <div className="recent-device-info">
                          <strong>{device.label}</strong>
                          <p>
                            {relativeTime(device.last_used_at)} · {connected ? stateCopy[connected.state].label : "当前未连接"}
                          </p>
                        </div>
                        <span>
                          {connected ? (
                            // 已连接的设备在上面的设备卡片里有唯一操作入口，
                            // 这里只呈现状态与「移除记录」，不再重复放开始/结束按钮。
                            <span className="recent-hint">{stateCopy[connected.state].label}</span>
                          ) : looksLikeWirelessEndpoint(device.serial) ? (
                            <button className="text-button" type="button" disabled={wirelessBusy} onClick={() => void reconnectRecentDevice(device.serial)}>重新连接</button>
                          ) : (
                            <span className="recent-hint">请用数据线重新连接</span>
                          )}
                          <button className="text-button danger" type="button" onClick={() => void forgetRecentDevice(device.serial)}>移除记录</button>
                        </span>
                      </div>
                    );
                  })}
                  {recentDevices.length > 0 && <p className="recent-note">记录只保存在本机，移除不影响连接与授权。</p>}
                  {recentMessage && <p className="recent-note" role="status">{recentMessage}</p>}
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
                      {recording.active && <p className="screenshot-path">录像正在写入，结束镜像后才会定型；录制中无法删除。</p>}
                    </div>
                  ) : (
                    <p className="capability-pending">在「设置」打开「录制这一会话的画面」后开始镜像；文件保存在「视频 / MirrorDock」，结束镜像即结束录制，可直接播放。</p>
                  )}
                  {recordingError && <p className="capability-pending" role="alert">{recordingError}</p>}
                </div>
                <div className="capability-panel transfer-panel" aria-live="polite">
                  <strong>文件传输</strong>
                  <p className="capability-pending">发送到手机的「下载 / MirrorDock」，或取回该目录的文件到本机。</p>
                  <span>
                    <button className="secondary-button" type="button" disabled={transferBusy} onClick={() => void sendFileTo(readyDevice.serial)}>
                      {transferBusy ? "正在处理…" : "选择文件发送到手机"}
                    </button>
                    <button className="secondary-button" type="button" disabled={transferBusy} onClick={() => void refreshDeviceFiles(readyDevice.serial)}>
                      {transferBusy ? "正在处理…" : deviceFiles ? "刷新手机文件列表" : "查看手机上的文件"}
                    </button>
                  </span>
                  {lastTransfer && (
                    <div className="screenshot-result">
                      <p className="capability-summary">{lastTransfer.file_name} · {formatBytes(lastTransfer.bytes)}</p>
                      <p className="screenshot-path">{lastTransfer.path}</p>
                      <button className="text-button" type="button" onClick={() => void revealTransfer()}>在文件夹中显示</button>
                    </div>
                  )}
                  {deviceFiles !== null && (deviceFiles.length > 0 ? (
                    <ul className="transfer-file-list">
                      {deviceFiles.map((name) => (
                        <li key={name}>
                          <span className="transfer-file-name">{name}</span>
                          <button className="text-button" type="button" disabled={transferBusy} onClick={() => void fetchDeviceFile(readyDevice.serial, name)}>取回到电脑</button>
                          <button className="text-button danger" type="button" disabled={transferBusy} onClick={() => void deleteDeviceFile(readyDevice.serial, name)}>删除</button>
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="capability-pending">手机的「下载 / MirrorDock」文件夹当前没有文件。</p>
                  ))}
                  {transferMessage && <p className="apply-notice" role="status">{transferMessage}</p>}
                  {transferError && <p className="capability-pending" role="alert">{transferError}</p>}
                </div>
                <div className="capability-panel apk-panel" aria-live="polite">
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
              </div>
            ) : (
              <div className="capability-panel">
                <strong>先连接手机</strong>
                <p className="capability-pending">截图、录像与文件传输需要先在「连接」页连接并授权手机。</p>
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
                    <li key={`${item.posted}-${item.pkg}-${index}`}>
                      <span className="transfer-file-name">
                        <strong style={{ marginRight: 6 }}>[{item.app || item.pkg}]</strong>
                        {item.title}
                        {item.text ? `：${item.text}` : ""}
                      </span>
                      <span style={{ marginLeft: "auto", whiteSpace: "nowrap" }}>{notificationTime(item.posted)}</span>
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
                  <button type="button" className="secondary-button" onClick={() => updateOptions(defaultOptions)}>恢复默认设置</button>
                </div>
              </header>
              <div className="settings-rows">
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">画质</span>
                    <span className="setting-desc">分辨率与码率越高越清晰，对电脑与手机性能要求也越高。</span>
                  </div>
                  <select className="setting-control" value={options.quality} onChange={e => updateOptions({...options, quality: e.target.value as SessionOptions["quality"]})}>
                    <option value="smooth">流畅 · 1024 / 2 Mbps</option><option value="balanced">均衡 · 1920 / 8 Mbps</option><option value="sharp">清晰 · 2560 / 16 Mbps</option>
                  </select>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">帧率上限</span>
                    <span className="setting-desc">默认跟随设备帧率。老手机发烫卡顿时选 30，长会话省电选 24。</span>
                  </div>
                  <select className="setting-control" aria-label="帧率上限" value={options.max_fps ?? 0} onChange={e => updateOptions({...options, max_fps: Number(e.target.value) === 0 ? null : Number(e.target.value)})}>
                    <option value={0}>跟随设备（不限）</option>
                    <option value={60}>最高 60 帧</option>
                    <option value={30}>最高 30 帧</option>
                    <option value={24}>最高 24 帧（最省电）</option>
                  </select>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">桌面模式（独立虚拟屏幕）</span>
                    <span className="setting-desc">不再镜像手机现有屏幕，而是在手机上创建一块独立虚拟屏幕：电脑上全屏看视频、写笔记，手机上回微信也不打断画面。需要 Android 10+，手机端会弹出「显示在其他应用上层」的确认。与摄像头画面互斥，更改后重启会话生效。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="桌面模式（独立虚拟屏幕）" checked={options.desktop_mode} onChange={e => { updateOptions({ ...options, desktop_mode: e.target.checked, camera_source: e.target.checked ? false : options.camera_source }); if (e.target.checked && readySerial && deviceApps === null) void loadDeviceApps(readySerial); }} /></label>
                </div>
                {options.desktop_mode && (
                  <div className="setting-row">
                    <div className="setting-info">
                      <span className="setting-name">虚拟屏启动的应用（可选）</span>
                      <span className="setting-desc">实测部分机型（如小米/MIUI）的桌面不在虚拟屏上显示（会得到白屏/黑屏）；填入应用包名后，虚拟屏会直接打开该应用，例如系统浏览器 com.android.browser。留空则显示系统桌面。</span>
                    </div>
                    <input
                      className="setting-control"
                      aria-label="虚拟屏启动的应用包名"
                      list="device-app-packages"
                      placeholder="com.android.browser"
                      value={options.desktop_app ?? ""}
                      onChange={e => { const pkg = e.target.value.trim(); updateOptions({ ...options, desktop_app: pkg ? pkg : null }); }}
                    />
                  </div>
                )}
                {options.desktop_mode && deviceApps !== null && (
                  <datalist id="device-app-packages">
                    {deviceApps.slice(0, 500).map(pkg => <option key={pkg} value={pkg} />)}
                  </datalist>
                )}
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
                  <select className="setting-control" aria-label="显示方向" value={options.rotation} onChange={e => updateOptions({...options, rotation: Number(e.target.value)})}>
                    <option value={0}>自动（跟随手机）</option>
                    <option value={90}>锁定 90°</option>
                    <option value={180}>锁定 180°</option>
                    <option value={270}>锁定 270°</option>
                  </select>
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
                    <span className="setting-desc">镜像进行中手机不会自动熄屏，也不会变暗，锁屏页有充足时间输入解锁密码。USB 与无线均生效：无线连接时会临时把手机置为「充电时保持唤醒」（状态栏可能显示充电中），屏幕即将变暗时也会自动恢复亮度；会话结束即恢复原设置。</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="会话期间保持手机唤醒" checked={options.keep_awake} onChange={e => updateOptions({...options, keep_awake: e.target.checked})} /></label>
                </div>
                <div className="setting-row">
                  <div className="setting-info">
                    <span className="setting-name">录制这一会话的画面</span>
                    <span className="setting-desc">MP4 保存在本机视频目录。{!proEdition && "专业版功能，在下方「版本与授权」激活后可用"}</span>
                  </div>
                  <label className="setting-toggle"><input type="checkbox" aria-label="录制这一会话的画面" checked={options.record} disabled={!proEdition} onChange={e => updateOptions({...options, record: e.target.checked})} /></label>
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
                  <select className="setting-control" value={options.shortcut_mod ?? ""} onChange={e => updateOptions({...options, shortcut_mod: e.target.value || null})}>
                    <option value="">默认（左 Alt / 左 Super）</option>
                    <option value="lctrl">左 Ctrl</option>
                    <option value="rctrl">右 Ctrl</option>
                    <option value="lalt">左 Alt</option>
                    <option value="ralt">右 Alt</option>
                    <option value="lsuper">左 Super（Win / ⌘）</option>
                    <option value="rsuper">右 Super</option>
                  </select>
                </div>
                <p className="setting-note">窗口内建快捷键：+H 主屏幕、+B 返回、+S 最近任务、+N 通知栏、+P 电源、+O 熄屏（镜像继续）、+↑/↓ 音量、+F 全屏、+Q 退出。</p>
                <p className="setting-note">受保护内容（支付、密码页）系统会屏蔽为黑屏；会话进行中的全局快捷键（无需切回本窗口）可在下方「全局快捷键」中自定义。</p>
                {sessionActive && <p className="setting-note">镜像窗口形态在启动时确定，运行中修改需重启窗口，画面会短暂中断。</p>}
                {applyNotice && <p className="setting-note apply-notice" role="status">{applyNotice}</p>}
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
