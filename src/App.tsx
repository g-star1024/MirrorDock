import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
// 系统文件选择器由官方 dialog 插件提供；MirrorDock 自身不枚举、不猜测用户文件。
import { open as openFilePicker } from "@tauri-apps/plugin-dialog";
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
function relativeTime(seconds: number) {
  if (!seconds) return "使用时间未知";
  const diff = Date.now() / 1000 - seconds;
  if (diff < 60) return "刚刚使用";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前使用`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前使用`;
  const days = Math.floor(diff / 86400);
  return days === 1 ? "昨天使用" : `${days} 天前使用`;
}
type AppError = { code: string; message: string; recovery: string };
type SessionPhase = "idle" | "unauthorized" | "offline" | "paired" | "connecting" | "streaming" | "failed";
// 进程正在运行不等于首帧已到达；未接入端到端探针前后端只会返回 unknown。
type FirstFrame = "unknown" | "reached";
type MirrorSession = { phase: SessionPhase; serial: string | null; first_frame: FirstFrame; error: AppError | null };
// 能力探测的结论用 null 表示"未知"，不得默认成"支持"或"不支持"。
type NoticeLevel = "info" | "limitation";
type CapabilityNotice = { code: string; level: NoticeLevel; title: string; detail: string };
type DeviceCapabilities = {
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
type DeviceLockReport = {
  keyguard: KeyguardState;
  secure_lock: boolean | null;
  screen: ScreenState;
  explanation: string;
  recovery: string;
};
type SessionOptions = { quality: "smooth" | "balanced" | "sharp"; fullscreen: boolean; always_on_top: boolean; rotation: number; keep_awake: boolean; record: boolean; clipboard_autosync: boolean };
// 镜像窗口形态由启动参数决定，运行中无法改写：后端「应用新设置」= 结束旧窗口 + 按新设置重开。
type SessionUpdate = { applied: boolean; note: string | null; session: MirrorSession };
// 最近一次会话的录制文件。active 表示此刻进程是否仍在写这个文件。
type Recording = { file_name: string; path: string; active: boolean };
// 截图结果。后端只回文件名、路径与字节数，不回传任何像素数据。
type Screenshot = { file_name: string; path: string; bytes: number };
// 与截图共用同一回执形状：发送时 path 是手机上的路径，取回时是本机路径。
type TransferReceipt = { file_name: string; path: string; bytes: number };
const defaultOptions: SessionOptions = { quality: "balanced", fullscreen: false, always_on_top: false, rotation: 0, keep_awake: true, record: false, clipboard_autosync: true };
function readOptions(): SessionOptions {
  try {
    const value = JSON.parse(localStorage.getItem("mirrordock.sessionOptions") ?? "null");
    if (value && ["smooth", "balanced", "sharp"].includes(value.quality) && [0,90,180,270].includes(value.rotation) && typeof value.fullscreen === "boolean" && typeof value.always_on_top === "boolean") {
      // 旧版本没有 keep_awake / record / clipboard_autosync 字段：前两者按各自默认值
      // 取向回填（唤醒开、录制关），剪贴板同步沿用 scrcpy 默认（开）。
      return {
        quality: value.quality,
        rotation: value.rotation,
        fullscreen: value.fullscreen,
        always_on_top: value.always_on_top,
        keep_awake: typeof value.keep_awake === "boolean" ? value.keep_awake : true,
        record: typeof value.record === "boolean" ? value.record : false,
        clipboard_autosync: typeof value.clipboard_autosync === "boolean" ? value.clipboard_autosync : true,
      };
    }
  } catch { /* Invalid or unavailable local settings use defaults. */ }
  return defaultOptions;
}

// 截图与录像的文件名都按**本机时间**生成（后端不猜时区），随后由后端按 ASCII 白名单
// 严格校验。只使用数字与短横线，任何路径分隔符都不会出现在这里。
function localTimestamp(now: Date) {
  const pad = (value: number) => String(value).padStart(2, "0");
  const date = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
  return `${date}-${time}`;
}

function screenshotFileName(now: Date) {
  return `MirrorDock-${localTimestamp(now)}.png`;
}

function recordingFileName(now: Date) {
  return `MirrorDock-${localTimestamp(now)}.mp4`;
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} 字节`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function lockSummary(report: DeviceLockReport) {
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
function errorMessage(error: unknown, fallback: string) {
  if (typeof error === "object" && error !== null && "message" in error && "recovery" in error) {
    const detail = error as AppError;
    return `${detail.message} ${detail.recovery}`;
  }
  return typeof error === "string" ? error : fallback;
}

function sessionErrorText(session: MirrorSession, fallback: string) {
  return session.error ? `${session.error.message} ${session.error.recovery}` : fallback;
}

function supportText(value: boolean | null, yes: string, no: string, unknown: string) {
  return value === true ? yes : value === false ? no : unknown;
}

function capabilitySummary(capabilities: DeviceCapabilities) {
  const system = capabilities.android_release
    ? `Android ${capabilities.android_release}`
    : "系统版本未知";
  return [
    capabilities.label,
    system,
    supportText(capabilities.mirroring_supported, "可以镜像", "可能无法镜像", "镜像支持情况未知"),
    supportText(capabilities.audio_forwarding_supported, "可转发声音", "不能转发声音", "声音能力未知"),
  ].join(" · ");
}

function sessionStatus(session: MirrorSession): string | null {
  switch (session.phase) {
    case "idle":
      return null;
    case "connecting":
      return "正在启动镜像窗口…";
    case "streaming":
      return session.first_frame === "reached"
        ? "镜像正在运行。关闭镜像窗口即可结束本次会话。"
        : "镜像进程已启动，但尚未确认首帧到达。请查看手机画面是否已经出现。";
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

function App() {
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
  const [deviceFiles, setDeviceFiles] = useState<string[] | null>(null);
  const [session, setSession] = useState<MirrorSession | null>(null);
  const [sessionError, setSessionError] = useState<string | null>(null);
  const [applyingOptions, setApplyingOptions] = useState(false);
  const [applyNotice, setApplyNotice] = useState<string | null>(null);
  const sessionActive = session?.phase === "connecting" || session?.phase === "streaming";
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
  function updateOptions(next: SessionOptions) {
    setOptions(next);
    // 设置一旦被改动，上一次「已应用 / 无需重启」的结论就不再适用，先清掉避免误导。
    setApplyNotice(null);
    try { localStorage.setItem("mirrordock.sessionOptions", JSON.stringify(next)); }
    catch { setSettingsNotice("本机设置无法保存，本次会话仍可使用这些选项。"); }
  }

  // 会话进行中应用新设置：镜像窗口会按新参数重新打开，画面会短暂中断。
  async function applySessionOptions() {
    setApplyingOptions(true);
    setApplyNotice(null);
    try {
      const result = await invoke<SessionUpdate>("update_session_options", {
        options,
        recordFileName: options.record ? recordingFileName(new Date()) : null,
      });
      setSession(result.session);
      setApplyNotice(result.applied ? "新设置已生效：镜像窗口已按新设置重新打开。" : result.note ?? "设置与当前会话一致，未重启镜像窗口。");
    } catch (error) {
      setApplyNotice(errorMessage(error, "无法应用新设置。"));
    } finally {
      setApplyingOptions(false);
    }
  }

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
  }, []);

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
      setRecentMessage("已发送连接请求，正在更新设备列表。如果手机重新开启过无线调试，端口可能已经变化，需要重新配对。");
      await refreshDevices();
    } catch (error) {
      setRecentMessage(errorMessage(error, "这条记录无法直接连接，请在下方“无线调试”区域重新配对。"));
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
  // 选中设备变化时重新探测能力。探测只读取设备信息，不启动镜像。
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
    return () => { disposed = true; };
  }, [readySerial]);
  const statusMessage = session ? sessionStatus(session) : null;
  const statusRole = session && ["unauthorized", "offline", "failed"].includes(session.phase) ? "alert" : "status";

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand" aria-label="MirrorDock">
          <span className="brand-mark" aria-hidden="true">M</span>
          <span>MirrorDock</span>
        </div>
        <span className="local-pill">仅在本机连接</span>
      </header>

      <section className="hero" aria-labelledby="page-title">
        <p className="eyebrow">连接你的 Android 手机</p>
        <h1 id="page-title">在电脑上安心使用手机</h1>
        <p className="intro">使用数据线连接后，MirrorDock 会引导你完成一次安全授权。你的屏幕内容不会上传到云端。</p>
      </section>

      <section className="connection-card" aria-live="polite">
        <div className="connection-heading">
          <div>
            <p className="eyebrow">第一步：连接手机</p>
            <h2>{isChecking ? "正在检查 USB 连接…" : readyDevice ? "手机已准备就绪" : "等待连接手机"}</h2>
          </div>
          <button className="secondary-button" type="button" onClick={() => void refreshDevices()} disabled={isChecking}>
            {isChecking ? "检查中…" : "重新检查"}
          </button>
        </div>

        {check?.diagnostic && <p className="diagnostic">{check.diagnostic}</p>}
        {launchError && <p className="diagnostic">{launchError}</p>}
        {sessionError && <p className="diagnostic" role="alert">{sessionError}</p>}
        {statusMessage && <p className="diagnostic" role={statusRole}>{statusMessage}</p>}
        {session?.phase === "streaming" && (
          <button className="secondary-button" type="button" onClick={() => void stopMirroring()} disabled={isStopping}>
            {isStopping ? "正在结束…" : "结束镜像"}
          </button>
        )}
        {settingsNotice && <p className="diagnostic">{settingsNotice}</p>}
        <fieldset className="session-options">
          <legend>镜像窗口设置{sessionActive ? "（会话中修改需重启镜像窗口）" : "（开始镜像时生效）"}</legend>
          <label>画质 <select value={options.quality} onChange={e => updateOptions({...options, quality: e.target.value as SessionOptions["quality"]})}>
            <option value="smooth">流畅 · 1024 / 2 Mbps</option><option value="balanced">均衡 · 1920 / 8 Mbps</option><option value="sharp">清晰 · 2560 / 16 Mbps</option>
          </select></label>
          <label>显示旋转 <select value={options.rotation} onChange={e => updateOptions({...options, rotation: Number(e.target.value)})}>
            {[0,90,180,270].map(value => <option key={value} value={value}>{value}°</option>)}
          </select></label>
          <label><input type="checkbox" checked={options.fullscreen} onChange={e => updateOptions({...options, fullscreen: e.target.checked})}/> 全屏启动</label>
          <label><input type="checkbox" checked={options.always_on_top} onChange={e => updateOptions({...options, always_on_top: e.target.checked})}/> 窗口置顶</label>
          <label><input type="checkbox" checked={options.keep_awake} onChange={e => updateOptions({...options, keep_awake: e.target.checked})}/> 会话期间保持唤醒（建议开启，避免镜像中手机自动锁屏）</label>
          <label><input type="checkbox" checked={options.record} onChange={e => updateOptions({...options, record: e.target.checked})}/> 录制这一会话的画面（MP4，保存在本机）</label>
          <label><input type="checkbox" checked={options.clipboard_autosync} onChange={e => updateOptions({...options, clipboard_autosync: e.target.checked})}/> 双向同步剪贴板（关闭后手机与电脑的复制内容不再自动互通）</label>
          {sessionActive && (
            <button type="button" className="secondary-button" disabled={applyingOptions} onClick={() => void applySessionOptions()}>
              {applyingOptions ? "正在应用…" : "应用并重启镜像窗口"}
            </button>
          )}
          <button type="button" className="secondary-button" onClick={() => updateOptions(defaultOptions)}>恢复默认设置</button>
          {sessionActive && <p>这些设置由镜像窗口在启动时确定，无法在运行中热更新。点击“应用并重启镜像窗口”后，画面会短暂中断并自动恢复。</p>}
          <p>无线卡顿时可选择“流畅”。受保护内容可能显示黑屏；旋转只改变电脑上的显示方向。</p>
          {applyNotice && <p className="apply-notice" role="status">{applyNotice}</p>}
        </fieldset>

        {readyDevice ? (
          <>
          {readyDevices.length > 1 && <div className="device-picker" aria-label="选择要镜像的设备">
            {readyDevices.map((device) => <button className={device.serial === readyDevice.serial ? "device-choice selected" : "device-choice"} type="button" key={device.serial} onClick={() => setSelectedSerial(device.serial)}>{device.label}</button>)}
          </div>}
          <div className="ready-panel">
            <span className="status-dot ready" aria-hidden="true" />
            <div>
              <strong>{readyDevice.label}</strong>
              <p>已获得 USB 调试授权。下一步将开启镜像窗口。</p>
            </div>
            <button className="primary-button" type="button" disabled={!scrcpyReady || isLaunching || sessionActive} onClick={() => void startMirroring(readyDevice.serial)}>
              {sessionActive ? "会话进行中" : isLaunching ? "正在启动…" : scrcpyReady ? "开始镜像" : "镜像引擎准备中"}
            </button>
          </div>
          <div className="capability-panel" aria-live="polite">
            <strong>这台手机的能力</strong>
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
              <p className="capability-pending">正在读取这台手机的能力信息…</p>
            )}
          </div>
          <div className="capability-panel" aria-live="polite">
            <strong>手机当前的锁屏状态</strong>
            {lockReport ? (
              <>
                <p className="capability-summary">{lockSummary(lockReport)}</p>
                <p className="capability-pending">{lockReport.explanation}</p>
                <p className="capability-pending">{lockReport.recovery}</p>
              </>
            ) : lockError ? (
              <p className="capability-pending" role="alert">{lockError}</p>
            ) : (
              <p className="capability-pending">正在读取手机当前的锁屏状态…</p>
            )}
            <button className="secondary-button" type="button" disabled={lockBusy} onClick={() => void wakeDevice(readyDevice.serial)}>
              {lockBusy ? "正在唤醒…" : "唤醒屏幕"}
            </button>
            <p className="capability-pending">MirrorDock 只点亮屏幕，不解锁。设备处于安全锁屏时，需要你本人在手机或镜像窗口中输入解锁凭据。</p>
          </div>
          <div className="capability-panel screenshot-panel" aria-live="polite">
            <strong>截图</strong>
            <p className="capability-pending">把手机当前画面保存到这台电脑的「图片 / MirrorDock」文件夹。截图只保存在本机，不会上传。</p>
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
            <p className="capability-pending">受保护页面（如支付、密码输入）由 Android 自行屏蔽，截出来会是黑屏，这不是故障。</p>
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
              <p className="capability-pending">当前没有录像。在「镜像窗口设置」中打开「录制这一会话的画面」，然后开始镜像即可录制。</p>
            )}
            {recordingError && <p className="capability-pending" role="alert">{recordingError}</p>}
            <p className="capability-pending">录像保存在本机的视频目录（Windows / Linux 为「视频」，macOS 为「影片」）下的 MirrorDock 文件夹，不会上传。录像由镜像窗口直接写入文件，结束镜像即同时结束录制。</p>
          </div>
          <div className="capability-panel transfer-panel" aria-live="polite">
            <strong>文件传输</strong>
            <p className="capability-pending">在电脑与手机之间收发文件。发送由你挑选文件后进入手机的「下载 / MirrorDock」；取回把手机该目录里的文件保存到这台电脑的「下载 / MirrorDock」。文件内容只经过数据线，不会上传。</p>
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
          </>
        ) : (
          <ol className="setup-steps">
            <li><span>1</span><div><strong>使用可传输数据的数据线连接手机</strong><p>如果手机弹出 USB 用途选择，请选择“文件传输”。</p></div></li>
            <li><span>2</span><div><strong>在手机上开启“USB 调试”</strong><p>这是 Android 提供的安全授权，用于将画面显示到这台电脑。</p></div></li>
            <li><span>3</span><div><strong>解锁手机并允许这台电脑</strong><p>在“允许 USB 调试吗？”中选择允许。你可以随时在手机设置中撤销。</p></div></li>
          </ol>
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
            {recentDevices.length > 0 && <p className="recent-note">这份记录只保存在这台电脑上，可随时逐条移除。移除记录不会断开连接，也不会撤销手机上的调试授权。</p>}
            {recentMessage && <p className="recent-note" role="status">{recentMessage}</p>}
          </div>
        )}
      </section>

      <section className="wireless-card" aria-labelledby="wireless-title">
        <div className="wireless-heading">
          <div>
            <p className="eyebrow">也可以使用无线调试</p>
            <h2 id="wireless-title">同一 Wi-Fi 下连接 Android 11 或更高版本</h2>
            <p>配对码仅用于这一次配对，不会保存。已连接设备的网络地址仅保存在这台电脑上。</p>
          </div>
          <button className="secondary-button" type="button" onClick={() => setWirelessExpanded((value) => !value)}>
            {wirelessExpanded ? "收起" : "设置无线连接"}
          </button>
        </div>

        {wirelessExpanded && <div className="wireless-content">
          <ol className="wireless-steps">
            <li>在手机的“开发者选项”中打开“无线调试”，并确认手机和电脑在同一 Wi-Fi。</li>
            <li>选择“使用配对码配对设备”，填写手机显示的配对地址和 6 位配对码。</li>
            <li>回到无线调试主页面，填写“IP 地址和端口”中的连接地址；它可能与配对地址不同。</li>
          </ol>
          <div className="wireless-form">
            <label>配对地址<input value={pairEndpoint} onChange={(event) => setPairEndpoint(event.target.value)} placeholder="例如 192.168.1.20:37123" autoComplete="off" /></label>
            <label>6 位配对码<input value={pairingCode} onChange={(event) => setPairingCode(event.target.value.replace(/\D/g, "").slice(0, 6))} inputMode="numeric" placeholder="不会保存" autoComplete="one-time-code" /></label>
            <label>连接地址<input value={connectEndpoint} onChange={(event) => setConnectEndpoint(event.target.value)} placeholder="例如 192.168.1.20:41839" autoComplete="off" /></label>
            <button className="primary-button" type="button" onClick={() => void pairAndConnect()} disabled={wirelessBusy}>{wirelessBusy ? "正在连接…" : "配对并连接"}</button>
          </div>
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

      <aside className="privacy-note">
        <strong>为什么需要授权？</strong>
        <p>MirrorDock 通过 Android 的 USB 调试机制获得画面和控制权限。该授权只授予你确认过的电脑，且可以在手机的开发者选项中随时撤销。</p>
      </aside>
    </main>
  );
}

export default App;
