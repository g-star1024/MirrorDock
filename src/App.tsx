import { useEffect, useRef, useState, type ReactElement } from "react";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
// 系统文件选择器由官方 dialog 插件提供；MirrorDock 自身不枚举、不猜测用户文件。
import { open as openFilePicker, save as saveFilePicker } from "@tauri-apps/plugin-dialog";
// 会话中的系统级快捷键：镜像窗口（scrcpy 窗口）持有焦点时主窗口收不到键盘事件，
// 只有全局快捷键能不切回主窗口就触发截图/录制/旋转。
import { register, unregisterAll } from "@tauri-apps/plugin-global-shortcut";
import { brandGuides, detectBrand, type BrandGuide } from "./brandGuides";
import { helpArticles } from "./helpContent";
import QRCode from "qrcode";
import "./App.css";

type DeviceState = "ready" | "unauthorized" | "offline" | "unknown";

type Device = {
  serial: string;
  label: string;
  state: DeviceState;
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
// 无线设备的标识形如 ip:port；USB 序列号不含冒号。仅用于决定给出哪种恢复动作。
function looksLikeWirelessEndpoint(serial: string) {
  return serial.includes(":");
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
type SessionOptions = { quality: "smooth" | "balanced" | "sharp"; fullscreen: boolean; always_on_top: boolean; rotation: number; keep_awake: boolean; record: boolean; clipboard_autosync: boolean; audio: boolean; shortcut_mod: string | null; show_touches: boolean; read_only: boolean };
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
  events: string[];
  offer: PairingOffer | null;
};

// 开发者中心自动发现（mDNS）：手机无线调试页广播的服务。
type WirelessServices = { pairing: string[]; connect: string[] };

// 应用级设置（后端持久化到 app-settings.json）：只影响客户端自身行为
// （窗口、图标），与镜像会话参数（SessionOptions）严格分开。
export type AppSettingsView = { hide_dock_icon: boolean };
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

/// 二维码载荷格式：MDP1|主机列表(逗号分隔)|端口|一次性配对码|SPKI SHA-256。
/// 伴侣 App 与桌面侧共享同一约定（见 companion_pairing.rs 模块注释）。
export function pairingPayload(offer: Pick<PairingOffer, "hosts" | "port" | "token" | "fingerprint">): string {
  return `MDP1|${offer.hosts.join(",")}|${offer.port}|${offer.token}|${offer.fingerprint}`;
}
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
// 安装 APK 的回执：summary 是后端把 adb 结论解析后的可读结果。
type ApkInstallReceipt = { file_name: string; bytes: number; summary: string };
const defaultOptions: SessionOptions = { quality: "balanced", fullscreen: false, always_on_top: false, rotation: 0, keep_awake: true, record: false, clipboard_autosync: true, audio: true, shortcut_mod: null, show_touches: false, read_only: false };
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
        read_only: typeof value.read_only === "boolean" ? value.read_only : false,
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

/** 面板里只留一句「现在该做什么」；完整说明在页面底部的「关于锁屏与解锁」。 */
export function lockActionHint(report: DeviceLockReport): string {
  if (report.screen === "asleep") {
    return "屏幕没亮：点「屏幕唤醒」即可点亮；解锁需要你本人在手机上输入。";
  }
  if (report.keyguard === "locked") {
    return "请在手机上解锁；解锁凭据只会输入在手机或镜像窗口里，MirrorDock 不记录。";
  }
  if (report.keyguard === "unlocked") {
    return "手机已解锁，可以直接开始镜像。";
  }
  return "锁屏状态读取不完整，不影响开始镜像；遇到异常请解锁手机后重试。";
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
  const [capabilitiesError, setCapabilitiesError] = useState<string | null>(null);
  const [lockReport, setLockReport] = useState<DeviceLockReport | null>(null);
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
  const [pairingQr, setPairingQr] = useState<string | null>(null);
  const [pairingBusy, setPairingBusy] = useState(false);
  const [pairingError, setPairingError] = useState<string | null>(null);
  const [session, setSession] = useState<MirrorSession | null>(null);
  const [sessionError, setSessionError] = useState<string | null>(null);
  const [applyingOptions, setApplyingOptions] = useState(false);
  const [applyNotice, setApplyNotice] = useState<string | null>(null);
  const sessionActive = session?.phase === "connecting" || session?.phase === "streaming";
  // 授权状态读取失败（null）按免费版呈现：录制开关禁用并给出激活指引。
  const proEdition = isProEdition(entitlement?.edition);
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const current = await invoke<MirrorSession>("mirror_session");
        if (!disposed) { setSession(current); setSessionError(null); }
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
  // 通用设置：开机自启（autostart 插件持久化）与 macOS 隐藏 Dock（后端持久化）。
  const [appSettings, setAppSettings] = useState<AppSettingsView>({ hide_dock_icon: false });
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
        if (!disposed) setAppSettings({ hide_dock_icon: settings.hide_dock_icon === true });
      } catch { if (!disposed) setAppSettings({ hide_dock_icon: false }); }
      try {
        const enabled = await invoke<unknown>("plugin:autostart|is_enabled");
        if (!disposed) setAutostartEnabled(enabled === true);
      } catch { /* 非 Tauri 环境（浏览器/测试）没有该命令，保持关闭 */ }
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
    setAppSettings({ hide_dock_icon: hide });
    setGeneralNotice(null);
    try {
      const saved = await invoke<Partial<AppSettingsView>>("set_app_settings", { settings: { hide_dock_icon: hide } });
      setAppSettings({ hide_dock_icon: saved.hide_dock_icon === true });
      setGeneralNotice(hide ? "已隐藏 Dock 图标，从屏幕顶部菜单栏图标使用 MirrorDock。" : "已恢复 Dock 图标。");
    } catch (error) {
      setAppSettings(previous);
      setGeneralNotice(errorMessage(error, "无法修改 Dock 图标设置。"));
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
      });
      setSession(result.session);
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
    }
  }

  async function stopMirroring() {
    setIsStopping(true);
    setLaunchError(null);
    try {
      await invoke("stop_mirroring");
      // 会话已结束，上一次「新设置已生效」的提示不再有意义。
      setApplyNotice(null);
    } catch (error) {
      setLaunchError(errorMessage(error, "无法结束镜像会话，请手动关闭镜像窗口。"));
    } finally {
      setIsStopping(false);
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
      setLockReport(await invoke<DeviceLockReport>("device_lock_report", { serial }));
      setLockError(null);
    } catch (error) {
      setLockError(errorMessage(error, "无法读取手机当前的锁屏状态。"));
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
      setPairingStatus({ phase: "listening", events: [], offer });
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

  useEffect(() => {
    if (!pairingOffer) return;
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
  }, [pairingOffer]);

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
    } catch (error) {
      setTransferError(errorMessage(error, "文件没有从手机取回。"));
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
  const scrcpyReady = check?.scrcpy_available ?? false;
  const readySerial = readyDevice?.serial ?? null;
  // 快捷键处理器读取的最新 serial。
  shortcutsRef.current.readySerial = readySerial;
  // 选中设备变化时重新探测能力与锁屏状态。两者都只读取设备信息，不启动镜像。
  useEffect(() => {
    if (!readySerial) {
      setCapabilities(null);
      setCapabilitiesError(null);
      setLockReport(null);
      setLockError(null);
      return;
    }
    let disposed = false;
    setCapabilities(null);
    setCapabilitiesError(null);
    setLockReport(null);
    setLockError(null);
    invoke<DeviceCapabilities>("probe_device_capabilities", { serial: readySerial })
      .then((value) => { if (!disposed) setCapabilities(value); })
      .catch((error) => {
        if (!disposed) setCapabilitiesError(errorMessage(error, "无法读取这台手机的能力信息。"));
      });
    invoke<DeviceLockReport>("device_lock_report", { serial: readySerial })
      .then((value) => { if (!disposed) setLockReport(value); })
      .catch((error) => {
        if (!disposed) setLockError(errorMessage(error, "无法读取手机当前的锁屏状态。"));
      });
    // 锁屏状态会随使用变化（点亮/解锁/熄屏），面板不能停留在旧状态：
    // 每 5 秒轻量复查一次；「屏幕唤醒」成功后另有立即刷新。复查失败保持上一次结果。
    const timer = window.setInterval(() => {
      invoke<DeviceLockReport>("device_lock_report", { serial: readySerial })
        .then((value) => { if (!disposed) setLockReport(value); })
        .catch(() => { /* 保持上一次结果，不刷错误打断面板 */ });
    }, 5000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [readySerial]);
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
  // 顶栏状态：一句话说清“现在这台电脑能不能用手机”。
  const topbarStatus = isChecking
    ? "正在检查连接…"
    : sessionActive
      ? "镜像运行中"
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
          {sessionActive && (
            <button className="secondary-button" type="button" onClick={() => void stopMirroring()} disabled={isStopping}>
              {isStopping ? "正在结束…" : "结束镜像"}
            </button>
          )}
        </header>

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
                  <p className="eyebrow">{sessionActive ? "镜像运行中" : "第一步：连接手机"}</p>
                  <h2>{isChecking ? "正在检查 USB 连接…" : readyDevice ? "手机已准备就绪" : "等待连接手机"}</h2>
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
                {settingsNotice && <p className="diagnostic">{settingsNotice}</p>}
              </div>

              {readyDevice ? (
                <>
                {readyDevices.length > 1 && <div className="device-picker" aria-label="选择要镜像的设备">
                  {readyDevices.map((device) => <button className={device.serial === readyDevice.serial ? "device-choice selected" : "device-choice"} type="button" key={device.serial} onClick={() => setSelectedSerial(device.serial)}>{device.label}</button>)}
                </div>}
                <div className="ready-panel">
                  <span className="status-dot ready" aria-hidden="true" />
                  <div>
                    <strong>{readyDevice.label}</strong>
                    <p>已授权，可以开始镜像。</p>
                  </div>
                  <button className="secondary-button ready-wake" type="button" disabled={lockBusy} onClick={() => void wakeDevice(readyDevice.serial)}>
                    {lockBusy ? "正在唤醒…" : "屏幕唤醒"}
                  </button>
                  <button className="primary-button" type="button" disabled={!scrcpyReady || isLaunching || sessionActive} onClick={() => void startMirroring(readyDevice.serial)}>
                    {sessionActive ? "会话进行中" : isLaunching ? "正在启动…" : scrcpyReady ? "开始镜像" : "镜像引擎准备中"}
                  </button>
                </div>
                <div className="panel-grid">
                  <div className="capability-panel" aria-live="polite">
                    <strong>手机当前的锁屏状态</strong>
                    {lockReport ? (
                      <>
                        <p className="capability-summary">{lockSummary(lockReport)}</p>
                        <p className="capability-pending">{lockActionHint(lockReport)}</p>
                      </>
                    ) : lockError ? (
                      <p className="capability-pending" role="alert">{lockError}</p>
                    ) : (
                      <p className="capability-pending">正在读取手机当前的锁屏状态…</p>
                    )}
                    {sessionActive && lockReport?.screen === "asleep" && (
                      <p className="capability-pending" role="status">屏幕已关闭：在镜像窗口上点右键即可直接点亮屏幕（scrcpy 内置手势），无需回到本窗口。</p>
                    )}
                  </div>
                </div>
                </>
              ) : (
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

              {!isChecking && check?.devices && check.devices.length > 0 && !readyDevice && (
                <div className="device-list">
                  {check.devices.map((device) => (
                    <div className="device-row" key={device.serial}>
                      <span className={`status-dot ${device.state}`} aria-hidden="true" />
                      <div><strong>{device.label}</strong><p>{stateCopy[device.state].detail}</p></div>
                      <span className="status-label">{stateCopy[device.state].label}</span>
                    </div>
                  ))}
                </div>
              )}

              {/* 列表清空后仍要显示反馈，否则移除最后一条记录会静默消失，用户不知道操作是否生效。 */}
              {(recentDevices.length > 0 || recentMessage) && (
                <div className="recent-devices" aria-label="最近使用过的设备">
                  {recentDevices.length > 0 && <strong>最近使用过的设备</strong>}
                  {recentDevices.map((device) => {
                    const connected = check?.devices.find((item) => item.serial === device.serial);
                    return (
                      <div className="recent-device" key={device.serial}>
                        <div className="recent-device-info">
                          <strong>{device.label}</strong>
                          <p>
                            {relativeTime(device.last_used_at)} · {connected ? stateCopy[connected.state].label : "当前未连接"}
                          </p>
                        </div>
                        <span>
                          {connected?.state === "ready" ? (
                            <button className="text-button" type="button" disabled={isLaunching || sessionActive} onClick={() => void startMirroring(device.serial)}>开始镜像</button>
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

            <aside className="privacy-note">
              <strong>为什么需要授权？</strong>
              <p>画面与控制经由 Android 官方调试机制，只授予你确认过的电脑，可随时在手机开发者选项中撤销。</p>
            </aside>

            {/* 能力说明属于「了解性内容」：沉到底部授权说明之下，占满整幅，不抢占操作动线。 */}
            {readyDevice && (
              <div className="capability-panel capability-foot" aria-live="polite">
                <strong>这台手机能做什么</strong>
                {capabilities ? (
                  <>
                    <p className="capability-summary">{capabilitySummary(capabilities)}</p>
                    <ul className="capability-notices">
                      {capabilities.notices.map((notice) => (
                        <li key={notice.code} className={`notice-${notice.level}`}>
                          <strong>{notice.title}</strong>
                          <p>{notice.detail}</p>
                        </li>
                      ))}
                    </ul>
                  </>
                ) : capabilitiesError ? (
                  <p className="capability-pending" role="alert">{capabilitiesError}</p>
                ) : (
                  <p className="capability-pending">正在读取这台手机的信息…</p>
                )}
              </div>
            )}

            {/* 锁屏长说明沉底：面板里只留状态与一句行动提示，想细看再到下面看。 */}
            {readyDevice && lockReport && (
              <div className="capability-panel" aria-live="polite">
                <strong>关于锁屏与解锁</strong>
                <p className="capability-pending">{lockReport.explanation}</p>
                <p className="capability-pending">{lockReport.recovery}</p>
                <p className="capability-pending">MirrorDock 只点亮屏幕，不解锁；设备处于安全锁屏时，需要你本人在手机或镜像窗口中输入解锁凭据。</p>
              </div>
            )}
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
                  <p>扫码不便时，用伴侣 App 扫码或手动填写地址和 6 位配对码（一次有效，不保存）。</p>
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
              {pairingOffer && (
                <div className="screenshot-result">
                  <p className="capability-summary">
                    配对状态：{pairingStatus?.phase === "connected" ? "伴侣已连接" : pairingStatus?.phase === "listening" ? "等待伴侣扫码" : "未开始"}
                    {" · 一次性配对码 "}
                    <code>{pairingOffer.token}</code>
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
                <div><h2>通用</h2><p>窗口关闭行为、菜单栏/托盘与开机启动。</p></div>
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
                    <span className="setting-desc">镜像进行中手机不会自动熄屏，锁屏页有充足时间输入解锁密码。USB 与无线均生效；无线连接通过临时延长手机的熄屏时间实现，会话结束后自动恢复原设置。</span>
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
