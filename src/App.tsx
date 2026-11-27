import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactElement } from "react";
import { AppSelect } from "./AppSelect";
import {
  ASSISTANT_GROUPS,
  ASSISTANT_VIEWS,
  AssistantViewPage,
  type AssistantSection,
  type AssistantViewKey,
} from "./assistantViews";
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
import {
  UI_THEMES,
  applyUiTheme,
  readUiTheme,
  themeMeta,
  writeUiTheme,
  type UiThemeKey,
} from "./uiTheme";
// X10-130 按键映射 / 操作回放的纯逻辑层：类型、校验、描述与本机持久化。
import {
  INJECTABLE_KEYCODES,
  clampWaitMs,
  describeAction,
  describeStep,
  isValidCoord,
  isValidDuration,
  isValidKeycode,
  keymapsToJson,
  newLocalId,
  normalizePhysicalKey,
  parseImportedKeymaps,
  parseImportedReplays,
  physicalKeyLabel,
  readKeymaps,
  readRecordPlans,
  readReplays,
  replaysToJson,
  validateReplayScript,
  writeKeymaps,
  writeRecordPlans,
  writeReplays,
  dateKey,
  isRecordPlanDue,
  type KeymapAction,
  type KeymapProfile,
  type RecordPlan,
  type ReplayScript,
  type ReplayStep,
} from "./inputAutomation";
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

// X10-111（C）：首选通道徽标——本次点「开始镜像」实际会走的通道。
// 后端 dedup 后 device.serial 即首选通道（USB 优先），与后端 is_wireless_endpoint 同口径判定。
export function preferredChannelLabel(device: Device): string {
  return looksLikeWirelessEndpoint(device.serial) ? "首选 无线" : "首选 USB";
}
// X10-111（B）：是否提示「可改用有线」——当前首选是无线、但同组存在 ready 的 USB 端点。
// 返回该 USB 的 serial（供「改用有线」按钮直接以它启动）；不该提示时返回 null。
// 判据全基于 ready 状态：offline 的 USB 不作数（杜绝「USB 休眠被误当成可切」的误判）。
export function suggestedUsbSerial(device: Device): string | null {
  if (!looksLikeWirelessEndpoint(device.serial)) return null; // 首选已是 USB，无需提示
  const usb = device.connections.find(
    (connection) => connection.kind === "usb" && connection.state === "ready"
  );
  return usb ? usb.serial : null;
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

/** P0-1：从通知正文保守提取一次性验证码。
 * 只在出现明确的验证码语境（中文「验证码/校验码/动态码/一次性密码」或英文
 * verification code / OTP）且其后 12 个字符内紧跟 4~8 位数字时才识别；
 * 金额、手机号、订单号、普通数字串不会被误判为验证码。 */
export function verificationCode(text: string): string | null {
  if (!text) return null;
  const match = /(?:验证码|校验码|动态码|一次性密码|verification\s*code|one[-\s]?time\s*(?:code|password)|\botp\b)[^\d\n]{0,12}(\d{4,8})(?!\d)/i.exec(text);
  return match ? match[1] : null;
}

/** P0-1：复制验证码到系统剪贴板。优先 Web API，失败走 Rust 系统工具兜底。
 * 返回成功与否；调用方提示语不得包含验证码明文。 */
async function copyCodeToClipboard(code: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(code);
      return true;
    }
  } catch {
    // WebView 可能禁用 Web API，继续走 Rust 兜底。
  }
  try {
    return await invoke<boolean>("copy_text_to_clipboard", { text: code });
  } catch {
    return false;
  }
}

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

// X10-129 批量安装队列里的一项；status 反映 adb 的真实结论，不做乐观假状态。
type BatchApkItem = {
  name: string;
  path: string;
  status: "pending" | "installing" | "done" | "failed";
  message?: string;
};

// X10-129 相册可浏览目录（与后端 GALLERY_ROOTS 对齐的白名单子集，无自由路径）。
const GALLERY_DIRS: { dir: string; label: string }[] = [
  { dir: "/sdcard/DCIM", label: "DCIM" },
  { dir: "/sdcard/DCIM/Camera", label: "相机" },
  { dir: "/sdcard/Pictures", label: "图片" },
  { dir: "/sdcard/Pictures/Screenshots", label: "截图" },
];

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

/** X10-136 电池与存储摘要：字段可缺失（后端读不到就返回 null，不猜默认值）。 */
export type DevicePowerStorage = {
  battery_level: number | null;
  charging: boolean | null;
  temperature: number | null;
  storage_total: string | null;
  storage_free: string | null;
};
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

/** 多设备批量截图的文件名：同一秒内多台设备不会互相覆盖，序列号拼进文件名。 */
export function screenshotFileNameForDevice(now: Date, serial: string) {
  // 序列号可能是无线地址（含冒号、点），只保留文件名白名单字符，其余换成短横线。
  const tag = serial.replace(/[^A-Za-z0-9._-]/g, "-").replace(/\.\.+/g, ".");
  return `MirrorDock-${localTimestamp(now)}-${tag}.png`;
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

// X10-150：设备中心「常用工具」六宫格——全部是既存真实视图的跳转入口，没有假能力。
const HOME_TOOLS: { view: AssistantViewKey; title: string; hint: string; icon: string; tone: string }[] = [
  { view: "mirror", title: "镜像控制", hint: "画面 · 键鼠直输", icon: "M8 5l11 7-11 7z", tone: "c1" },
  { view: "files", title: "文件传输", hint: "双向传输 · 本机目录", icon: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z", tone: "c2" },
  { view: "apps", title: "应用管理", hint: "列表 / 卸载 / 导出", icon: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z", tone: "c3" },
  { view: "notify", title: "通知中心", hint: "实时镜像 · 按应用筛选", icon: "M6 9a6 6 0 1 1 12 0c0 5 2 5 2 6H4c0-1 2-1 2-6zM10 19h4", tone: "c4" },
  { view: "shot", title: "截图", hint: "保存到本机", icon: "M4 7h3l1.5-2h7L17 7h3v12H4zM12 17a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z", tone: "c5" },
  { view: "info", title: "设备信息", hint: "只读诊断摘要", icon: "M12 11v5M12 8h.01M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z", tone: "c6" },
];

// X10-150：设备中心「最近任务」——本应用运行期间真实执行过的操作记录，
// 只保留最近 8 条、不持久化。没有发生过的操作绝不会出现在这里。
type RecentTask = { id: number; title: string; status: "done" | "failed" | "active"; time: string };

// 左侧导航（参照 ToDesk / UU 远程的分栏布局）。每个面板都常驻渲染、由 CSS
// 控制显隐：切换导航不销毁任何状态（输入框内容、上传结果都不丢）。
type TabKey = "home" | "tools" | "wireless" | "settings" | "help";
const LEGACY_VIEWS = new Set<AssistantViewKey>(["home", "connect", "settings", "help"]);

// X10-144：设置中心二级分组导航。每一项对应一个既存设置卡；卡片 DOM 与事件
// 绑定保持原样，只由 CSS 控制显隐——切换分组不丢输入内容、不卸载任何控件。
// 铁律：新版本必须保留设置中心的全部真实设置项，不允许用演示开关替换。
type SettingsGroupKey = "appearance" | "general" | "mirror" | "desktop" | "shortcuts" | "license" | "about";
const SETTINGS_GROUPS: { key: SettingsGroupKey; label: string; note: string }[] = [
  { key: "appearance", label: "外观主题", note: "5 套本机主题" },
  { key: "general", label: "通用", note: "启动 / 重连 / 停靠" },
  { key: "mirror", label: "镜像窗口", note: "画质 / 帧率 / 输入 / 录制" },
  { key: "desktop", label: "按设备 · 桌面模式", note: "每设备独立配置" },
  { key: "shortcuts", label: "全局快捷键", note: "截图 / 录制 / 旋转" },
  { key: "license", label: "版本与授权", note: "专业版离线激活" },
  { key: "about", label: "关于 MirrorDock", note: "版本 / 许可 / 更新" },
];

// 顶栏主题菜单的小色点：取各主题的签名色（重设计 v2.5）。
const THEME_DOT: Record<UiThemeKey, string> = {
  a: "#4a7cf3",
  b: "#101216",
  c: "#7a5cf0",
  d: "#ffffff",
  e: "#8fb4ff",
};

// 命令面板里可执行的页头过滤视图：这些页面才有可筛选的表格行。
const FILTERABLE_VIEWS = new Set<AssistantViewKey>([
  // X10-146：应用管理的筛选框改为预览同款「卡头搜索」，与表格同属一张卡。
  "multi", "mirror", "keymap", "replay", "shot", "album", "notify", "help",
]);

// Context Panel（右栏）只在设备相关的视图渲染（重设计 §4：右栏从全量降级为情境面板）。
// X10-153：右栏按重构预览 1:1——情境右栏仅设备相关视图显示（connect/info/multi/mirror/present），
// 其余视图隐藏（重构约束 #2：避免空栏占位 280px）。与预览 CTX_VIEWS 完全一致。
const RAIL_VIEWS = new Set<AssistantViewKey>(["connect", "info", "multi", "mirror", "present"]);

function tabForAssistantView(view: AssistantViewKey): TabKey {
  if (view === "home") return "home";
  if (view === "connect") return "wireless";
  if (view === "settings") return "settings";
  if (view === "help") return "help";
  return "tools";
}

const assistantViewIcons: Record<AssistantViewKey, ReactElement> = {
  home: <NavIcon d="M4 20h16M6 20V9.5L12 4l6 5.5V20M10 20v-5h4v5" />,
  connect: <NavIcon d="M3.5 5.5A2 2 0 0 1 5.5 3.5h13a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2v-9Zm5 13h7" />,
  info: <NavIcon d="M12 11v5M12 8h.01M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z" />,
  multi: <NavIcon d="M8 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm8 0a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM3 20v-2a5 5 0 0 1 10 0v2Zm10 0v-2a5 5 0 0 1 8-4" />,
  mirror: <NavIcon d="M8 5l11 7-11 7z" />,
  present: <NavIcon d="M4 5h16v11H4zM9 20h6M12 16v4" />,
  keymap: <NavIcon d="M4 7h16v10H4zM7 10h.01M11 10h.01M15 10h.01M7 14h10" />,
  schedule: <NavIcon d="M12 7v5l3 2M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z" />,
  replay: <NavIcon d="M4 12a8 8 0 1 0 2.3-5.7M4 4v4h4M12 8v4l3 2" />,
  files: <NavIcon d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />,
  shot: <NavIcon d="M4 7h3l1.5-2h7L17 7h3v12H4zM12 17a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z" />,
  record: <NavIcon d="M12 19a7 7 0 1 0 0-14 7 7 0 0 0 0 14Z" />,
  album: <NavIcon d="M3 5h18v14H3zM7 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3ZM4 18l5-5 4 4 3-3 4 4" />,
  install: <NavIcon d="M12 3v11M8 10l4 4 4-4M5 20h14" />,
  apps: <NavIcon d="M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z" />,
  notify: <NavIcon d="M6 9a6 6 0 1 1 12 0c0 5 2 5 2 6H4c0-1 2-1 2-6zM10 19h4" />,
  settings: <NavIcon d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm7-3a7 7 0 0 0-.1-1.2l2-1.5-2-3.5-2.4 1a7 7 0 0 0-2-1.2L14 3h-4l-.5 2.6a7 7 0 0 0-2 1.2l-2.4-1-2 3.5 2 1.5A7 7 0 0 0 5 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.5 2.4-1a7 7 0 0 0 2 1.2L10 21h4l.5-2.6a7 7 0 0 0 2-1.2l2.4 1 2-3.5-2-1.5c.07-.4.1-.8.1-1.2Z" />,
  help: <NavIcon d="M12 17h.01M9.1 9a3 3 0 0 1 5.8 1c0 2-3 2.5-3 4M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z" />,
  pro: <NavIcon d="M12 3l8 3v6c0 5-3.4 8-8 9-4.6-1-8-4-8-9V6zM9 12l2 2 4-4" />,
};

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
  // X10-128：爱思风格原型对齐后，真实导航以 19 个功能视图为准；
  // tab 只保留给仍使用原有完整面板的「连接 / 设置 / 帮助」等页面。
  const [view, setView] = useState<AssistantViewKey>("home");
  const [viewFilter, setViewFilter] = useState("");
  // X10-146：应用管理按预览在卡头提供名称 / 包名排序；只影响展示顺序。
  const [appsSort, setAppsSort] = useState<"name" | "package">("name");
  // X10-153：侧栏按重构预览 1:1 复刻——静态分组标题 + 常显导航项（无折叠、无设备树）。
  // 分组数据与视图键见 assistantViews.tsx ASSISTANT_GROUPS（与预览 NAV 完全一致）。
  const [uiTheme, setUiTheme] = useState<UiThemeKey>(() => readUiTheme());
  // 重设计 v2.5：⌘K 命令面板、顶栏主题菜单、⌘B 侧栏折叠。
  const [cmdkOpen, setCmdkOpen] = useState(false);
  const [cmdkQuery, setCmdkQuery] = useState("");
  const [cmdkIndex, setCmdkIndex] = useState(0);
  const [themeMenuOpen, setThemeMenuOpen] = useState(false);
  const [railCollapsed, setRailCollapsed] = useState(false);
  // X10-144：设置中心二级分组（默认外观，与原设置页首屏一致）。
  const [settingsGroup, setSettingsGroup] = useState<SettingsGroupKey>("appearance");

  function openView(next: AssistantViewKey) {
    setView(next);
    setViewFilter("");
    setTab(tabForAssistantView(next));
  }

  // X10-127：主题只影响本机视觉，切换后立即写入 DOM 与 localStorage。
  useEffect(() => {
    applyUiTheme(uiTheme);
  }, [uiTheme]);

  // ⌘K / Ctrl+K 打开命令面板；⌘B / Ctrl+B 折叠侧栏（重设计 v2.5）。
  useEffect(() => {
    const onShellHotkey = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      if ((event.metaKey || event.ctrlKey) && key === "k") {
        event.preventDefault();
        setCmdkOpen((open) => !open);
        setCmdkQuery("");
        setCmdkIndex(0);
      } else if ((event.metaKey || event.ctrlKey) && key === "b") {
        event.preventDefault();
        setRailCollapsed((collapsed) => !collapsed);
      }
    };
    window.addEventListener("keydown", onShellHotkey);
    return () => window.removeEventListener("keydown", onShellHotkey);
  }, []);

  const chooseUiTheme = (next: UiThemeKey) => {
    setUiTheme(next);
    writeUiTheme(next);
  };
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
  // X10-139 多端一键截图的结论（逐台成功/失败），只在多端视图展示。
  const [multiCaptureSummary, setMultiCaptureSummary] = useState<string | null>(null);
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
  // X10-129 批量安装：多选 APK 逐个安装；每个结论都来自 adb 的真实回执。
  const [batchApks, setBatchApks] = useState<BatchApkItem[]>([]);
  const [batchBusy, setBatchBusy] = useState(false);
  const batchCancelRef = useRef(false);
  // X10-129 相册：固定白名单目录浏览；列表与取回失败都如实展示，不伪装空态。
  const [galleryDir, setGalleryDir] = useState<string>(GALLERY_DIRS[0].dir);
  const [galleryFiles, setGalleryFiles] = useState<string[] | null>(null);
  const [galleryBusy, setGalleryBusy] = useState(false);
  const [galleryError, setGalleryError] = useState<string | null>(null);
  const [galleryMessage, setGalleryMessage] = useState<string | null>(null);
  const [galleryFetched, setGalleryFetched] = useState<Record<string, TransferReceipt>>({});
  const [galleryErrors, setGalleryErrors] = useState<Record<string, string>>({});
  const [fetchingGalleryName, setFetchingGalleryName] = useState<string | null>(null);
  // X10-130 按键映射：方案与绑定只保存在本机 localStorage。
  const [keymaps, setKeymaps] = useState<{ profiles: KeymapProfile[]; activeId: string | null }>(() => readKeymaps());
  const [capturingKey, setCapturingKey] = useState(false);
  const [capturedKey, setCapturedKey] = useState<{ physicalKey: string; label: string } | null>(null);
  const [newActionKind, setNewActionKind] = useState<"key" | "tap" | "swipe">("key");
  const [newKeycode, setNewKeycode] = useState<string>(INJECTABLE_KEYCODES[0].value);
  const [draftTap, setDraftTap] = useState({ x: "540", y: "1200" });
  const [draftSwipe, setDraftSwipe] = useState({ x1: "540", y1: "1600", x2: "540", y2: "400", duration: "300" });
  const [keymapNotice, setKeymapNotice] = useState<string | null>(null);
  const [keymapError, setKeymapError] = useState<string | null>(null);
  const [lastInjected, setLastInjected] = useState<string | null>(null);
  // X10-130 操作回放：脚本与本机映射一样只存 localStorage；执行状态逐步展示。
  const [replays, setReplays] = useState<ReplayScript[]>(() => readReplays());
  const [activeReplayId, setActiveReplayId] = useState<string | null>(null);
  const [draftStepKind, setDraftStepKind] = useState<"tap" | "swipe" | "key">("tap");
  const [draftStepWait, setDraftStepWait] = useState("500");
  const [replayStatus, setReplayStatus] = useState<string[] | null>(null);
  const [replayRunning, setReplayRunning] = useState(false);
  const [replayNotice, setReplayNotice] = useState<string | null>(null);
  const [replayError, setReplayError] = useState<string | null>(null);
  const replayCancelRef = useRef(false);
  // X10-131 定时录制计划：计划存本机 localStorage；运行日志只在内存展示。
  // 触发前提与应用运行一致——不做后台拉起，不绕过设备授权；到点时设备需
  // 已有显示会话（录制依赖显示会话），失败会如实写进运行日志。
  const [recordPlans, setRecordPlans] = useState<RecordPlan[]>(() => readRecordPlans());
  const [planRunLog, setPlanRunLog] = useState<{ at: number; planName: string; text: string; error: boolean }[]>([]);
  const [draftPlan, setDraftPlan] = useState({ name: "", time: "09:30", duration: "15" });
  const planLastRunRef = useRef<Record<string, string>>({});
  const planStopRef = useRef<{ at: number; planId: string } | null>(null);
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
  /** P0-1/P0-2：不含验证码明文的提示语（明文一律不进 state 之外的任何地方）。 */
  const [notifyNotice, setNotifyNotice] = useState<string | null>(null);
  /** P0-3：手机发来的剪贴板文字，用户点「复制」才写入系统剪贴板。 */
  const [incomingClipboard, setIncomingClipboard] = useState<string | null>(null);
  const [clipboardNotice, setClipboardNotice] = useState<string | null>(null);
  // X10-133 本机保留：默认关闭（通知仍只在内存）；开启后通知列表写本机
  // localStorage，重启应用后恢复。关闭即清除已存记录，不上传。
  const [notifyKeep, setNotifyKeep] = useState<boolean>(() => {
    try {
      return localStorage.getItem("mirrordock.notifyKeep") === "1";
    } catch {
      return false;
    }
  });
  // X10-69 快捷回复：每条通知的草稿与发送状态（按通知 key 索引，仅内存）。
  const [notifReplyDrafts, setNotifReplyDrafts] = useState<Record<string, string>>({});
  const [notifReplyBusy, setNotifReplyBusy] = useState<string | null>(null);
  const [notifReplyNotice, setNotifReplyNotice] = useState<{ key: string; text: string; error: boolean } | null>(null);

  // X10-133：切换「本机保留通知记录」。开启时把当前通知快照写入本机，
  // 关闭时清除已存记录；通知内容不出本机。
  function toggleNotifyKeep() {
    const next = !notifyKeep;
    setNotifyKeep(next);
    try {
      localStorage.setItem("mirrordock.notifyKeep", next ? "1" : "0");
      if (next) localStorage.setItem("mirrordock.notifyLog", JSON.stringify(phoneNotifications));
      else localStorage.removeItem("mirrordock.notifyLog");
    } catch {
      /* 存储不可用时仅本次会话生效 */
    }
  }

  // 通知列表变化时同步写入本机（仅开启保留时）。
  useEffect(() => {
    if (!notifyKeep) return;
    try {
      localStorage.setItem("mirrordock.notifyLog", JSON.stringify(phoneNotifications));
    } catch {
      /* 存储不可用时仅本次会话生效 */
    }
  }, [notifyKeep, phoneNotifications]);

  // 启动时恢复上次保留的通知（仅开启保留时）。
  useEffect(() => {
    if (!notifyKeep) return;
    try {
      const raw: unknown = JSON.parse(localStorage.getItem("mirrordock.notifyLog") ?? "[]");
      if (Array.isArray(raw)) {
        const restored = raw.filter(
          (item): item is PhoneNotification =>
            typeof item === "object" && item !== null && typeof (item as PhoneNotification).pkg === "string",
        );
        if (restored.length > 0) setPhoneNotifications(restored.slice(0, 50));
      }
    } catch {
      /* 坏数据直接忽略，按无历史处理 */
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
  // P0-1：复制验证码。提示语不含明文；复制失败明确报错，不伪装成功。
  async function copyNotificationCode(code: string) {
    setNotifyNotice(null);
    const ok = await copyCodeToClipboard(code);
    setNotifyNotice(ok ? "验证码已复制到本机剪贴板。" : "复制失败：浏览器与系统剪贴板均不可用，请手动选中复制。");
  }

  // P0-2：清除通知同步到手机。手机不在线时保留本地列表并说明原因，
  // 绝不先清本地再假装"已同步"——避免用户误以为手机也清了。
  async function clearNotifications(key?: string) {
    setNotifyNotice(null);
    try {
      const sent = await invoke<boolean>("companion_notification_clear", key ? { key } : {});
      if (!sent) {
        setNotifyNotice("手机伴侣常驻连接不在线，未清除通知；请确认手机在线后重试。");
        return;
      }
      setPhoneNotifications((prev) => (key ? prev.filter((n) => n.key !== key) : []));
      setNotifyNotice(key ? "已清除该通知，手机同步移除。" : "已清空通知，手机同步移除。");
    } catch (error) {
      setNotifyNotice(errorMessage(error, "清除失败，请稍后再试。"));
    }
  }

  // P0-3 电脑→手机：用户点按钮才读本机系统剪贴板并下发，不自动监听。
  async function sendClipboardToPhone() {
    setClipboardNotice(null);
    try {
      const result = await invoke<boolean | null>("companion_clipboard_send");
      if (result === null) setClipboardNotice("本机剪贴板没有可发送的文字。");
      else if (result === false) setClipboardNotice("手机常驻连接不在线，未发送；请确认手机在线后重试。");
      else setClipboardNotice("已发送剪贴板文字到手机。");
    } catch (error) {
      setClipboardNotice(errorMessage(error, "发送失败，请稍后再试。"));
    }
  }

  // P0-3 手机→电脑：手机发来的文字只在内存；用户点「复制」才写入系统剪贴板。
  async function acceptIncomingClipboard() {
    if (!incomingClipboard) return;
    const ok = await copyCodeToClipboard(incomingClipboard);
    setClipboardNotice(ok ? "已写入本机剪贴板。" : "写入剪贴板失败，请手动选中文字复制。");
    if (ok) setIncomingClipboard(null);
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
  // 帮助中心折叠（X10-125）：默认全部只显示标题，点标题独立展开/收起。
  const [expandedHelpIds, setExpandedHelpIds] = useState<ReadonlySet<string>>(new Set());
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

  // X10-134 应用动作：启动 / 卸载。启动用 monkey 触发默认入口；卸载先弹显式
  // 确认（不可恢复），成功后清掉应用清单缓存，下次打开视图重新读真实状态。
  const [appActionBusy, setAppActionBusy] = useState<string | null>(null);
  const [appActionNotice, setAppActionNotice] = useState<{ text: string; error: boolean } | null>(null);

  async function launchDeviceApp(packageName: string) {
    if (!readySerial || appActionBusy) return;
    setAppActionBusy(packageName);
    setAppActionNotice(null);
    try {
      await invoke("launch_device_app", { serial: readySerial, package: packageName });
      setAppActionNotice({ text: `已在手机上打开 ${packageName}。`, error: false });
    } catch (error) {
      setAppActionNotice({ text: errorMessage(error, "应用没有在手机上打开。"), error: true });
    } finally {
      setAppActionBusy(null);
    }
  }

  async function uninstallDeviceApp(packageName: string, appName: string) {
    if (!readySerial || appActionBusy) return;
    if (!window.confirm(`确定在手机上卸载「${appName || packageName}」（${packageName}）吗？卸载后无法恢复；系统预装应用无法卸载。`)) return;
    setAppActionBusy(packageName);
    setAppActionNotice(null);
    try {
      await invoke("uninstall_device_app", { serial: readySerial, package: packageName, keepData: false });
      setAppActionNotice({ text: `已卸载 ${appName || packageName}。`, error: false });
      setDeviceApps((prev) => {
        if (!readySerial) return prev;
        const next = { ...prev };
        delete next[readySerial];
        return next;
      });
    } catch (error) {
      setAppActionNotice({ text: errorMessage(error, "卸载没有完成。"), error: true });
    } finally {
      setAppActionBusy(null);
    }
  }

  // X10-136 电池与存储摘要：进入设备信息视图自动读取一次，可手动刷新；
  // 结果按设备标记，换设备后不显示旧数据。只读，不落盘。
  const [powerStorage, setPowerStorage] = useState<DevicePowerStorage | null>(null);
  const [powerSerial, setPowerSerial] = useState<string | null>(null);
  const [powerBusy, setPowerBusy] = useState(false);
  const [powerError, setPowerError] = useState<string | null>(null);
  // X10-150：设备中心「最近任务」。只记录真实完成的操作（截图/安装/录制），
  // 会话内存态、最多 8 条、不持久化——与预览模板的演示数据严格区分。
  const [recentTasks, setRecentTasks] = useState<RecentTask[]>([]);
  const recentTaskSeq = useRef(0);
  function pushRecentTask(title: string, status: RecentTask["status"]) {
    recentTaskSeq.current += 1;
    const entry: RecentTask = {
      id: recentTaskSeq.current,
      title,
      status,
      time: new Date().toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false }),
    };
    setRecentTasks((prev) => [entry, ...prev].slice(0, 8));
  }

  async function loadPowerStorage(serial: string) {
    setPowerBusy(true);
    setPowerError(null);
    try {
      setPowerStorage(await invoke<DevicePowerStorage>("device_battery_storage", { serial }));
      setPowerSerial(serial);
    } catch (error) {
      setPowerStorage(null);
      setPowerSerial(serial);
      setPowerError(errorMessage(error, "无法读取电池与存储信息。"));
    } finally {
      setPowerBusy(false);
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
      if (saved) pushRecentTask(`${saved.file_name} 已保存`, "done");
    } catch (error) {
      setScreenshot(null);
      setScreenshotError(errorMessage(error, "无法保存截图。"));
      pushRecentTask(`截图失败：${errorMessage(error, "无法保存截图。")}`, "failed");
    } finally {
      setScreenshotBusy(false);
    }
  }

  // X10-139 一键全部截图：对所有就绪设备各截一张，文件名带序列号避免同秒覆盖；
  // 单台失败不阻断其他设备，结论逐台展示。
  async function captureAllScreens() {
    const targets = (check?.devices ?? []).filter((device) => device.state === "ready");
    if (targets.length === 0) {
      setMultiCaptureSummary("没有就绪的手机可截图；先在「连接」页连上并授权设备。");
      return;
    }
    setScreenshotBusy(true);
    setMultiCaptureSummary(null);
    let done = 0;
    const failures: string[] = [];
    for (const device of targets) {
      const fileName = screenshotFileNameForDevice(new Date(), device.serial);
      try {
        await invoke<Screenshot>("capture_screenshot", { serial: device.serial, fileName });
        done += 1;
      } catch (error) {
        failures.push(`${displayLabels[device.serial] ?? device.label}：${errorMessage(error, "截图失败")}`);
      }
    }
    setScreenshotBusy(false);
    setMultiCaptureSummary(
      `全部截图结束：${done}/${targets.length} 台成功，保存在「图片 / MirrorDock」。` +
        (failures.length > 0 ? `失败：${failures.join("；")}` : ""),
    );
    pushRecentTask(`全部截图：${done}/${targets.length} 台成功`, failures.length > 0 ? "failed" : "done");
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
        if (stopped) { setRecording(stopped); pushRecentTask(`${stopped.file_name} 已保存`, "done"); }
        else setRecording(null);
      } else {
        const stamp = new Date().toISOString().replace(/[-:T]/g, "").slice(0, 14);
        const started = await invoke<Recording>("start_recording", {
          fileName: `mirrordock-record-${stamp}.mp4`,
        });
        setRecording(started);
        if (started) pushRecentTask(`${started.file_name} 录制中`, "active");
      }
    } catch (error) {
      setRecordingError(errorMessage(error, recording?.active ? "无法结束录制。" : "无法开始录制。"));
      // X10-99：结束录制报错（如后端核验「没产出文件」）时，后端的录制状态其实已清，
      // 前端不能还把按钮停在「录制中」——把本地状态一并复位，托盘/按钮回到「开始录制」。
      if (recording?.active) setRecording(null);
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

  // ---- X10-131 定时录制计划 ---------------------------------------------
  // 计划只在本应用运行时触发：到点自动 start_recording，时长到自动 stop。
  // 录制依赖显示会话，若到点时设备没有会话，adb 会失败并如实写进运行日志。

  function pushPlanLog(planName: string, text: string, error: boolean) {
    setPlanRunLog((prev) => [{ at: Date.now(), planName, text, error }, ...prev].slice(0, 20));
  }

  function addRecordPlan() {
    const name = draftPlan.name.trim() || `计划 ${recordPlans.length + 1}`;
    const duration = Number(draftPlan.duration);
    if (!/^\d{2}:\d{2}$/.test(draftPlan.time) || !Number.isInteger(duration) || duration < 1 || duration > 240) {
      setRecordingError("计划时长需为 1–240 分钟的整数，时间需为 HH:MM。");
      return;
    }
    const plan: RecordPlan = { id: newLocalId(), name, time: draftPlan.time, durationMin: duration, enabled: true };
    const next = [...recordPlans, plan];
    setRecordPlans(next);
    writeRecordPlans(next);
    setRecordingError(null);
  }

  function toggleRecordPlan(id: string) {
    const next = recordPlans.map((plan) => (plan.id === id ? { ...plan, enabled: !plan.enabled } : plan));
    setRecordPlans(next);
    writeRecordPlans(next);
  }

  function removeRecordPlan(id: string) {
    const next = recordPlans.filter((plan) => plan.id !== id);
    setRecordPlans(next);
    writeRecordPlans(next);
    if (planStopRef.current?.planId === id) planStopRef.current = null;
    delete planLastRunRef.current[id];
  }

  async function runPlanRecording(plan: RecordPlan) {
    if (!proEdition) {
      pushPlanLog(plan.name, "未执行：录制是专业版功能，激活后计划才会开始。", true);
      return;
    }
    const stamp = new Date().toISOString().replace(/[-:T]/g, "").slice(0, 14);
    try {
      const started = await invoke<Recording>("start_recording", { fileName: `mirrordock-record-${stamp}.mp4` });
      setRecording(started);
      planStopRef.current = { at: Date.now() + plan.durationMin * 60000, planId: plan.id };
      pushPlanLog(plan.name, `已按计划开始录制，${plan.durationMin} 分钟后自动结束。`, false);
    } catch (error) {
      pushPlanLog(plan.name, errorMessage(error, "计划录制未能开始（到点时设备需要有运行中的镜像会话）。"), true);
    }
  }

  async function stopPlanRecording(planId: string) {
    const plan = recordPlans.find((item) => item.id === planId);
    const planName = plan?.name ?? "定时计划";
    try {
      const stopped = await invoke<Recording | null>("stop_recording", {});
      setRecording(stopped);
      pushPlanLog(planName, "计划时长已到，录制已结束。", false);
    } catch (error) {
      // 结束失败时后端状态已清（与手动结束同一语义），本地同步复位。
      setRecording(null);
      pushPlanLog(planName, errorMessage(error, "计划结束录制时出现问题。"), true);
    }
  }

  // 调度循环：每 20 秒检查一次到点计划与到期停止。ref 存放「当天已触发」与
  // 「计划停止时刻」，避免依赖抖动导致重复触发。
  useEffect(() => {
    const timer = setInterval(() => {
      const now = new Date();
      const pending = planStopRef.current;
      if (pending && Date.now() >= pending.at) {
        planStopRef.current = null;
        void stopPlanRecording(pending.planId);
        return;
      }
      if (pending || recording?.active) return;
      for (const plan of recordPlans) {
        const lastRun = planLastRunRef.current[plan.id];
        if (!isRecordPlanDue(plan, now, lastRun)) continue;
        planLastRunRef.current[plan.id] = dateKey(now);
        void runPlanRecording(plan);
        break;
      }
    }, 20000);
    return () => clearInterval(timer);
    // recordPlans / recording 变化时重建循环；其余依赖经 ref 读取。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recordPlans, recording?.active]);

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
      pushRecentTask(`${apkPath.split(/[\\/]/).pop() ?? apkPath} 已安装`, "done");
    } catch (error) {
      setApkError(errorMessage(error, "安装没有完成。"));
      pushRecentTask(`安装失败：${apkPath.split(/[\\/]/).pop() ?? apkPath}`, "failed");
    } finally {
      setApkBusy(false);
    }
  }

  // X10-129 批量安装：一次挑多个 APK，逐个交给后端安装。
  async function pickApksBatch() {
    setApkError(null);
    setApkMessage(null);
    let picked: string | string[] | null;
    try {
      picked = await openFilePicker({
        multiple: true,
        title: "选择要安装到手机的 APK（可多选）",
        filters: [{ name: "Android 安装包", extensions: ["apk"] }],
      });
    } catch (error) {
      setApkError(errorMessage(error, "无法打开文件选择器。"));
      return;
    }
    // 用户取消选择不算错误，界面回到原样即可。
    const paths = Array.isArray(picked) ? picked.filter((path) => path.length > 0) : [];
    if (paths.length === 0) return;
    setBatchApks(paths.map((path) => ({
      name: path.split(/[\\/]/).pop() ?? path,
      path,
      status: "pending" as const,
    })));
  }

  // 批量安装执行：逐个调用后端，单个失败不阻断后续；「停止」在每步之间生效。
  async function installBatch(serial: string) {
    if (batchApks.length === 0) return;
    setBatchBusy(true);
    batchCancelRef.current = false;
    setApkError(null);
    setApkMessage(null);
    let done = 0;
    let failed = 0;
    try {
      for (let index = 0; index < batchApks.length; index += 1) {
        if (batchCancelRef.current) break;
        const item = batchApks[index];
        setBatchApks((prev) => prev.map((entry, i) => (i === index ? { ...entry, status: "installing", message: undefined } : entry)));
        try {
          const receipt = await invoke<ApkInstallReceipt>("install_apk_to_device", { serial, apkPath: item.path });
          setBatchApks((prev) => prev.map((entry, i) => (i === index ? { ...entry, status: "done", message: receipt.summary } : entry)));
          done += 1;
        } catch (error) {
          setBatchApks((prev) => prev.map((entry, i) => (i === index ? { ...entry, status: "failed", message: errorMessage(error, "安装失败。") } : entry)));
          failed += 1;
        }
      }
      const skipped = batchApks.length - done - failed;
      setApkMessage(
        `批量安装结束：${done} 个成功${failed > 0 ? `，${failed} 个失败` : ""}${skipped > 0 ? `，${skipped} 个未执行` : ""}。` +
          "安装过程手机上可能逐个出现确认，需要你本人同意。",
      );
      pushRecentTask(`批量安装：${done} 个成功${failed > 0 ? ` · ${failed} 个失败` : ""}`, failed > 0 ? "failed" : "done");
    } finally {
      setBatchBusy(false);
    }
  }

  // X10-137 拖入多个 APK → 全部加入批量安装队列并切到安装页；不再像以前那样
  // 只装第一个、其余静默丢弃。队列按路径去重追加，不覆盖已挑选的内容。
  function enqueueDroppedApks(apks: string[]) {
    setBatchApks((prev) => {
      const known = new Set(prev.map((item) => item.path));
      const additions = apks
        .filter((path) => !known.has(path))
        .map((path) => ({ name: path.split(/[\\/]/).pop() ?? path, path, status: "pending" as const }));
      return [...prev, ...additions];
    });
    openView("install");
    setDropInstallNotice({
      kind: "info",
      text: `已把 ${apks.length} 个 APK 加入批量安装队列；到「批量安装」页选择手机后点「开始批量安装」。`,
    });
  }

  // -- X10-129 相册 --

  async function refreshGallery(dir: string = galleryDir) {
    if (!transferDeviceSerial) return;
    setGalleryBusy(true);
    setGalleryError(null);
    setGalleryMessage(null);
    try {
      const names = await invoke<string[]>("list_device_gallery", { serial: transferDeviceSerial, dir });
      setGalleryFiles(names);
    } catch (error) {
      // 读取失败置空并如实报错，不伪装成「相册是空的」。
      setGalleryFiles(null);
      setGalleryError(errorMessage(error, "无法读取相册目录。"));
    } finally {
      setGalleryBusy(false);
    }
  }

  function chooseGalleryDir(dir: string) {
    setGalleryDir(dir);
    setGalleryFiles(null);
    setGalleryError(null);
    setGalleryMessage(null);
    setGalleryFetched({});
    setGalleryErrors({});
    void refreshGallery(dir);
  }

  async function fetchGalleryFile(fileName: string): Promise<boolean> {
    if (!transferDeviceSerial) return false;
    setGalleryMessage(null);
    setGalleryError(null);
    setFetchingGalleryName(fileName);
    setGalleryErrors((prev) => {
      if (!(fileName in prev)) return prev;
      const next = { ...prev };
      delete next[fileName];
      return next;
    });
    try {
      const receipt = await invoke<TransferReceipt>("fetch_device_gallery_file", {
        serial: transferDeviceSerial,
        dir: galleryDir,
        fileName,
      });
      setGalleryFetched((prev) => ({ ...prev, [fileName]: receipt }));
      setGalleryMessage(`已取回「${fileName}」，保存在「下载 / MirrorDock / 相册」。`);
      return true;
    } catch (error) {
      setGalleryErrors((prev) => ({ ...prev, [fileName]: errorMessage(error, "没有取回成功。") }));
      return false;
    } finally {
      setFetchingGalleryName(null);
    }
  }

  // X10-138 一键取回全部：按顺序逐个取回未成功过的文件，单个失败不阻断后续；
  // 已取回的文件不重复下载。
  async function fetchAllGalleryFiles() {
    if (!transferDeviceSerial || galleryFiles === null || fetchingGalleryName !== null) return;
    const pending = galleryFiles.filter((name) => !galleryFetched[name]);
    if (pending.length === 0) {
      setGalleryMessage("这个目录里的文件都已取回过，没有需要重复下载的。");
      return;
    }
    let done = 0;
    for (const name of pending) {
      const ok = await fetchGalleryFile(name);
      if (ok) done += 1;
    }
    setGalleryMessage(`一键取回结束：${done}/${pending.length} 个成功，保存在「下载 / MirrorDock / 相册」。`);
  }

  // -- X10-130 按键映射 --

  const activeKeymapProfile = keymaps.activeId
    ? keymaps.profiles.find((profile) => profile.id === keymaps.activeId) ?? null
    : null;

  function persistKeymaps(next: { profiles: KeymapProfile[]; activeId: string | null }) {
    setKeymaps(next);
    writeKeymaps(next.profiles, next.activeId);
  }

  // X10-140 映射方案导出 / 导入：备份成 .json 文件、在机器之间迁移。
  // 导入内容逐条校验，坏条目丢弃；导入成功立即写 localStorage。
  async function exportKeymaps() {
    setKeymapError(null);
    setKeymapNotice(null);
    try {
      const path = await saveFilePicker({
        title: "导出键位映射方案",
        defaultPath: "mirrordock-keymaps.json",
        filters: [{ name: "JSON 文件", extensions: ["json"] }],
      });
      if (!path) return;
      await invoke("write_export_file", { path, contents: keymapsToJson(keymaps) });
      setKeymapNotice("映射方案已导出，可以在其他电脑上用「导入方案」恢复。");
    } catch (error) {
      setKeymapError(errorMessage(error, "导出没有完成。"));
    }
  }

  async function importKeymaps() {
    setKeymapError(null);
    setKeymapNotice(null);
    try {
      const path = await openFilePicker({
        title: "导入键位映射方案",
        multiple: false,
        filters: [{ name: "JSON 文件", extensions: ["json"] }],
      });
      if (typeof path !== "string" || path.length === 0) return;
      const rawText = await invoke<string>("read_import_file", { path });
      const parsed = parseImportedKeymaps(JSON.parse(rawText));
      if (!parsed) {
        setKeymapError("这个文件不是 MirrorDock 导出的映射方案；没有做任何改动。");
        return;
      }
      persistKeymaps(parsed);
      setKeymapNotice(`已导入 ${parsed.profiles.length} 个映射方案；坏条目已自动跳过。`);
    } catch {
      setKeymapError("读取或解析导入文件没有成功；没有做任何改动。");
    }
  }

  function createKeymapProfile() {
    const profile: KeymapProfile = {
      id: newLocalId(),
      name: `方案 ${keymaps.profiles.length + 1}`,
      bindings: [],
    };
    persistKeymaps({ profiles: [...keymaps.profiles, profile], activeId: profile.id });
    setKeymapError(null);
    setKeymapNotice(null);
  }

  function deleteKeymapProfile(id: string) {
    const profiles = keymaps.profiles.filter((profile) => profile.id !== id);
    persistKeymaps({ profiles, activeId: keymaps.activeId === id ? null : keymaps.activeId });
  }

  function updateActiveProfile(mutate: (profile: KeymapProfile) => KeymapProfile) {
    persistKeymaps({
      ...keymaps,
      profiles: keymaps.profiles.map((profile) => (profile.id === keymaps.activeId ? mutate(profile) : profile)),
    });
  }

  /** 注入一个动作到已授权设备（映射测试与回放共用）。 */
  async function injectAction(action: KeymapAction): Promise<void> {
    const serial = readyDevice?.serial;
    if (!serial) throw new Error("需要一台已授权设备。");
    if (action.kind === "key") {
      await invoke("inject_device_key", { serial, keycode: action.keycode });
    } else if (action.kind === "tap") {
      await invoke("inject_device_tap", { serial, x: action.x, y: action.y });
    } else {
      await invoke("inject_device_swipe", {
        serial,
        x1: action.x1,
        y1: action.y1,
        x2: action.x2,
        y2: action.y2,
        durationMs: action.duration,
      });
    }
  }

  async function testBinding(binding: { physicalKey: string; label: string; action: KeymapAction }) {
    setKeymapError(null);
    setKeymapNotice(null);
    try {
      await injectAction(binding.action);
      setLastInjected(`${binding.label} → ${describeAction(binding.action)}：已发送。`);
    } catch (error) {
      setKeymapError(errorMessage(error, "注入没有成功。"));
    }
  }

  function handleTestPanelKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    const physicalKey = normalizePhysicalKey(event);
    if (!physicalKey) return;
    event.preventDefault();
    const binding = activeKeymapProfile?.bindings.find((item) => item.physicalKey === physicalKey);
    if (!binding) {
      setLastInjected(`${physicalKeyLabel(physicalKey)}：未映射，没有发送。`);
      return;
    }
    void testBinding(binding);
  }

  // 捕获映射用的物理键：全局捕获阶段监听，拿到第一个可识别的键位就停。
  useEffect(() => {
    if (!capturingKey) return;
    const onKeyDown = (event: KeyboardEvent) => {
      const physicalKey = normalizePhysicalKey(event);
      if (!physicalKey) return;
      event.preventDefault();
      setCapturedKey({ physicalKey, label: physicalKeyLabel(physicalKey) });
      setCapturingKey(false);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [capturingKey]);

  function addKeymapBinding() {
    if (!activeKeymapProfile) {
      setKeymapError("先新建或选择一个映射方案。");
      return;
    }
    if (!capturedKey) {
      setKeymapError("先点「捕获按键」并按下一个物理键。");
      return;
    }
    let action: KeymapAction;
    if (newActionKind === "key") {
      if (!isValidKeycode(newKeycode)) {
        setKeymapError("这个按键码不在可注入范围内。");
        return;
      }
      action = { kind: "key", keycode: newKeycode };
    } else if (newActionKind === "tap") {
      const x = Number.parseInt(draftTap.x, 10);
      const y = Number.parseInt(draftTap.y, 10);
      if (!isValidCoord(x) || !isValidCoord(y)) {
        setKeymapError("点击坐标必须是 0..=9999 的整数。");
        return;
      }
      action = { kind: "tap", x, y };
    } else {
      const x1 = Number.parseInt(draftSwipe.x1, 10);
      const y1 = Number.parseInt(draftSwipe.y1, 10);
      const x2 = Number.parseInt(draftSwipe.x2, 10);
      const y2 = Number.parseInt(draftSwipe.y2, 10);
      const duration = Number.parseInt(draftSwipe.duration, 10);
      if (![x1, y1, x2, y2].every(isValidCoord) || !isValidDuration(duration)) {
        setKeymapError("滑动坐标（0..=9999）或时长（50..=60000ms）超出范围。");
        return;
      }
      action = { kind: "swipe", x1, y1, x2, y2, duration };
    }
    // 同一个物理键只保留一条映射：后加的替换旧的，避免隐式双发。
    updateActiveProfile((profile) => ({
      ...profile,
      bindings: [
        ...profile.bindings.filter((binding) => binding.physicalKey !== capturedKey.physicalKey),
        { physicalKey: capturedKey.physicalKey, label: capturedKey.label, action },
      ],
    }));
    setCapturedKey(null);
    setKeymapError(null);
    setKeymapNotice(`已把 ${capturedKey.label} 映射到「${describeAction(action)}」（同键旧映射已替换）。`);
  }

  function removeKeymapBinding(physicalKey: string) {
    updateActiveProfile((profile) => ({
      ...profile,
      bindings: profile.bindings.filter((binding) => binding.physicalKey !== physicalKey),
    }));
  }

  // -- X10-130 操作回放 --

  const activeReplay = activeReplayId ? replays.find((script) => script.id === activeReplayId) ?? null : null;

  function persistReplays(next: ReplayScript[]) {
    setReplays(next);
    writeReplays(next);
  }

  // X10-140 回放脚本导出 / 导入：与映射方案同一套文件读写与校验策略。
  async function exportReplays() {
    setReplayError(null);
    setReplayNotice(null);
    try {
      const path = await saveFilePicker({
        title: "导出回放脚本",
        defaultPath: "mirrordock-replays.json",
        filters: [{ name: "JSON 文件", extensions: ["json"] }],
      });
      if (!path) return;
      await invoke("write_export_file", { path, contents: replaysToJson(replays) });
      setReplayNotice("回放脚本已导出，可以在其他电脑上用「导入脚本」恢复。");
    } catch (error) {
      setReplayError(errorMessage(error, "导出没有完成。"));
    }
  }

  async function importReplays() {
    setReplayError(null);
    setReplayNotice(null);
    try {
      const path = await openFilePicker({
        title: "导入回放脚本",
        multiple: false,
        filters: [{ name: "JSON 文件", extensions: ["json"] }],
      });
      if (typeof path !== "string" || path.length === 0) return;
      const rawText = await invoke<string>("read_import_file", { path });
      const parsed = parseImportedReplays(JSON.parse(rawText));
      if (!parsed) {
        setReplayError("这个文件不是 MirrorDock 导出的回放脚本；没有做任何改动。");
        return;
      }
      // 与本地脚本去重（按 id 覆盖同名脚本，其余追加）。
      const merged = [...replays];
      for (const script of parsed) {
        const index = merged.findIndex((item) => item.id === script.id);
        if (index >= 0) merged[index] = script;
        else merged.push(script);
      }
      persistReplays(merged);
      setReplayNotice(`已导入 ${parsed.length} 个回放脚本；坏步已自动跳过。`);
    } catch {
      setReplayError("读取或解析导入文件没有成功；没有做任何改动。");
    }
  }

  function createReplayScript() {
    const script: ReplayScript = {
      id: newLocalId(),
      name: `脚本 ${replays.length + 1}`,
      steps: [],
    };
    persistReplays([...replays, script]);
    setActiveReplayId(script.id);
    setReplayError(null);
    setReplayNotice(null);
    setReplayStatus(null);
  }

  function deleteReplayScript(id: string) {
    persistReplays(replays.filter((script) => script.id !== id));
    if (activeReplayId === id) setActiveReplayId(null);
  }

  function updateActiveScript(mutate: (script: ReplayScript) => ReplayScript) {
    persistReplays(replays.map((script) => (script.id === activeReplayId ? mutate(script) : script)));
  }

  function addReplayStep() {
    if (!activeReplay) {
      setReplayError("先新建或选择一个回放脚本。");
      return;
    }
    const waitMs = clampWaitMs(Number.parseInt(draftStepWait, 10) || 0);
    let step: ReplayStep;
    if (draftStepKind === "key") {
      if (!isValidKeycode(newKeycode)) {
        setReplayError("这个按键码不在可注入范围内。");
        return;
      }
      step = { kind: "key", keycode: newKeycode, waitMs };
    } else if (draftStepKind === "tap") {
      const x = Number.parseInt(draftTap.x, 10);
      const y = Number.parseInt(draftTap.y, 10);
      if (!isValidCoord(x) || !isValidCoord(y)) {
        setReplayError("点击坐标必须是 0..=9999 的整数。");
        return;
      }
      step = { kind: "tap", x, y, waitMs };
    } else {
      const x1 = Number.parseInt(draftSwipe.x1, 10);
      const y1 = Number.parseInt(draftSwipe.y1, 10);
      const x2 = Number.parseInt(draftSwipe.x2, 10);
      const y2 = Number.parseInt(draftSwipe.y2, 10);
      const duration = Number.parseInt(draftSwipe.duration, 10);
      if (![x1, y1, x2, y2].every(isValidCoord) || !isValidDuration(duration)) {
        setReplayError("滑动坐标（0..=9999）或时长（50..=60000ms）超出范围。");
        return;
      }
      step = { kind: "swipe", x1, y1, x2, y2, duration, waitMs };
    }
    updateActiveScript((script) => ({ ...script, steps: [...script.steps, step] }));
    setReplayError(null);
  }

  function moveReplayStep(index: number, delta: number) {
    updateActiveScript((script) => {
      const steps = [...script.steps];
      const target = index + delta;
      if (target < 0 || target >= steps.length) return script;
      [steps[index], steps[target]] = [steps[target], steps[index]];
      return { ...script, steps };
    });
  }

  function removeReplayStep(index: number) {
    updateActiveScript((script) => ({ ...script, steps: script.steps.filter((_, i) => i !== index) }));
  }

  async function runReplayScript(script: ReplayScript) {
    const serial = readyDevice?.serial;
    if (!serial) {
      setReplayError("需要一台已授权设备才能回放。");
      return;
    }
    const problem = validateReplayScript(script.steps);
    if (problem) {
      setReplayError(problem);
      return;
    }
    setReplayError(null);
    setReplayNotice(null);
    setReplayStatus(script.steps.map(() => "待执行"));
    setReplayRunning(true);
    replayCancelRef.current = false;
    const sleep = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));
    try {
      for (let index = 0; index < script.steps.length; index += 1) {
        if (replayCancelRef.current) {
          setReplayNotice(`已停止：前 ${index} 步已执行，第 ${index + 1} 步及之后未执行。`);
          return;
        }
        const step = script.steps[index];
        if (step.waitMs > 0) await sleep(step.waitMs);
        if (replayCancelRef.current) {
          setReplayNotice(`已停止：前 ${index} 步已执行，第 ${index + 1} 步及之后未执行。`);
          return;
        }
        setReplayStatus((prev) => (prev ? prev.map((value, i) => (i === index ? "执行中…" : value)) : prev));
        try {
          await injectAction(step);
          setReplayStatus((prev) => (prev ? prev.map((value, i) => (i === index ? "已执行" : value)) : prev));
        } catch (error) {
          setReplayStatus((prev) => (prev ? prev.map((value, i) => (i === index ? "失败" : value)) : prev));
          setReplayError(`第 ${index + 1} 步执行失败：${errorMessage(error, "注入没有成功。")}`);
          return;
        }
      }
      setReplayNotice(`回放完成：共 ${script.steps.length} 步。`);
    } finally {
      setReplayRunning(false);
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
    if (view !== "files" && tab !== "tools") return;
    if (deviceFiles !== null || transferBusy) return;
    if (!transferDeviceSerial) return;
    void refreshDeviceFiles(transferDeviceSerial);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, tab, transferDeviceSerial]);

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
    // X10-137：拖入 1 个 APK 保持原「直接安装」；拖入多个则全部进批量安装队列，
    // 不再只装第一个、其余静默丢弃。
    if (apks.length === 1) void installDroppedApk(apks);
    else if (apks.length > 1) enqueueDroppedApks(apks);
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
  // X10-105：录制进程在镜像仍存活时闪退——后端已自清状态并推本事件，前端把录制
  // 按钮从「录制中」复位回「开始录制」，并提示用户（区别于手动结束的「已定型可播放」，
  // 闪退的 MP4 可能缺 moov 索引无法播放）。
  useEffect(() => {
    const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!hasTauri) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<{ serial: string; unexpected: boolean }>("recording-ended", (event) => {
      if (event.payload?.unexpected) {
        setRecording(null);
        setRecordingError("录制意外中断（可能因手机性能不足或网络波动），镜像未受影响。中断的录像文件可能无法播放。");
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
    // P0-3：手机前台点「发送剪贴板到电脑」后上行的文字。只进内存，
    // 用户点「复制」才写入系统剪贴板——不自动替换用户剪贴板（可见可撤销）。
    let unlistenClipboard: (() => void) | null = null;
    void listen<{ text: string }>("companion-clipboard", (event) => {
      if (!disposed && event.payload && typeof event.payload.text === "string") {
        setIncomingClipboard(event.payload.text);
        setClipboardNotice(null);
      }
    }).then((dispose) => {
      if (disposed) dispose();
      else unlistenClipboard = dispose;
    });
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
      if (unlistenClipboard) unlistenClipboard();
    };
  }, []);
  // 收到新文件事件后的实际动作：**无条件加载列表**，不再要求用户先点过一次按钮。
  // 旧实现要求 `deviceFiles !== null` 才刷新，而 deviceFiles 只在手动点按钮后
  // 才赋值 —— 于是形成死锁：不点按钮就永远看不到新文件（用户实际报的现象）。
  // 文件传输不依赖镜像会话，所以这里也不看 sessionActive。
  useEffect(() => {
    if (filesChangedHint === null) return;
    setTransferMessage("手机发送区有新文件到达。");
    // 切到文件传输页并自动刷新：新文件是用户等着要看的东西，不该要求他先点按钮。
    openView("files");
    if (transferDeviceSerial && !transferBusy) {
      void refreshDeviceFiles(transferDeviceSerial);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filesChangedHint]);
  // X10-129 相册视图打开时自动读取当前目录：与文件传输同策略，不要求先点按钮。
  // 目录切换由 chooseGalleryDir 自己刷新；这里只负责「进页面 / 换设备」两个时机。
  useEffect(() => {
    if (view !== "album") return;
    if (galleryFiles !== null || galleryBusy) return;
    if (!transferDeviceSerial) return;
    void refreshGallery(galleryDir);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, transferDeviceSerial]);
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

  const isLegacyView = LEGACY_VIEWS.has(view);
  const currentDeviceLabel = readyDevice
    ? displayLabels[readyDevice.serial] ?? readyDevice.label
    : null;
  const appsTargetSerial = transferDeviceSerial ?? readySerial ?? sessionSerial;
  const visibleDeviceApps = [...(deviceApps[appsTargetSerial ?? ""] ?? [])].sort((a, b) =>
    appsSort === "package"
      ? a.package.localeCompare(b.package)
      : (a.name || a.package).localeCompare(b.name || b.package),
  );

  useEffect(() => {
    if (view !== "apps" || !appsTargetSerial) return;
    void ensureDeviceApps(appsTargetSerial);
  }, [view, appsTargetSerial, ensureDeviceApps]);

  // X10-136：进入设备信息视图自动读取一次电池与存储；换设备即重读。
  // X10-150：设备中心 Hero 卡需要真实电量，进入设备中心也自动读取。
  useEffect(() => {
    if ((view !== "info" && view !== "home") || !readySerial) return;
    if (powerSerial === readySerial && (powerStorage || powerBusy)) return;
    void loadPowerStorage(readySerial);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, readySerial]);

  function runViewAction() {
    switch (view) {
      case "home":
      case "connect":
        openView("connect");
        break;
      case "info":
        void previewDiagnostics();
        break;
      case "multi":
        void refreshDevices();
        break;
      case "mirror":
        if (readyDevice) void startMirroring(readyDevice.serial);
        else openView("connect");
        break;
      case "keymap":
        if (readySerial) void openKeyboardSettings();
        else openView("connect");
        break;
      case "files":
        if (transferDeviceSerial) void refreshDeviceFiles(transferDeviceSerial);
        break;
      case "shot":
        if (readyDevice) void captureScreen(readyDevice.serial);
        else openView("connect");
        break;
      case "record":
        void toggleRecording();
        break;
      case "install":
        void pickApk();
        break;
      case "present":
        if (readyDevice) void startMirroring(readyDevice.serial);
        else openView("connect");
        break;
      case "schedule":
        addRecordPlan();
        break;
      // X10-129/130：页头快捷按钮与视图内主操作保持同一行为。
      case "album":
        void refreshGallery();
        break;
      case "replay":
        if (!readyDevice) openView("connect");
        else if (activeReplay) void runReplayScript(activeReplay);
        break;
      case "apps":
        if (appsTargetSerial) void ensureDeviceApps(appsTargetSerial);
        break;
      case "notify":
        // P0-2：页头「清空通知」走同步命令，手机不在线时不清本地、明确提示。
        void clearNotifications();
        break;
      case "settings":
        openView("settings");
        break;
      case "help":
        openView("help");
        break;
      case "pro":
        openView("settings");
        break;
      default:
        break;
    }
  }

  // X10-150：设备卡动作簇抽取——连接卡设备列表与设备中心 Hero 卡共用同一份真实动作，
  // 保证「每个动作全页只出现一次」；文案、禁用条件与事件一字未改。
  function deviceActions(device: Device, owned: boolean, favorite: boolean, usbAlt: string | null) {
    return (
      <>
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
              <>
                <button className="primary-button" type="button" disabled={!scrcpyReady || isLaunching || isStopping} onClick={() => void startMirroring(device.serial)}>
                  {launchingSerial === device.serial ? "正在启动…" : scrcpyReady ? "开始镜像" : "镜像引擎准备中"}
                </button>
                {/* X10-111（B）：首选是无线但有 ready USB 时，给一键改用有线的入口。 */}
                {usbAlt && (
                  <button
                    className="secondary-button"
                    type="button"
                    disabled={!scrcpyReady || isLaunching || isStopping}
                    title="检测到你插着数据线——有线更稳、不易断连，点击改走 USB 启动镜像"
                    onClick={() => void startMirroring(usbAlt)}
                  >
                    {launchingSerial === usbAlt ? "正在启动…" : "改用有线"}
                  </button>
                )}
              </>
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
      </>
    );
  }

  function assistantSections(current: AssistantViewKey): AssistantSection[] {
    switch (current) {
      case "info": {
        // X10-136：电池与存储摘要——只在数据属于当前设备时显示，换设备即失效。
        const power = powerSerial && powerSerial === readySerial ? powerStorage : null;
        const batteryText = power
          ? [
              power.battery_level !== null ? `电量 ${power.battery_level}%` : null,
              power.charging === true ? "充电中" : power.charging === false ? "未充电" : null,
              power.temperature !== null ? `${power.temperature.toFixed(1)}°C` : null,
            ]
              .filter(Boolean)
              .join(" · ") || "字段缺失"
          : powerError ?? (powerBusy ? "读取中…" : "尚未读取");
        const storageText = power
          ? power.storage_total
            ? `总 ${power.storage_total} · 可用 ${power.storage_free ?? "未知"}`
            : "未读取到存储信息"
          : powerError ?? (powerBusy ? "读取中…" : "尚未读取");
        return [
          {
            kind: "kv",
            title: "只读设备摘要",
            tone: 6,
            items: [
              ["当前设备", currentDeviceLabel ?? topbarStatus],
              ["系统", capabilities?.android_release ? `Android ${capabilities.android_release}` : "未知"],
              ["镜像能力", capabilities ? capabilitySummary(capabilities) : "尚未探测"],
              ["锁屏 / 屏幕", readyDevice && lockReports[readyDevice.serial] ? lockSummary(lockReports[readyDevice.serial]) : "尚未探测"],
              ["电池", batteryText],
              ["存储", storageText],
            ],
          },
          {
            kind: "actions",
            actions: [
              { label: "刷新电池 / 存储", onClick: () => readySerial && void loadPowerStorage(readySerial), disabled: powerBusy || !readySerial },
              { label: "预览诊断内容", onClick: () => void previewDiagnostics(), disabled: diagnosticsBusy },
              { label: "导出诊断包", onClick: () => void exportDiagnostics(), disabled: diagnosticsBusy || !diagnostics },
            ],
          },
          ...(diagnostics ? [{ kind: "kv" as const, title: "诊断预览", tone: 6, items: [["版本", diagnostics.app_version], ["系统", diagnostics.system], ["镜像引擎", diagnostics.scrcpy_available ? "可用" : "不可用"], ["最近事件", `${diagnostics.events.length} 条`]] as [string, string][] }] : []),
          { kind: "note", text: diagnosticsError ?? "诊断内容只在本机生成与导出，不上传；不包含剪贴板、口令或画面帧。" },
        ];
      }
      case "multi":
        return [
          ...(multiCaptureSummary
            ? [{ kind: "note" as const, text: `全部截图结论：${multiCaptureSummary}` }]
            : []),
          {
            kind: "table",
            title: "当前设备",
            tone: 8,
            columns: ["设备", "状态", "通道", "操作"],
            rows: (check?.devices ?? []).map((device) => ({
              cells: [
                displayLabels[device.serial] ?? device.label,
                device.state,
                connectionLabel(device) || "—",
                // X10-132：逐设备快捷操作，直接复用开始镜像 / 截图的真实命令。
                <span key={device.serial} className="assistant-row-actions">
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={device.state !== "ready" || isLaunching || launchingSerial === device.serial}
                    onClick={() => void startMirroring(device.serial)}
                  >
                    {launchingSerial === device.serial ? "启动中…" : "镜像"}
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    disabled={device.state !== "ready" || screenshotBusy}
                    onClick={() => void captureScreen(device.serial)}
                  >
                    截图
                  </button>
                </span>,
              ],
              filterText: `${device.label} ${device.state}`,
            })),
            empty: "未检测到设备。插上数据线并授权后，这里会显示真实设备。",
          },
          {
            kind: "table",
            title: "镜像会话",
            tone: 8,
            columns: ["设备", "阶段", "首帧"],
            rows: activeSessionList.map((item) => ({
              cells: [item.serial ?? "—", item.phase, item.first_frame === "reached" ? "已到达" : "未确认"],
              filterText: `${item.serial ?? ""} ${item.phase}`,
            })),
            empty: "当前没有镜像会话。",
          },
          {
            kind: "actions",
            actions: [
              { label: "刷新设备", onClick: () => void refreshDevices(), disabled: isChecking },
              // X10-139：对所有就绪设备各截一张，文件名带序列号不互相覆盖。
              { label: "一键全部截图", onClick: () => void captureAllScreens(), disabled: screenshotBusy },
              { label: "结束全部", onClick: () => void stopMirroring(), disabled: isStopping || activeSessionCount === 0, danger: true },
            ],
          },
          { kind: "note", text: "多端视图只读本机设备与会话；设备之间不互传文件，也不需要账号或云端。" },
        ];
      case "mirror":
        return [
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head">
                  <h3 className="card-title">镜像状态</h3>
                  <span className="spacer" />
                  <span className={`badge ${sessionActive ? "ok" : "neutral"}`}>{sessionActive ? "会话进行中" : "未开始"}</span>
                </div>
                <div className="card-body">
                  {/* X10-149：失败原因直接显示在状态行（发起错误 > 会话错误 > 常规状态）。
                      之前 launchError / sessionError 只渲染在主页状态栈，镜像页点了
                      「开始镜像」失败时毫无反馈，用户感知为「窗口不显示、没反应」。 */}
                  {(() => {
                    const mirrorError = launchError ?? sessionError ?? (session?.error ? sessionErrorText(session, "") : null);
                    return (
                      <div className={`mirror-state ${mirrorError ? "error" : ""}`} role={mirrorError ? "alert" : undefined}>
                        <span className={`dot ${mirrorError ? "off" : sessionActive ? "busy" : "off"}`} aria-hidden="true" />
                        <strong>{mirrorError ?? statusMessage ?? "尚未开始镜像"}</strong>
                      </div>
                    );
                  })()}
                  <p className="hint">
                    {session
                      ? `阶段：${session.phase} · 首帧：${session.first_frame === "reached" ? "已到达" : "未确认"}`
                      : "设备授权后即可开始"}
                    。画面在<b>独立的镜像窗口</b>中打开，本页只负责发起、结束与参数调整，不重复显示画面。
                  </p>
                </div>
              </section>
            ),
          },
          {
            kind: "kv",
            title: "会话参数",
            tone: 2,
            items: [
              ["目标设备", currentDeviceLabel ?? "未选择设备"],
              ["首选通道", readyDevice ? preferredChannelLabel(readyDevice) : "—"],
              ["画质", options.quality === "smooth" ? "流畅" : options.quality === "sharp" ? "高清" : "均衡"],
              ["桌面模式", options.desktop_mode ? "开启" : "关闭"],
              ["视频录制", options.record ? (proEdition ? "已启用" : "专业版未激活") : "未启用"],
            ],
          },
          {
            kind: "actions",
            actions: [
              { label: "开始镜像", primary: true, onClick: () => readyDevice ? void startMirroring(readyDevice.serial) : openView("connect"), disabled: isLaunching },
              { label: "结束镜像", onClick: () => void stopMirroring(), disabled: isStopping || activeSessionCount === 0, danger: true },
              { label: "设备信息", onClick: () => openView("info") },
            ],
          },
          { kind: "note", text: "受保护的支付、密码等页面会由系统限制为黑屏，这不是镜像故障。会话可随时结束。" },
        ];
      case "present":
        return [
          {
            kind: "checklist",
            title: "就绪检查",
            tone: 3,
            items: [
              { label: "目标设备已连接", ok: !!readyDevice, why: readyDevice ? (displayLabels[readyDevice.serial] ?? readyDevice.label) : (
                <>未连接手机 <button type="button" className="btn-ghost" onClick={() => openView("connect")}>去连接</button></>
              ) },
              { label: "镜像引擎可用", ok: scrcpyReady, why: scrcpyReady ? "scrcpy 4.1" : "不可用" },
              { label: "桌面模式", ok: true, why: options.desktop_mode ? "开启" : "关闭" },
              { label: "活动会话", ok: activeSessionCount > 0, why: activeSessionCount > 0 ? `${activeSessionCount} 台进行中` : "未开始" },
            ],
          },
          {
            kind: "actions",
            actions: [
              { label: "开始演示", primary: true, onClick: () => readyDevice ? void startMirroring(readyDevice.serial) : openView("connect"), disabled: isLaunching || !scrcpyReady },
              { label: "结束演示", onClick: () => void stopMirroring(), disabled: isStopping || activeSessionCount === 0, danger: true },
              { label: options.desktop_mode ? "关闭桌面模式" : "开启桌面模式", onClick: () => void applyOptionsUpdate({ ...options, desktop_mode: !options.desktop_mode }), disabled: applyingOptions },
            ],
          },
          { kind: "note", text: "演示 = 按当前画质与桌面模式设置发起真实镜像会话；会话画面在独立镜像窗口呈现，可全屏投到外部显示器。激光笔 / 白板标注、演示文稿管理尚未实现，这里不伪装。" },
        ];
      case "keymap": {
        const profile = activeKeymapProfile;
        return [
          {
            kind: "table",
            title: "本机全局快捷键",
            tone: 6,
            columns: ["动作", "快捷键"],
            rows: [
              { cells: ["截图", shortcuts.screenshot], filterText: "截图" },
              { cells: ["录制开关", shortcuts.record], filterText: "录制" },
              { cells: ["旋转画面", shortcuts.rotate], filterText: "旋转" },
            ],
          },
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head"><h3 className="card-title">键位映射方案</h3><span className="spacer" /></div>
                <div className="card-body">
                  <div className="assistant-chip-row" role="group" aria-label="映射方案">
                    {keymaps.profiles.map((item) => (
                      <button
                        key={item.id}
                        type="button"
                        className={`chip${keymaps.activeId === item.id ? " active" : ""}`}
                        onClick={() => persistKeymaps({ ...keymaps, activeId: item.id })}
                      >
                        {item.name}
                      </button>
                    ))}
                    <button type="button" className="chip" onClick={createKeymapProfile}>＋ 新建方案</button>
                  </div>
                  {profile ? (
                    <div className="plan-grid" style={{ marginTop: 12 }}>
                      <article className="plan-card">
                        <h4>{profile.name}<span className="badge neutral">{profile.bindings.length} 步</span></h4>
                        {profile.bindings.length > 0 ? profile.bindings.map((binding) => (
                          <div className="step-row" key={binding.physicalKey}>
                            <span>{binding.label}</span>
                            <span className="mono"><kbd className="kbd">{binding.label}</kbd></span>
                            <span className="mono">{describeAction(binding.action)}</span>
                            <span className="detail-ops">
                              <button type="button" className="btn-ghost" disabled={!readyDevice} onClick={() => void testBinding(binding)}>测试</button>
                              <button type="button" className="btn-ghost danger" onClick={() => removeKeymapBinding(binding.physicalKey)}>删除</button>
                            </span>
                          </div>
                        )) : <p className="hint">这个方案还没有映射条目；在下方添加。</p>}
                        <div className="detail-row">
                          <button
                            type="button"
                            className={`btn btn-sm${capturingKey ? " btn-primary" : " btn-secondary"}`}
                            onClick={() => setCapturingKey(true)}
                          >
                            {capturingKey ? "请按下任意键…" : capturedKey ? `物理键：${capturedKey.label}` : "捕获按键"}
                          </button>
                          <select className="select" value={newActionKind} onChange={(event) => setNewActionKind(event.target.value as typeof newActionKind)} aria-label="注入动作类型">
                            <option value="key">按键</option>
                            <option value="tap">点击</option>
                            <option value="swipe">滑动</option>
                          </select>
                          {newActionKind === "key" && (
                            <select className="select" value={newKeycode} onChange={(event) => setNewKeycode(event.target.value)} aria-label="Android 按键码">
                              {INJECTABLE_KEYCODES.map((item) => (
                                <option key={item.value} value={item.value}>{item.label}</option>
                              ))}
                            </select>
                          )}
                          {newActionKind === "tap" && (
                            <span className="detail-row">
                              X <input className="input mono" style={{ width: 64 }} value={draftTap.x} onChange={(event) => setDraftTap({ ...draftTap, x: event.target.value })} inputMode="numeric" aria-label="点击 X 坐标" />
                              Y <input className="input mono" style={{ width: 64 }} value={draftTap.y} onChange={(event) => setDraftTap({ ...draftTap, y: event.target.value })} inputMode="numeric" aria-label="点击 Y 坐标" />
                            </span>
                          )}
                          {newActionKind === "swipe" && (
                            <span className="detail-row">
                              起 X <input className="input mono" style={{ width: 58 }} value={draftSwipe.x1} onChange={(event) => setDraftSwipe({ ...draftSwipe, x1: event.target.value })} inputMode="numeric" aria-label="滑动起点 X" />
                              Y <input className="input mono" style={{ width: 58 }} value={draftSwipe.y1} onChange={(event) => setDraftSwipe({ ...draftSwipe, y1: event.target.value })} inputMode="numeric" aria-label="滑动起点 Y" />
                              终 X <input className="input mono" style={{ width: 58 }} value={draftSwipe.x2} onChange={(event) => setDraftSwipe({ ...draftSwipe, x2: event.target.value })} inputMode="numeric" aria-label="滑动终点 X" />
                              Y <input className="input mono" style={{ width: 58 }} value={draftSwipe.y2} onChange={(event) => setDraftSwipe({ ...draftSwipe, y2: event.target.value })} inputMode="numeric" aria-label="滑动终点 Y" />
                              时长 <input className="input mono" style={{ width: 72 }} value={draftSwipe.duration} onChange={(event) => setDraftSwipe({ ...draftSwipe, duration: event.target.value })} inputMode="numeric" aria-label="滑动时长毫秒" />
                            </span>
                          )}
                          <button type="button" className="btn btn-primary btn-sm" onClick={addKeymapBinding}>添加到方案</button>
                        </div>
                        {keymapError && <p className="field-error" role="alert">{keymapError}</p>}
                        {keymapNotice && <p className="hint" role="status">{keymapNotice}</p>}
                        <div className="plan-foot">
                          {/* X10-140：方案备份与迁移，走本机 .json 文件。 */}
                          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void exportKeymaps()}>导出方案</button>
                          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void importKeymaps()}>导入方案</button>
                          <button type="button" className="btn-ghost danger" style={{ marginInlineStart: "auto" }} onClick={() => deleteKeymapProfile(profile.id)}>删除方案</button>
                        </div>
                      </article>
                    </div>
                  ) : (
                    <>
                      <div className="alert alert-info" style={{ marginTop: 12 }}>
                        <div className="alert-text">{keymaps.profiles.length > 0 ? `本机已恢复 ${keymaps.profiles.length} 个方案；点方案名选中后编辑。` : "还没有映射方案；点「＋ 新建方案」开始编排。"}</div>
                      </div>
                      <div className="assistant-action-row" style={{ marginTop: 8 }}>
                        <button type="button" className="btn btn-secondary btn-sm" onClick={() => void exportKeymaps()} disabled={keymaps.profiles.length === 0}>导出方案</button>
                        <button type="button" className="btn btn-secondary btn-sm" onClick={() => void importKeymaps()}>导入方案</button>
                      </div>
                    </>
                  )}
                </div>
              </section>
            ),
          },
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head"><h3 className="card-title">测试面板</h3><span className="spacer" /></div>
                <div className="card-body">
                  <div className="assistant-capture-zone" tabIndex={0} role="group" aria-label="按键测试面板" onKeyDown={handleTestPanelKeyDown}>
                    <p>点这里获得焦点，然后按已映射的物理键——动作会通过 adb 注入到「{currentDeviceLabel ?? "已授权设备"}」。未映射的键不会发送。</p>
                    {lastInjected && <p className="hint" role="status">{lastInjected}</p>}
                  </div>
                </div>
              </section>
            ),
          },
          { kind: "actions", actions: [{ label: "打开键盘设置", onClick: () => readySerial ? void openKeyboardSettings() : openView("connect") }] },
          { kind: "note", text: "映射方案只保存在本机；焦点在镜像窗口时键盘输入由 scrcpy 直输处理，映射注入在这个测试面板聚焦时生效。触控注入走 adb，待真机验证。" },
        ];
      }
      case "schedule":
        return [
          { kind: "kv", title: "录制状态", tone: 4, items: [["录制", recording?.active ? "进行中" : recording ? "已结束" : "未开始"], ["文件", recording?.file_name ?? "—"], ["计划停止", planStopRef.current ? new Date(planStopRef.current.at).toLocaleTimeString() : "无"], ["设备授权", "录制经既有会话，不绕过授权"]] },
          {
            kind: "table",
            title: recordPlans.length > 0 ? `录制计划 · ${recordPlans.length}` : "录制计划",
            tone: 4,
            columns: ["名称", "时间", "时长", "状态", "操作"],
            rows: recordPlans.map((plan) => ({
              cells: [
                plan.name,
                `每天 ${plan.time}`,
                `${plan.durationMin} 分钟`,
                <span className={`badge ${plan.enabled ? "ok" : "neutral"}`}>{plan.enabled ? "启用" : "暂停"}</span>,
                <span key={plan.id} className="detail-ops">
                  <button type="button" className="btn btn-secondary btn-sm" aria-pressed={plan.enabled} onClick={() => toggleRecordPlan(plan.id)}>{plan.enabled ? "暂停" : "启用"}</button>
                  <button type="button" className="btn-ghost danger" aria-label={`删除计划：${plan.name}`} onClick={() => removeRecordPlan(plan.id)}>删除</button>
                </span>,
              ],
              filterText: `${plan.name} ${plan.time}`,
            })),
            empty: "还没有录制计划；填写下方表单添加。",
          },
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head"><h3 className="card-title">添加计划</h3><span className="spacer" /></div>
                <div className="card-body">
                  <form className="detail-row" aria-label="添加录制计划" onSubmit={(event) => { event.preventDefault(); addRecordPlan(); }}>
                    <input className="input grow" aria-label="计划名称" placeholder="计划名称" value={draftPlan.name} onChange={(e) => setDraftPlan({ ...draftPlan, name: e.target.value })} />
                    <input className="input" aria-label="开始时间" type="time" value={draftPlan.time} onChange={(e) => setDraftPlan({ ...draftPlan, time: e.target.value })} />
                    <input className="input" style={{ width: 88 }} aria-label="时长（分钟）" type="number" min={1} max={240} value={draftPlan.duration} onChange={(e) => setDraftPlan({ ...draftPlan, duration: e.target.value })} />
                    <button type="submit" className="btn btn-primary btn-sm">添加计划</button>
                  </form>
                  {planRunLog.length > 0 && (
                    <div className="card-section" style={{ marginTop: 12 }}>
                      <strong>计划运行日志</strong>
                      <ul className="assistant-note-list" aria-label="计划运行日志">
                        {planRunLog.map((entry, index) => (
                          <li key={`${entry.at}-${index}`}>
                            <span className="assistant-muted">{new Date(entry.at).toLocaleTimeString()} · {entry.planName}</span>
                            <span className={entry.error ? "assistant-error-text" : ""}>{entry.text}</span>
                          </li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>
              </section>
            ),
          },
          { kind: "actions", actions: [{ label: "开始录制", primary: true, onClick: () => void toggleRecording(), disabled: recordingBusy }, { label: "结束录制", onClick: () => void toggleRecording(), disabled: recordingBusy || !recording?.active, danger: true }] },
          { kind: "note", text: recordingError ?? "计划只在本应用运行时触发：不做开机后台拉起、不绕过设备授权；到点时设备需已有镜像会话（录制依赖显示会话），失败原因会写进运行日志。点击计划可启停，× 可删除。" },
        ];
      case "replay": {
        const script = activeReplay;
        return [
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head"><h3 className="card-title">回放脚本</h3><span className="spacer" /></div>
                <div className="card-body">
                  <div className="assistant-chip-row" role="group" aria-label="回放脚本">
                    {replays.map((item) => (
                      <button
                        key={item.id}
                        type="button"
                        className={`chip${activeReplayId === item.id ? " active" : ""}`}
                        onClick={() => setActiveReplayId(item.id)}
                      >
                        {item.name}
                      </button>
                    ))}
                    <button type="button" className="chip" onClick={createReplayScript}>＋ 新建脚本</button>
                  </div>
                  {script ? (
                    <div className="plan-grid" style={{ marginTop: 12 }}>
                      <article className="plan-card">
                        <h4>{script.name}<span className="badge neutral">{script.steps.length} 步</span></h4>
                        {script.steps.length > 0 ? script.steps.map((step, index) => (
                          <div className="step-row" key={`${step.kind}-${index}`}>
                            <span>{index + 1} · {describeStep(step)}</span>
                            <span className="mono">{replayStatus?.[index] ?? "—"}</span>
                            <span className="mono" />
                            <span className="detail-ops">
                              <button type="button" className="btn-ghost" onClick={() => moveReplayStep(index, -1)} disabled={index === 0}>上移</button>
                              <button type="button" className="btn-ghost" onClick={() => moveReplayStep(index, 1)} disabled={index === script.steps.length - 1}>下移</button>
                              <button type="button" className="btn-ghost danger" onClick={() => removeReplayStep(index)} disabled={replayRunning}>删除</button>
                            </span>
                          </div>
                        )) : <p className="hint">脚本还没有步骤；在下方添加动作。</p>}
                        <div className="detail-row">
                          <select className="select" value={draftStepKind} onChange={(event) => setDraftStepKind(event.target.value as typeof draftStepKind)} aria-label="步骤类型">
                            <option value="tap">点击</option>
                            <option value="swipe">滑动</option>
                            <option value="key">按键</option>
                          </select>
                          {draftStepKind === "key" && (
                            <select className="select" value={newKeycode} onChange={(event) => setNewKeycode(event.target.value)} aria-label="Android 按键码">
                              {INJECTABLE_KEYCODES.map((item) => (
                                <option key={item.value} value={item.value}>{item.label}</option>
                              ))}
                            </select>
                          )}
                          {draftStepKind === "tap" && (
                            <span className="detail-row">
                              X <input className="input mono" style={{ width: 64 }} value={draftTap.x} onChange={(event) => setDraftTap({ ...draftTap, x: event.target.value })} inputMode="numeric" aria-label="点击 X 坐标" />
                              Y <input className="input mono" style={{ width: 64 }} value={draftTap.y} onChange={(event) => setDraftTap({ ...draftTap, y: event.target.value })} inputMode="numeric" aria-label="点击 Y 坐标" />
                            </span>
                          )}
                          {draftStepKind === "swipe" && (
                            <span className="detail-row">
                              起 X <input className="input mono" style={{ width: 58 }} value={draftSwipe.x1} onChange={(event) => setDraftSwipe({ ...draftSwipe, x1: event.target.value })} inputMode="numeric" aria-label="滑动起点 X" />
                              Y <input className="input mono" style={{ width: 58 }} value={draftSwipe.y1} onChange={(event) => setDraftSwipe({ ...draftSwipe, y1: event.target.value })} inputMode="numeric" aria-label="滑动起点 Y" />
                              终 X <input className="input mono" style={{ width: 58 }} value={draftSwipe.x2} onChange={(event) => setDraftSwipe({ ...draftSwipe, x2: event.target.value })} inputMode="numeric" aria-label="滑动终点 X" />
                              Y <input className="input mono" style={{ width: 58 }} value={draftSwipe.y2} onChange={(event) => setDraftSwipe({ ...draftSwipe, y2: event.target.value })} inputMode="numeric" aria-label="滑动终点 Y" />
                              时长 <input className="input mono" style={{ width: 72 }} value={draftSwipe.duration} onChange={(event) => setDraftSwipe({ ...draftSwipe, duration: event.target.value })} inputMode="numeric" aria-label="滑动时长毫秒" />
                            </span>
                          )}
                          <label className="detail-row">
                            等待(ms)
                            <input className="input mono" style={{ width: 76 }} value={draftStepWait} onChange={(event) => setDraftStepWait(event.target.value)} inputMode="numeric" aria-label="执行前等待毫秒" />
                          </label>
                          <button type="button" className="btn btn-primary btn-sm" onClick={addReplayStep}>添加步骤</button>
                        </div>
                        {replayError && <p className="field-error" role="alert">{replayError}</p>}
                        <div className="plan-foot">
                          {/* X10-140：脚本备份与迁移。 */}
                          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void exportReplays()}>导出脚本</button>
                          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void importReplays()}>导入脚本</button>
                          <button type="button" className="btn-ghost danger" style={{ marginInlineStart: "auto" }} onClick={() => deleteReplayScript(script.id)} disabled={replayRunning}>删除脚本</button>
                        </div>
                      </article>
                    </div>
                  ) : (
                    <>
                      <div className="alert alert-info" style={{ marginTop: 12 }}>
                        <div className="alert-text">{replays.length > 0 ? `本机已恢复 ${replays.length} 个脚本；点脚本名选中后编排或回放。` : "还没有回放脚本；点「＋ 新建脚本」开始编排。"}</div>
                      </div>
                      <div className="assistant-action-row" style={{ marginTop: 8 }}>
                        <button type="button" className="btn btn-secondary btn-sm" onClick={() => void exportReplays()} disabled={replays.length === 0}>导出脚本</button>
                        <button type="button" className="btn btn-secondary btn-sm" onClick={() => void importReplays()}>导入脚本</button>
                      </div>
                    </>
                  )}
                </div>
              </section>
            ),
          },
          {
            kind: "actions",
            actions: [
              { label: "开始回放", primary: true, onClick: () => script && void runReplayScript(script), disabled: replayRunning || !script || script.steps.length === 0 || !readyDevice },
              { label: "停止", onClick: () => { replayCancelRef.current = true; }, disabled: !replayRunning, danger: true },
            ],
          },
          {
            kind: "custom",
            children: (
              <details className="card details-block">
                <summary className="card-head"><h3 className="card-title">能力边界</h3><span className="spacer" /></summary>
                <div className="card-body">默认禁用操作回放，开启需二次确认；回放文件只保存在本机，不上传；回放期间设备端可见操作，不做静默注入。</div>
              </details>
            ),
          },
          { kind: "note", text: [replayNotice, "坐标以当前设备的分辨率与屏幕方向为准；脚本只保存在本机。录制镜像窗口里的真实触控需要控制通道，尚未实现——这里是编排式回放，不会伪装成录屏式回放。待真机验证。"].filter(Boolean).join(" ") },
        ];
      }
      case "files":
        return [
          {
            kind: "custom",
            children: (
              <div className="files-split">
                <section className="card">
                  <div className="card-head">                    <h3 className="card-title">手机 → 电脑</h3>
                    {deviceFiles !== null && <span className="badge neutral">{deviceFiles.length} 项</span>}
                    <span className="spacer" />
                  </div>
                  {transferGroups.length === 0 ? (
                    <div className="empty hint">
                      <p>{deviceFiles === null ? "尚未读取手机文件，点「刷新文件」查看。" : "当前没有可显示文件。"}</p>
                    </div>
                  ) : (
                    <div className="table-wrap" role="region" aria-label="手机发送区" tabIndex={0}>
                      <table className="table">
                        <thead>
                          <tr><th>名称</th><th>状态</th><th data-col="ops">操作</th></tr>
                        </thead>
                        <tbody>
                          {transferGroups.flatMap((group) => group.files.map((name) => (
                            <tr key={name}>
                              <td className="cell-main">{name}</td>
                              <td>{fetchedFiles[name] ? <span className="badge ok">已取回</span> : fetchErrors[name] ?? <span className="badge neutral">可取回</span>}</td>
                              <td data-col="ops">
                                {fetchingName === name ? "正在取回…" : fetchedFiles[name] ? "完成" : fetchErrors[name] ? (
                                  <button type="button" className="btn-ghost" disabled={fetchingName !== null} onClick={() => void fetchDeviceFile(transferDeviceSerial ?? "", name)}>重试</button>
                                ) : (
                                  <button type="button" className="btn-ghost" disabled={fetchingName !== null} onClick={() => void fetchDeviceFile(transferDeviceSerial ?? "", name)}>取回到电脑</button>
                                )}
                                {" · "}
                                <button type="button" className="btn-ghost" disabled={fetchingName !== null} onClick={() => void confirmDeleteDeviceFile(name)}>删除</button>
                              </td>
                            </tr>
                          )))}
                        </tbody>
                      </table>
                    </div>
                  )}
                  <div className="card-foot">
                    <button type="button" className="btn btn-primary" onClick={() => transferDeviceSerial && void refreshDeviceFiles(transferDeviceSerial)} disabled={transferBusy}>刷新文件</button>
                    <button type="button" className="btn btn-secondary" onClick={() => void openFetchedFolder()}>打开文件夹</button>
                    <span className="spacer" />
                    <span>{deviceFiles ? `共 ${deviceFiles.length} 个文件${fetchedCount > 0 ? `，${fetchedCount} 个已取回` : ""}` : ""}</span>
                  </div>
                </section>
                <section className="card">
                  <div className="card-head"><h3 className="card-title">电脑 → 手机</h3><span className="spacer" /></div>
                  <div className="card-body">
                    <button
                      type="button"
                      className="dropzone"
                      disabled={!transferDeviceSerial || transferBusy}
                      onClick={() => transferDeviceSerial && void sendFileTo(transferDeviceSerial)}
                    >
                      把文件拖到这里，或点此选择推送到手机
                      <small>{transferDeviceSerial ? "推送后需在手机上确认接收" : "连接设备后可用"}</small>
                    </button>
                    <p className="setting-note">文件通道走 ADB，不依赖镜像会话；读取失败会明确报错，不会伪装成「没有文件」。</p>
                  </div>
                </section>
              </div>
            ),
          },
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head"><h3 className="card-title">剪贴板（手动）</h3><span className="badge neutral">不自动监听</span><span className="spacer" /></div>
                <div className="card-body">
                  {incomingClipboard && (
                    <div className="alert alert-info" role="status">
                      <div className="alert-text">
                        <b>手机发来一段剪贴板文字</b>
                        <p className="clipboard-preview">{incomingClipboard}</p>
                      </div>
                      <div className="alert-actions">
                        <button type="button" className="btn btn-primary" onClick={() => void acceptIncomingClipboard()}>复制到本机</button>
                        <button type="button" className="btn btn-secondary" onClick={() => setIncomingClipboard(null)}>忽略</button>
                      </div>
                    </div>
                  )}
                  <button type="button" className="btn btn-secondary" onClick={() => void sendClipboardToPhone()}>发送本机剪贴板到手机</button>
                  <p className="setting-note">两端都只在点按钮时读写，不后台监听、不上传；文字仅在本机与已配对手机之间直传，不进日志。写手机剪贴板后手机会有系统提示。</p>
                  {clipboardNotice && <p className="hint" role="status">{clipboardNotice}</p>}
                </div>
              </section>
            ),
          },
          {
            kind: "note",
            text: [transferError, transferMessage].filter(Boolean).join(" "),
          },
        ];
      case "shot":
        return [
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head">                  <h3 className="card-title">最近截图</h3>
                  {screenshot && <span className="badge neutral">{formatBytes(screenshot.bytes)}</span>}
                  <span className="spacer" />
                </div>
                {screenshot ? (
                  <div className="card-body">
                    <div className="thumb-strip">
                      <div className="thumb" role="button" tabIndex={0} title={screenshot.path}>
                        <span>{screenshot.file_name}</span>
                        <button type="button" className="icon-btn" aria-label="删除这张截图" onClick={() => void undoCapture()} disabled={screenshotBusy}>×</button>
                      </div>
                    </div>
                    <p className="screenshot-path">{screenshot.path}</p>
                  </div>
                ) : (
                  <div className="empty hint">
                    <p>{screenshotError ?? "本机还没有新截图；点「立即截图」获取第一张。"}</p>
                  </div>
                )}
                <div className="card-foot">
                  <button type="button" className="btn btn-primary" onClick={() => readyDevice ? void captureScreen(readyDevice.serial) : openView("connect")} disabled={screenshotBusy}>立即截图</button>
                  <button type="button" className="btn btn-secondary" onClick={() => void revealCapture()} disabled={!screenshot}>在文件夹中显示</button>
                </div>
              </section>
            ),
          },
          { kind: "note", text: "截图默认保存到本机 MirrorDock 目录；受保护页面会截成黑屏。" },
        ];
      case "record":
        return [
          ...(!proEdition ? [{ kind: "note" as const, kind_of_alert: "info" as const, text: "录像是专业版功能；未激活时核心镜像与传输保持免费可用。" }] : []),
          ...(recording?.active ? [{
            kind: "custom" as const,
            children: (
              <section className="card">
                <div className="card-body" style={{ display: "flex", alignItems: "center", gap: 12 }}>
                  <span className="dot busy" aria-hidden="true" />
                  <strong style={{ fontSize: 20 }}>{recording.file_name}</strong>
                  <span className="hint">正在录制到本机</span>
                </div>
              </section>
            ),
          }] : []),
          { kind: "kv", title: "录像参数", tone: 5, items: [["状态", recording?.active ? `正在录制：${recording.file_name}` : recording ? "已结束" : "未开始"], ["文件", recording?.file_name ?? "—"], ["保存位置", appSettings.recording_dir ?? "本机视频 / MirrorDock"], ["授权", "手动启停，不重启镜像"]] },
          { kind: "actions", actions: [{ label: "开始录制", primary: true, onClick: () => void toggleRecording(), disabled: recordingBusy }, { label: "结束录制", onClick: () => void toggleRecording(), disabled: recordingBusy || !recording?.active }, { label: "在文件夹中显示", onClick: () => void revealRecording(), disabled: !recording }] },
          { kind: "note", text: recordingError ?? "录像只保存在本机，不默认云录制；「结束录制」无确认，误停止可重新开始。" },
        ];
      case "album":
        return [
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head">                  <h3 className="card-title">相册目录</h3>
                  <span className="badge neutral">{GALLERY_DIRS.find((item) => item.dir === galleryDir)?.label ?? galleryDir}</span>
                  <span className="spacer" />
                </div>
                <div className="card-body tight">
                  <div className="assistant-chip-row" role="group" aria-label="相册目录">
                    {GALLERY_DIRS.map((item) => (
                      <button
                        key={item.dir}
                        type="button"
                        className={`assistant-chip${galleryDir === item.dir ? " active" : ""}`}
                        disabled={galleryBusy || !transferDeviceSerial}
                        onClick={() => chooseGalleryDir(item.dir)}
                      >
                        {item.label}
                      </button>
                    ))}
                  </div>
                </div>
                {galleryFiles === null || galleryFiles.length === 0 ? (
                  <div className="empty hint">
                    <p>{galleryFiles === null ? "尚未读取相册目录；连接设备后会自动读取，也可以点「刷新相册」。" : "这个目录里没有照片或视频。"}</p>
                  </div>
                ) : (
                  <div className="table-wrap" role="region" aria-label="相册文件" tabIndex={0}>
                    <table className="table">
                      <thead>
                        <tr><th>文件</th><th>状态</th><th data-col="ops">操作</th></tr>
                      </thead>
                      <tbody>
                        {galleryFiles.map((name) => (
                          <tr key={name}>
                            <td className="cell-main">{name}</td>
                            <td>{galleryFetched[name] ? <span className="badge ok">已取回</span> : galleryErrors[name] ?? <span className="badge neutral">可取回</span>}</td>
                            <td data-col="ops">
                              {fetchingGalleryName === name ? "正在取回…" : galleryFetched[name] ? "完成" : (
                                <button type="button" className="btn-ghost" disabled={fetchingGalleryName !== null} onClick={() => void fetchGalleryFile(name)}>
                                  {galleryErrors[name] ? "重试" : "取回到电脑"}
                                </button>
                              )}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
                <div className="card-foot">
                  <button type="button" className="btn btn-primary" onClick={() => void refreshGallery()} disabled={galleryBusy || !transferDeviceSerial}>刷新相册</button>
                  {/* X10-138：一键取回本目录全部未取回的文件，逐个执行、失败不阻断。 */}
                  <button type="button" className="btn btn-secondary" onClick={() => void fetchAllGalleryFiles()} disabled={galleryBusy || fetchingGalleryName !== null || !transferDeviceSerial || galleryFiles === null || (galleryFiles ?? []).every((name) => galleryFetched[name])}>一键取回全部</button>
                  <span className="spacer" />
                  <span>文件保存到本机「下载 / MirrorDock / 相册」</span>
                </div>
              </section>
            ),
          },
          { kind: "note", text: [galleryError, galleryMessage, "相册为只读浏览，只提供这几个公开媒体目录；其他位置请用文件传输。读取失败会明确报错，不上传任何内容。"].filter(Boolean).join(" ") },
        ];
      case "install":
        return [
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head"><h3 className="card-title">选择安装包</h3><span className="spacer" /></div>
                <div className="card-body">
                  <button type="button" className="dropzone" disabled={batchBusy} onClick={() => void pickApksBatch()}>
                    把 APK 拖到这里，或选择 APK（可多选）
                    <small>每个安装包都在手机上确认，不做静默安装</small>
                  </button>
                  <p className="setting-note">安装包逐个经 adb 安装；失败原因来自 adb 的真实输出。单个 APK 也可以直接拖进窗口安装。</p>
                </div>
              </section>
            ),
          },
          {
            kind: "table",
            title: batchApks.length > 0 ? `安装包 · ${batchApks.length}` : "安装队列",
            tone: 4,
            columns: ["文件", "状态", "结果"],
            rows: batchApks.map((item) => ({
              cells: [
                item.name,
                item.status === "installing" ? <span className="badge warn">安装中…</span> : item.status === "done" ? <span className="badge ok">已安装</span> : item.status === "failed" ? <span className="badge danger">失败</span> : <span className="badge neutral">待安装</span>,
                item.message ?? "—",
              ],
              filterText: item.name,
            })),
            empty: "还没有选择安装包；点上方拖放区一次挑多个。",
          },
          {
            kind: "actions",
            actions: [
              { label: "全部安装", primary: true, onClick: () => readyDevice && void installBatch(readyDevice.serial), disabled: batchBusy || batchApks.length === 0 || !readyDevice },
              { label: "停止", onClick: () => { batchCancelRef.current = true; }, disabled: !batchBusy, danger: true },
              { label: "清空队列", onClick: () => setBatchApks([]), disabled: batchBusy },
            ],
          },
          { kind: "note", text: apkError ?? apkMessage ?? `目标设备：${currentDeviceLabel ?? "未选择设备"} · 进度：${batchBusy ? "正在逐个安装…" : "空闲"}` },
        ];
      case "apps": {
        const appQuery = viewFilter.trim().toLowerCase();
        const filteredApps = appQuery
          ? visibleDeviceApps.filter((app) => `${app.name} ${app.package}`.toLowerCase().includes(appQuery))
          : visibleDeviceApps;
        return [
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head">
                  <h3 className="card-title">已安装应用</h3>
                  <span className="badge neutral">{filteredApps.length} 项</span>
                  <span className="spacer" />
                  <label className="search-box">
                    <span aria-hidden="true">⌕</span>
                    <input
                      type="search"
                      value={viewFilter}
                      onChange={(event) => setViewFilter(event.target.value)}
                      placeholder="搜索应用名或包名"
                      aria-label="搜索应用名或包名"
                    />
                  </label>
                  <select className="select" value={appsSort} onChange={(event) => setAppsSort(event.target.value as "name" | "package")} aria-label="应用排序">
                    <option value="name">按名称</option>
                    <option value="package">按包名</option>
                  </select>
                </div>
                {filteredApps.length > 0 ? (
                  <div className="table-wrap" role="region" aria-label="已安装应用" tabIndex={0}>
                    <table className="table">
                      <thead>
                        <tr><th>应用</th><th>包名</th><th data-col="ops">操作</th></tr>
                      </thead>
                      <tbody>
                        {filteredApps.map((app) => (
                          <tr key={app.package}>
                            <td className="cell-main">{app.name || app.package}</td>
                            <td className="mono">{app.package}</td>
                            <td data-col="ops">
                              <span className="detail-ops">
                                {/* X10-134：启动走系统默认入口；卸载先弹显式确认（不可恢复）。 */}
                                <button
                                  type="button"
                                  className="btn btn-primary btn-sm"
                                  disabled={!readySerial || appActionBusy !== null}
                                  onClick={() => void launchDeviceApp(app.package)}
                                >
                                  打开
                                </button>
                                <button
                                  type="button"
                                  className="btn btn-danger btn-sm"
                                  disabled={!readySerial || appActionBusy !== null}
                                  onClick={() => void uninstallDeviceApp(app.package, app.name)}
                                >
                                  卸载
                                </button>
                              </span>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                ) : (
                  <div className="empty hint"><p>{appQuery ? `没有匹配「${viewFilter}」的应用` : "尚未读取应用清单；连接设备后点「刷新清单」。"}</p></div>
                )}
                <div className="card-foot">
                  <button type="button" className="btn btn-primary btn-sm" onClick={() => appsTargetSerial && void ensureDeviceApps(appsTargetSerial)} disabled={!appsTargetSerial}>刷新清单</button>
                  <button type="button" className="btn btn-secondary btn-sm" onClick={() => openView("install")}>安装 APK</button>
                  <span className="spacer" />
                  <span>应用数据不读取、不上传</span>
                </div>
              </section>
            ),
          },
          { kind: "note", text: appActionNotice ? `${appActionNotice.error ? "⚠" : "✓"} ${appActionNotice.text}` : "「打开」用系统默认入口启动应用；「卸载」会先弹确认、卸载后无法恢复，系统预装应用无法卸载。应用数据不读取、不上传。" },
        ];
      }
      case "notify":
        return [
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head">
                  <h3 className="card-title">实时通知</h3>
                  <span className="badge neutral">{phoneNotifications.length} 条</span>
                  <span className="spacer" />
                  {phoneNotifications.length > 0 && (
                    <button type="button" className="btn-ghost danger" onClick={() => void clearNotifications()}>清空</button>
                  )}
                </div>
                {phoneNotifications.length === 0 ? (
                  <div className="empty hint">
                    <p>伴侣 App 打开通知镜像并授权后，新通知会实时出现在这里。</p>
                  </div>
                ) : phoneNotifications.map((item) => {
                  const code = verificationCode(`${item.title} ${item.text}`);
                  return (
                  <div className="notify-row" key={item.key ?? `${item.pkg}-${item.posted}`}>
                    <span className="tool-ico" aria-hidden="true">{(item.app || item.pkg || "?").slice(0, 1)}</span>
                    <div className="notify-body">
                      <b>{item.app || item.pkg}</b>
                      <p className="notify-text">{item.title}{item.text ? `：${item.text}` : ""}</p>
                      <div className="notify-inline">
                        {code && (
                          <button type="button" className="secondary-button sm" onClick={() => void copyNotificationCode(code)}>
                            复制验证码
                          </button>
                        )}
                        {item.replyable && item.key && (
                          <details className="notify-reply-details">
                            <summary className="btn-ghost">快捷回复</summary>
                            <div className="notify-reply">
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
                          </details>
                        )}
                      </div>
                    </div>
                    <time>{notificationTime(item.posted)}</time>
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label="清除此通知并同步到手机"
                      title="清除并同步到手机"
                      disabled={!item.key}
                      onClick={() => item.key && void clearNotifications(item.key)}
                    >
                      ×
                    </button>
                    <button
                      type="button"
                      className="btn btn-primary btn-sm"
                      disabled={!readySerial || appActionBusy !== null}
                      onClick={() => void launchDeviceApp(item.pkg)}
                    >
                      {appActionBusy === item.pkg ? "打开中…" : "打开"}
                    </button>
                  </div>
                  );
                })}
                {notifyNotice && <p className="hint" role="status" style={{ padding: "8px 20px" }}>{notifyNotice}</p>}
                <div className="card-foot">
                  <button
                    type="button"
                    className="switch"
                    role="switch"
                    aria-checked={notifyKeep}
                    aria-label="本机保留"
                    onClick={toggleNotifyKeep}
                  />
                  <span>本机保留（最多 50 条，重启后恢复）</span>
                </div>
              </section>
            ),
          },
          { kind: "note", text: notifyKeep ? "通知已保留在本机（最多 50 条），重启应用后恢复；关闭开关即清除已存记录。内容不出本机。「复制验证码」只写入本机剪贴板；「清除」会同步移除手机通知，需手机常驻连接在线。" : "通知正文只在本窗口临时展示，默认不落盘、不上传；「复制验证码」只写入本机剪贴板；「清除」会同步移除手机通知，需手机常驻连接在线，离线时不会清本地。" },
        ];
      case "pro":
        return [
          { kind: "kv", title: "版本与权益", tone: 3, items: [["当前版本", editionLabel(entitlement?.edition)], ["本机校验", "不需要账号"], ["核心功能", "镜像与文件传输保持免费"], ["录像", proEdition ? "已激活" : "专业版功能"]] },
          {
            kind: "custom",
            children: (
              <section className="card">
                <div className="card-head">
                  <h3 className="card-title">激活</h3>
                  <span className={`badge ${proEdition ? "ok" : "neutral"}`}>{proEdition ? "已激活 · Pro" : "免费版 · 功能正常"}</span>
                  <span className="spacer" />
                </div>
                <div className="card-body">
                  <div className="detail-row">
                    <input className="input mono grow" value={licenseInput} onChange={(event) => setLicenseInput(event.target.value)} placeholder="MD1-XXXXXX-XXXXXX-…" aria-label="许可证密钥" />
                    <button type="button" className="btn btn-primary btn-sm" disabled={licenseBusy || !licenseInput.trim()} onClick={() => void activateLicense()}>{licenseBusy ? "激活中…" : "激活"}</button>
                    <button type="button" className="btn btn-secondary btn-sm" disabled={licenseBusy || !proEdition} onClick={() => void deactivateLicense()}>撤销本机授权</button>
                  </div>
                  {licenseMessage && <p className="hint" role="status">{licenseMessage}</p>}
                  {licenseError && <p className="field-error" role="alert">{licenseError}</p>}
                </div>
              </section>
            ),
          },
          { kind: "note", text: "激活仅在本机校验；不收集设备信息，也不需要联网账号。" },
        ];
      default:
        return [];
    }
  }

  // ⌘K 命令清单：19 个跳页 + 7 个真实动作（复用既有 invoke 函数，不新增假能力）。
  const cmdkCommands: { id: string; icon: ReactElement; label: string; hint: string; keywords: string; run: () => void }[] = [
    ...(Object.keys(ASSISTANT_VIEWS) as AssistantViewKey[]).map((key) => ({
      id: `view-${key}`,
      icon: assistantViewIcons[key],
      label: ASSISTANT_VIEWS[key].title,
      hint: "跳转",
      keywords: ASSISTANT_VIEWS[key].subtitle,
      run: () => openView(key),
    })),
    {
      id: "action-refresh",
      icon: assistantViewIcons.multi,
      label: "重新检查设备",
      hint: "动作",
      keywords: "刷新 adb devices 检查",
      run: () => void refreshDevices(),
    },
    {
      id: "action-shot",
      icon: assistantViewIcons.shot,
      label: "立即截图",
      hint: "动作",
      keywords: "截图 screenshot 保存",
      run: () => { if (readyDevice) void captureScreen(readyDevice.serial); else openView("connect"); },
    },
    {
      id: "action-record",
      icon: assistantViewIcons.record,
      label: recording?.active ? "停止录制" : "开始录制",
      hint: "动作",
      keywords: "录制 record 录屏",
      run: () => void toggleRecording(),
    },
    {
      id: "action-reveal",
      icon: assistantViewIcons.files,
      label: "打开截图文件夹",
      hint: "动作",
      keywords: "输出 文件夹 打开 图片",
      run: () => void revealCapture(),
    },
    {
      id: "action-diagnostic",
      icon: assistantViewIcons.info,
      label: "复制诊断摘要",
      hint: "动作",
      keywords: "诊断 复制 摘要 debug",
      run: () => void previewDiagnostics(),
    },
    {
      id: "action-collapse",
      icon: assistantViewIcons.settings,
      label: railCollapsed ? "展开侧栏" : "折叠侧栏",
      hint: "动作",
      keywords: "侧栏 折叠 展开 隐藏",
      run: () => setRailCollapsed((collapsed) => !collapsed),
    },
    {
      id: "action-theme",
      icon: assistantViewIcons.settings,
      label: "切换下一套主题",
      hint: "动作",
      keywords: "主题 外观 换肤",
      run: () => {
        const order = UI_THEMES.map((theme) => theme.key);
        const next = order[(order.indexOf(uiTheme) + 1) % order.length];
        setUiTheme(next);
        writeUiTheme(next);
      },
    },
  ];
  const cmdkQueryLower = cmdkQuery.trim().toLowerCase();
  const cmdkResults = cmdkQueryLower
    ? cmdkCommands.filter((command) => `${command.label} ${command.keywords}`.toLowerCase().includes(cmdkQueryLower))
    : cmdkCommands;

  return (
    <div className={`app-shell ${railCollapsed ? "rail-collapsed" : ""}`} data-theme={uiTheme}>
      {/* X10-127 外观层：顶部一级导航 + 工具栏 + 设备树 + 工作区 + 右侧状态栏。
          只是外壳与视觉；设备选择、页签、镜像按钮仍走原有状态机。 */}
      <header className="titlebar">
        {/* X10-153：标题栏按重构预览 1:1——brand-mark「MD」+ MirrorDock，无副标题；右侧为 pill + 图标按钮。 */}
        <div className="title-brand">
          <span className="brand-mark" aria-hidden="true">MD</span>
          <strong>MirrorDock</strong>
        </div>
        <button
          type="button"
          className="cmdk-trigger"
          aria-label="打开命令面板"
          onClick={() => { setCmdkOpen(true); setCmdkQuery(""); setCmdkIndex(0); }}
        >
          <span aria-hidden="true">⌕</span>
          搜索功能或执行命令
          <kbd aria-hidden="true">⌘K</kbd>
        </button>
        <div className="title-right">
          <span className="title-device-pill" title={topbarStatus}>
            <span className={`status-dot ${readyDevice ? "ready" : sessionActive ? "ready" : "idle"}`} aria-hidden="true" />
            <span>{currentDeviceLabel ?? topbarStatus}</span>
          </span>
          <div className="theme-menu">
            <button
              type="button"
              className="icon-btn"
              aria-haspopup="menu"
              aria-expanded={themeMenuOpen}
              aria-label="切换外观主题"
              title={`主题 · ${themeMeta(uiTheme).label}`}
              onClick={() => setThemeMenuOpen((open) => !open)}
            >
              <NavIcon d="M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10ZM12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
            </button>
            {themeMenuOpen && (
              <div className="theme-menu-pop" role="menu" aria-label="外观主题">
                {UI_THEMES.map((theme) => (
                  <button
                    key={theme.key}
                    type="button"
                    role="menuitemradio"
                    aria-checked={uiTheme === theme.key}
                    className="theme-menu-item"
                    onClick={() => {
                      setUiTheme(theme.key);
                      writeUiTheme(theme.key);
                      setThemeMenuOpen(false);
                    }}
                  >
                    <span className="theme-dot" aria-hidden="true" style={{ background: THEME_DOT[theme.key] }} />
                    {theme.label}
                  </button>
                ))}
              </div>
            )}
          </div>
          <button className="icon-btn" type="button" aria-label="打开帮助中心" title="帮助中心" onClick={() => openView("help")}>
            <NavIcon d="M12 17h.01M9.1 9a3 3 0 0 1 5.8 1c0 2-3 2.5-3 4M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z" />
          </button>
        </div>
      </header>

      <div className="workspace">
        <aside className="sidebar" aria-label="主导航">
          {/* X10-153：侧栏按重构预览 1:1 复刻——side-scroll 内为静态分组标题 + 常显导航项。
              移除设备树与可折叠分组（预览无此结构）；设备选择/动作已由「设备中心」主区
              的设备卡与 Hero 卡承载（selectedSerial 缺省回退首个就绪设备）。
              分组/顺序/图标与预览 NAV 一致（assistantViews.tsx ASSISTANT_GROUPS）。 */}
          <div className="side-scroll">
            {ASSISTANT_GROUPS.map((group) => (
              <nav aria-label={group.name} key={group.name}>
                <div className="nav-group-title">{group.name}</div>
                {group.ids.map((id) => (
                  <button
                    key={id}
                    type="button"
                    className={view === id ? "nav-item active" : "nav-item"}
                    aria-current={view === id ? "page" : undefined}
                    onClick={() => openView(id)}
                    title={ASSISTANT_VIEWS[id].title}
                  >
                    <span className="ic" aria-hidden="true">{assistantViewIcons[id]}</span>
                    <span className="nav-label">{ASSISTANT_VIEWS[id].title}</span>
                  </button>
                ))}
              </nav>
            ))}
          </div>
          <div className="side-foot">
            <button
              className="icon-btn"
              type="button"
              aria-label={railCollapsed ? "展开侧栏" : "折叠侧栏"}
              title="折叠侧栏 ⌘B"
              onClick={() => setRailCollapsed((collapsed) => !collapsed)}
            >
              <NavIcon d="M4 5h16v14H4zM9.5 5v14" />
            </button>
            <span className="hint-line">本地版 · 无需账号</span>
            <span className="spacer" />
          </div>
        </aside>

        <div className="main-area">

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

        <div className="assistant-work-head">
          <h1>{ASSISTANT_VIEWS[view].title}</h1>
          <span>{ASSISTANT_VIEWS[view].subtitle}</span>
          <div className="assistant-work-spacer" />
          {FILTERABLE_VIEWS.has(view) && (
            <label className="assistant-work-search">
              <span aria-hidden="true">⌕</span>
              <input
                type="search"
                aria-label="筛选本页内容"
                placeholder="筛选本页内容"
                value={viewFilter}
                onChange={(event) => setViewFilter(event.target.value)}
              />
            </label>
          )}
          <button className="btn btn-primary btn-sm" type="button" aria-label={`页面操作：${ASSISTANT_VIEWS[view].action}`} onClick={runViewAction}>{ASSISTANT_VIEWS[view].action}</button>
        </div>

        <div className="content">
          {isLegacyView ? (
          <>
          {/* -- 连接（主页） ------------------------------------------------ */}
          <section className={`tab-panel ${tab === "home" ? "" : "panel-hidden"}`} aria-label="连接">
            {/* X10-150：按预览模板重构设备中心。有就绪设备时显示主设备 Hero 卡
                （身份/系统/连接/电量/镜像会话，全部真实数据），无设备时保留原引导页。 */}
            {readyDevice ? (() => {
              const primary = readyDevice;
              const primaryLabel = displayLabels[primary.serial] ?? primary.label;
              const ownSession = sessionFor(primary.serial);
              const primaryOwned = ownSession?.phase === "connecting" || ownSession?.phase === "streaming";
              const primaryFavorite = favoriteDevices.includes(primary.serial);
              const primaryUsbAlt = primary.state === "ready" && !primaryOwned ? suggestedUsbSerial(primary) : null;
              const heroPower = powerSerial && powerSerial === readySerial ? powerStorage : null;
              const heroBattery = heroPower && heroPower.battery_level !== null
                ? `${heroPower.battery_level}%${heroPower.charging ? " · 充电中" : ""}`
                : null;
              const heroSub = capabilities?.android_release
                ? `Android ${capabilities.android_release}`
                : "系统版本可在「设备信息」页读取";
              const heroChannel = connectionLabel(primary);
              const prefBadge = preferredChannelLabel(primary);
              return (
                <div className="device-hero" aria-label="主设备概览">
                  <div className="hero-phone" aria-hidden="true">
                    <NavIcon d="M8 3.5h8a1 1 0 0 1 1 1v15a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1v-15a1 1 0 0 1 1-1ZM10.5 17h3" />
                    {heroBattery !== null && <small>{heroBattery}</small>}
                  </div>
                  <div className="hero-id">
                    <b>{primaryLabel}</b>
                    <span>{heroSub}</span>
                    <div className="hero-chips">
                      <span className="badge ok"><span className="dot ok" aria-hidden="true" />{heroChannel ? `${heroChannel} · 已授权` : "已授权"}</span>
                      {prefBadge && (
                        <span className="badge neutral" title="本次点「开始镜像」实际会走的通道（USB 更稳）">{prefBadge}</span>
                      )}
                      <span className="badge neutral">主设备</span>
                    </div>
                  </div>
                  <div className="hero-divider" aria-hidden="true" />
                  <div className="hero-media">
                    <span className="tool-ico c2" aria-hidden="true"><NavIcon d="M8 5l11 7-11 7z" /></span>
                    <div>
                      <b>{sessionActive
                        ? (activeSessionCount > 1 ? `镜像会话进行中 · ${activeSessionCount} 台` : "镜像会话进行中")
                        : "镜像会话未开始"}</b>
                      <small>画面在独立镜像窗口渲染 · 本机直连，不经过任何服务器</small>
                    </div>
                  </div>
                  <div className="hero-actions">
                    <button className="secondary-button" type="button" onClick={() => void refreshDevices()} disabled={isChecking}>
                      {isChecking ? "检查中…" : "重新检查"}
                    </button>
                    <span className="device-card-actions">{deviceActions(primary, primaryOwned, primaryFavorite, primaryUsbAlt)}</span>
                  </div>
                </div>
              );
            })() : null}

            <section className="connection-card" aria-live="polite">
              {/* X10-145：组件语言对齐预览——卡头用 card-head + badge，正文用 card-body，
                  状态提示用 alert。文案、状态分支与事件保持原样。 */}
              {/* X10-155：按预览 1:1 复刻——连接卡头不再因就绪设备而隐藏（预览 demo 在
                  Hero 卡下方仍显示「第一步：连接手机 / 手机已准备就绪」+ 重新检查）。
                  状态提示与设备卡列表（其余设备）保持原样。 */}
              <div className="card-head connection-heading">
                <div>
                  <span className="eyebrow badge neutral">
                    {sessionActive
                      ? activeSessionCount > 1
                        ? `镜像运行中：${activeSessionCount} 台设备`
                        : sessionDeviceLabel
                          ? `镜像运行中：${sessionDeviceLabel}`
                          : "镜像运行中"
                      : "第一步：连接手机"}
                  </span>
                  <h2 className="card-title">
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
                  会话仍在但设备已从列表消失（如无线瞬断）时，补一张兜底卡保留结束入口。
                  X10-150：主设备已由 Hero 卡承载（动作共用 deviceActions），列表只显示其余设备。 */}
              {!isChecking && check?.devices && check.devices.length > 0 && (
                <div className="device-cards" aria-label="已连接的设备">
                  {check.devices
                    .filter((device) => !(readyDevice && device.serial === readyDevice.serial))
                    .filter((device) => device.state === "ready" || !hiddenRevokedSerials.includes(device.serial))
                    .map((device) => {
                    const badge = connectionLabel(device);
                    const lockStamp = device.state === "ready" ? lockReports[device.serial] : undefined;
                    const own = sessionFor(device.serial);
                    const owned = own?.phase === "connecting" || own?.phase === "streaming";
                    const favorite = favoriteDevices.includes(device.serial);
                    // X10-111（C+B）：首选通道徽标 + 「可改用有线」提示（仅未在镜像时给切换入口）。
                    const prefBadge = device.state === "ready" ? preferredChannelLabel(device) : null;
                    const usbAlt = device.state === "ready" && !owned ? suggestedUsbSerial(device) : null;
                    return (
                      <div className={`device-card ${owned ? "device-card-active" : ""}`} key={device.serial}>
                        <span className={`status-dot ${owned ? "ready" : device.state}`} aria-hidden="true" />
                        <div>
                          <strong>
                            {favorite && <span className="fav-star" aria-hidden="true">★ </span>}
                            {displayLabels[device.serial] ?? device.label}
                            {badge && <span className="conn-badge inline">{badge}</span>}
                            {prefBadge && (
                              <span
                                className={`conn-badge inline pref-badge ${looksLikeWirelessEndpoint(device.serial) ? "wireless" : "usb"}`}
                                title="本次点「开始镜像」实际会走的通道（USB 更稳）"
                              >
                                {prefBadge}
                              </span>
                            )}
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
                          {deviceActions(device, owned, favorite, usbAlt)}
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

              {/* X10-154：对齐预览——「在电脑上安心使用手机」由卡片外大标题改为卡片内
                  引导（statusStack 后、三步前），与预览 card-body 内 <b>+hint 结构一致。 */}
              {!isChecking && !readyDevice && (
                <div className="conn-empty-lead">
                  <b>在电脑上安心使用手机</b>
                  <p className="hint">用数据线或同一 Wi-Fi 连接手机，画面只出现在这台电脑上。</p>
                </div>
              )}

              {(!isChecking && !readyDevice) && (
                <ol className="setup-steps steps">
                  <li className="step-line"><div><strong>使用可传输数据的数据线连接手机</strong><p>如果手机弹出 USB 用途选择，请选择“文件传输”。</p></div></li>
                  <li className="step-line"><div><strong>在手机上开启“USB 调试”</strong><p>这是 Android 提供的安全授权，用于将画面显示到这台电脑。</p></div></li>
                  <li className="step-line"><div><strong>解锁手机并允许这台电脑</strong><p>在“允许 USB 调试吗？”中选择允许。你可以随时在手机设置中撤销。</p></div></li>
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
                        className={`chip${shownGuide?.key === guide.key ? " active" : ""}`}
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
                      <ol className="setup-steps brand-steps steps">
                        <li className="step-line"><div><strong>打开开发者选项</strong><p>{shownGuide.openDeveloperOptions}</p></div></li>
                        <li className="step-line"><div><strong>开启 USB 调试</strong><p>{shownGuide.usbDebugging}</p></div></li>
                        <li className="step-line"><div><strong>（可选）无线调试</strong><p>{shownGuide.wireless}</p></div></li>
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

              {/* X10-150：按预览模板补齐设备中心下半区——常用工具六宫格（全部是真实
                  视图入口）、最近任务（仅本会话真实操作，不持久化）、安全边界说明。 */}
              {readyDevice && (
                <>
                  <section className="card" aria-label="常用工具">
                    <div className="card-head"><h3 className="card-title">常用工具</h3></div>
                    <div className="card-body">
                      <div className="tool-grid-home">
                        {HOME_TOOLS.map((tool) => (
                          <button key={tool.view} className="tool-card" type="button" onClick={() => openView(tool.view)}>
                            <span className={`tool-ico ${tool.tone}`} aria-hidden="true"><NavIcon d={tool.icon} /></span>
                            <span><b>{tool.title}</b><small>{tool.hint}</small></span>
                          </button>
                        ))}
                      </div>
                    </div>
                  </section>
                  <section className="card" aria-label="最近任务">
                    <div className="card-head">
                      <h3 className="card-title">最近任务</h3>
                      <span className="spacer" />
                      <span className="badge neutral">本次运行 · 不持久化</span>
                    </div>
                    {recentTasks.length === 0 ? (
                      <div className="card-body tight">
                        <p className="hint">本次运行还没有操作记录。截图、安装、录制完成后会显示在这里，只保留最近 8 条。</p>
                      </div>
                    ) : (
                      <div className="table-wrap" role="region" aria-label="最近任务列表" tabIndex={0}>
                        <table className="table">
                          <thead><tr><th>任务</th><th data-col="secondary">状态</th><th data-col="meta">时间</th></tr></thead>
                          <tbody>
                            {recentTasks.map((task) => (
                              <tr key={task.id}>
                                <td><b>{task.title}</b></td>
                                <td data-col="secondary">
                                  <span className={`badge ${task.status === "done" ? "ok" : task.status === "failed" ? "warn" : "neutral"}`}>
                                    {task.status === "done" ? "已完成" : task.status === "failed" ? "失败" : "进行中"}
                                  </span>
                                </td>
                                <td data-col="meta">{task.time}</td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    )}
                  </section>
                  <p className="assistant-note" role="note">安全边界：无需账号、数据不出本机；镜像、传输、卸载都需要设备显式授权。</p>
                </>
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
                  <button className="text-button" type="button" onClick={() => void clearNotifications()}>
                    清空（同步手机）
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
            {/* X10-148：页标题已在上方页头（连接与授权），这里只保留说明行，去掉重复大标题。 */}
            <div className="page-head">
              <p className="intro">同一 Wi-Fi 下连接 Android 11+；推荐扫码配对，全程免输入。</p>
            </div>
            <section className="connection-card" aria-live="polite">
              <div className="card-head wireless-heading">
                <div>
                  <span className="badge ok">推荐</span>
                  <h2 className="card-title">扫码配对</h2>
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
              <div className="card-head wireless-heading">
                <div>
                  <span className="badge neutral">手动方式</span>
                  <h2 id="wireless-title" className="card-title">配对码配对</h2>
                  <p>扫码不便时，填手机屏幕上的地址和 6 位配对码。配对码不会保存。</p>
                </div>
                <button className="secondary-button" type="button" onClick={() => setWirelessExpanded((value) => !value)}>
                  {wirelessExpanded ? "收起" : "手动配对"}
                </button>
              </div>

              {wirelessExpanded && <div className="wireless-content">
                <ol className="wireless-steps steps">
                  <li className="step-line">在手机的“开发者选项”中打开“无线调试”，并确认手机和电脑在同一 Wi-Fi。</li>
                  <li className="step-line">选择“使用配对码配对设备”，填写手机显示的配对地址和 6 位配对码。</li>
                  <li className="step-line">回到无线调试主页面，填写“IP 地址和端口”中的连接地址；它可能与配对地址不同。</li>
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
              {/* X10-154：对齐预览——「本机已保存的无线设备」常显（含空态），不再
                  只有存在已保存设备时才出现。 */}
              <div className="trusted-devices" aria-label="本机已保存的无线设备">
                <strong>本机已保存的无线设备</strong>
                {trustedDevices.length > 0 ? trustedDevices.map((device) => <div className="trusted-device" key={device.endpoint}>
                  <code>{device.endpoint}</code>
                  <span>
                    <button className="text-button" type="button" disabled={wirelessBusy} onClick={() => void reconnect(device.endpoint)}>重新连接</button>
                    <button className="text-button danger" type="button" disabled={wirelessBusy} onClick={() => void forgetDevice(device.endpoint)}>忘记</button>
                  </span>
                </div>) : (
                  <p className="capability-pending">完成一次配对后，设备端点会保存在这里。</p>
                )}
              </div>
            </section>

            <section className="connection-card" aria-live="polite">
              <div className="card-head wireless-heading">
                <div>
                  <span className="badge warn">伴侣 App（实验）</span>
                  <h2 className="card-title">扫码配对</h2>
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

            {/* X10-154：对齐预览——USB 连接在「设备中心」完成的收尾引导卡 + 跳转入口。 */}
            <section className="connection-card">
              <p className="capability-pending">USB 连接与授权在「设备中心」完成：插上数据线、在手机上允许 USB 调试后，设备卡会出现在设备中心。</p>
              <div className="action-row" style={{ marginTop: 8 }}>
                <button className="secondary-button" type="button" onClick={() => openView("home")}>打开设备中心</button>
              </div>
            </section>
          </section>

          {/* -- 设置 -------------------------------------------------------- */}
          <section className={`tab-panel ${tab === "settings" ? "" : "panel-hidden"}`} aria-label="设置">
            {/* X10-148：页标题已在上方页头（设置），这里只保留说明行，去掉重复大标题。 */}
            <div className="page-head">
              <p className="intro">外观主题、镜像窗口、快捷键与授权。全部设置项按分组保留，切换分组不会清除任何配置。</p>
            </div>

            {/* X10-144：左侧二级分组导航 + 右侧原设置卡。7 组对应原 7 块内容，
                卡片 DOM / 事件绑定一字未改，只由 CSS 控制显隐，零丢项。 */}
            <div className="settings-split">
              <nav className="settings-nav" aria-label="设置分组">
                {SETTINGS_GROUPS.map((group) => (
                  <button
                    key={group.key}
                    type="button"
                    className={`settings-nav-item${settingsGroup === group.key ? " active" : ""}`}
                    aria-current={settingsGroup === group.key ? "true" : undefined}
                    onClick={() => setSettingsGroup(group.key)}
                  >
                    <span>{group.label}</span>
                    <small>{group.note}</small>
                  </button>
                ))}
              </nav>
              <div className="settings-content">
            <section className={`settings-card theme-settings${settingsGroup === "appearance" ? "" : " panel-hidden"}`} aria-labelledby="theme-settings-title">
              <header className="settings-card-head">
                <div><h2 id="theme-settings-title">外观主题</h2><p>只改变这台电脑上的 MirrorDock 界面；连接、镜像、文件与授权完全不受影响。</p></div>
              </header>
              <div className="theme-options" role="group" aria-label="外观主题">
                {UI_THEMES.map((theme) => (
                  <button
                    key={theme.key}
                    type="button"
                    className={uiTheme === theme.key ? "theme-option active" : "theme-option"}
                    aria-pressed={uiTheme === theme.key}
                    onClick={() => chooseUiTheme(theme.key)}
                  >
                    <span className={`theme-swatch swatch-${theme.key}`} aria-hidden="true"><i /><i /><i /></span>
                    <strong>{theme.label}</strong>
                    <small>{theme.description}</small>
                  </button>
                ))}
              </div>
              <p className="setting-note">主题偏好只保存在本机浏览器存储中，不上传、不需要账号。当前：{themeMeta(uiTheme).label}。</p>
            </section>

            <section className={`settings-card${settingsGroup === "general" ? "" : " panel-hidden"}`}>
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

            <section className={`settings-card${settingsGroup === "mirror" ? "" : " panel-hidden"}`}>
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
            <section className={`settings-card${settingsGroup === "desktop" ? "" : " panel-hidden"}`}>
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

            <section className={`settings-card${settingsGroup === "shortcuts" ? "" : " panel-hidden"}`}>
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

            <section className={`settings-card${settingsGroup === "license" ? "" : " panel-hidden"}`}>
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
            <section className={`settings-card${settingsGroup === "about" ? "" : " panel-hidden"}`}>
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
              </div>
            </div>
          </section>

          {/* -- 帮助 -------------------------------------------------------- */}
          <section className={`tab-panel ${tab === "help" ? "" : "panel-hidden"}`} aria-label="帮助">
            {/* X10-148：页标题已在上方页头（帮助中心），这里只保留说明行，去掉重复大标题。 */}
            <div className="page-head">
              <p className="intro">常用问题都在下面的帮助主题里；联系支持时先在这里预览诊断内容，确认无误后再导出。</p>
            </div>

            {/* 内置帮助文档：随应用分发、离线可读。默认折叠只显示标题（X10-125）。 */}
            <div className="help-list">
              {helpArticles.map((article) => {
                const expanded = expandedHelpIds.has(article.id);
                return (
                  <div className={`card capability-panel help-article ${expanded ? "help-article-open" : ""}`} key={article.id} aria-live="polite">
                    <button
                      type="button"
                      className="card-head help-article-head"
                      aria-expanded={expanded}
                      onClick={() => setExpandedHelpIds((previous) => {
                        const next = new Set(previous);
                        if (next.has(article.id)) { next.delete(article.id); } else { next.add(article.id); }
                        return next;
                      })}
                    >
                      <span className="help-article-chevron" aria-hidden="true">{expanded ? "▾" : "▸"}</span>
                      <span className="help-article-title">{article.title}</span>
                    </button>
                    {expanded && article.paragraphs.map((paragraph, index) => (
                      <p className="capability-pending" key={index}>{paragraph}</p>
                    ))}
                  </div>
                );
              })}
            </div>

            <section className="card connection-card" aria-live="polite">
              <div className="card-head"><h3 className="card-title">诊断包</h3><span className="badge neutral">仅本机 · 不上传</span></div>
              <div className="card-body">
                <p className="hint">仅含版本、系统与最近操作结果（已抹去序列号、配对码与路径）。先预览，确认后才保存；不会自动上传任何内容。</p>
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
              </div>
            </section>
            <aside className="privacy-note">
              <strong>隐私承诺</strong>
              <p>本地优先：数据只经过你的数据线或局域网，不经过任何服务器。</p>
            </aside>
          </section>
          </>
          ) : (
            <AssistantViewPage view={view} sections={assistantSections(view)} filter={viewFilter} />
          )}
        </div>
        </div>

        {RAIL_VIEWS.has(view) && (
        <aside className="right-rail" aria-label="情境上下文">
          {/* X10-153：右栏按重构预览 1:1 复刻——情境右栏（ctx-block/ctx-note/ctx-dl），
              仅设备相关视图显示；数据全部接真实状态，无演示数据。
              原右栏的「设备状态/常用工具/APK 拖放/最近任务/安全边界」均已存在于主区
              （设备中心 Hero+设备卡、工具格、安装页拖放区、安全边界说明），故不重复。 */}
          {view === "connect" && (
            <>
              <section className="ctx-block">
                <h3>排查要点</h3>
                <p className="ctx-note">连接失败先看三点：数据线是否支持数据传输、USB 调试是否开启、手机上是否点过「始终允许」。</p>
              </section>
              <section className="ctx-block">
                <h3>替代方案</h3>
                <p className="ctx-note">插线无效时，可先走无线连接或配对码；两种都失败再考虑品牌排查指引。</p>
              </section>
            </>
          )}
          {view === "info" && (
            <section className="ctx-block">
              <h3>诊断快照</h3>
              {readyDevice ? (
                <p className="ctx-note">{displayLabels[readyDevice.serial] ?? readyDevice.label} · {connectionLabel(readyDevice) || "已授权"} · Android 设备</p>
              ) : (
                <p className="ctx-note">连接后，每次刷新会保存一条诊断快照。</p>
              )}
            </section>
          )}
          {view === "multi" && (
            <section className="ctx-block">
              <h3>会话概览</h3>
              {readyDevice || activeSessionCount > 0 ? (
                <dl className="ctx-dl">
                  <div><dt>在线设备</dt><dd>{check?.devices?.filter((device) => device.state === "ready").length ?? 0} 台</dd></div>
                  <div><dt>进行中会话</dt><dd>{activeSessionCount}</dd></div>
                </dl>
              ) : (
                <p className="ctx-note">未连接设备。</p>
              )}
            </section>
          )}
          {view === "mirror" && (
            <>
              <section className="ctx-block">
                <h3>通道与画质</h3>
                <dl className="ctx-dl">
                  <div><dt>通道</dt><dd>{readyDevice ? connectionLabel(readyDevice) || "已授权" : "—"}</dd></div>
                  <div><dt>画质</dt><dd>{options.quality === "smooth" ? "流畅" : options.quality === "sharp" ? "高清" : "均衡"}</dd></div>
                  <div><dt>桌面模式</dt><dd>{options.desktop_mode ? "开启" : "关闭"}</dd></div>
                </dl>
              </section>
              <section className="ctx-block">
                <h3>保护内容</h3>
                <p className="ctx-note">受 Android 安全标志保护的应用（如支付页）画面将显示黑屏，画布左下角有常驻角标。</p>
              </section>
            </>
          )}
          {view === "present" && (
            <section className="ctx-block">
              <h3>演示边界</h3>
              <p className="ctx-note">画面全屏投到外部显示器；激光笔 / 白板标注、演示文稿管理尚未实现，不伪装。</p>
            </section>
          )}
        </aside>
        )}
      </div>

        {/* ⌘K 命令面板：19 个跳页 + 7 个真实动作；无设备的动作跳连接页（如实降级）。 */}
        {cmdkOpen && (
          <div className="cmdk-mask" onClick={(event) => { if (event.target === event.currentTarget) setCmdkOpen(false); }}>
            <div className="cmdk" role="dialog" aria-modal="true" aria-label="命令面板">
              <input
                className="cmdk-input"
                type="text"
                autoFocus
                role="combobox"
                aria-expanded="true"
                aria-controls="cmdk-listbox"
                placeholder="输入功能名或动作…"
                value={cmdkQuery}
                onChange={(event) => { setCmdkQuery(event.target.value); setCmdkIndex(0); }}
                onKeyDown={(event) => {
                  if (event.key === "ArrowDown") { event.preventDefault(); setCmdkIndex((i) => Math.min(i + 1, cmdkResults.length - 1)); }
                  else if (event.key === "ArrowUp") { event.preventDefault(); setCmdkIndex((i) => Math.max(i - 1, 0)); }
                  else if (event.key === "Enter") { event.preventDefault(); cmdkResults[cmdkIndex]?.run(); setCmdkOpen(false); }
                  else if (event.key === "Escape") { event.preventDefault(); setCmdkOpen(false); }
                }}
              />
              <ul className="cmdk-list" id="cmdk-listbox" role="listbox" aria-label="命令列表">
                {cmdkResults.length === 0 && <li className="cmdk-empty">没有匹配「{cmdkQuery}」的命令</li>}
                {cmdkResults.map((command, index) => (
                  <li key={command.id} role="option" aria-selected={index === cmdkIndex}>
                    <button
                      type="button"
                      className="cmdk-item"
                      onMouseEnter={() => setCmdkIndex(index)}
                      onClick={() => { command.run(); setCmdkOpen(false); }}
                    >
                      <span className="cmdk-icon" aria-hidden="true">{command.icon}</span>
                      {command.label}
                      <small>{command.hint}</small>
                    </button>
                  </li>
                ))}
              </ul>
              <div className="cmdk-foot" aria-hidden="true">
                <span><kbd>↑↓</kbd> 选择</span>
                <span><kbd>回车</kbd> 执行</span>
                <span><kbd>Esc</kbd> 关闭</span>
                <span style={{ marginInlineStart: "auto" }}>本地处理 · 无遥测</span>
              </div>
            </div>
          </div>
        )}

        <footer className="statusbar">
          <span className={`status-dot ${readyDevice ? "ready" : sessionActive ? "ready" : "idle"}`} aria-hidden="true" />
          <span>{currentDeviceLabel ?? topbarStatus}</span>
          <span className="statusbar-sep" aria-hidden="true">|</span>
          <span>scrcpy 4.1 · 固定哈希</span>
          <span className="statusbar-spacer" />
          <span>本地连接 · 无云中继</span>
          <span className="statusbar-sep" aria-hidden="true">|</span>
          <span>版本 {appVersion || "—"}</span>
        </footer>
    </div>
  );
}

export default App;
