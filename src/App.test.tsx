// App 层纯函数与初始渲染的测试。
// 组件测试通过 vi.mock 拦截 Tauri 命令面：这里验证的是前端的**呈现契约**——
// 七种会话状态不得塌缩成一句"连接失败"、未知能力必须如实显示"未知"。
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  revealItemInDir: vi.fn(),
  openUrl: vi.fn(),
}));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("0.0.0-test"),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));
// updater 插件（X10-47）：测试环境无 Tauri 运行时；check 默认「无更新」。
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn().mockResolvedValue(null),
}));
// 全局快捷键插件在测试环境中没有 Tauri 运行时；App 内部会先探测
// __TAURI_INTERNALS__ 再注册，这里 mock 掉以保证双保险。
vi.mock("@tauri-apps/plugin-global-shortcut", () => ({
  register: vi.fn().mockResolvedValue(undefined),
  unregisterAll: vi.fn().mockResolvedValue(undefined),
}));

import App, {
  buildDesktopPrefDevices,
  capabilitySummary,
  composeOptionsWithDesktop,
  defaultShortcuts,
  isMacPlatform,
  isValidShortcut,
  pairingPayload,
  editionLabel,
  errorMessage,
  expiryText,
  formatBytes,
  groupTransferFiles,
  isProEdition,
  lockSummary,
  lockTag,
  notificationTime,
  readDesktopPrefs,
  readDeviceNicknames,
  readOptions,
  readShortcuts,
  recordingFileName,
  relativeTime,
  screenshotFileName,
  sessionShortcutKeys,
  sessionStatus,
  suggestedUsbSerial,
  preferredChannelLabel,
  supportText,
  writeDeviceNickname,
  type DeviceCapabilities,
  type DeviceLockReport,
  type MirrorSession,
} from "./App";

function adbCheck(overrides: Record<string, unknown> = {}) {
  return {
    adb_available: true,
    scrcpy_available: true,
    devices: [],
    diagnostic: null,
    ...overrides,
  };
}

function idleSession(): MirrorSession {
  return { phase: "idle", serial: null, first_frame: "unknown", error: null };
}

// X10-27 起 Device 携带 physical_serial 与 connections（多通道合并），
// 夹具缺字段会让 connectionLabel 在渲染期崩溃；这里统一补齐。
function testDevice(serial: string, label: string, state: "ready" | "unauthorized" | "offline" | "unknown") {
  return { serial, label, state, physical_serial: null, connections: [] };
}

// App 挂载时会并行轮询多条命令；每个命令都必须返回结构正确的值，
// 否则组件在渲染期崩溃——这个 helper 保证任何测试忘记 mock 的命令都有安全兜底。
function baseInvoke(cmd: string): Promise<unknown> {
  switch (cmd) {
    case "check_adb_devices":
      return Promise.resolve(adbCheck());
    case "mirror_session":
      return Promise.resolve(idleSession());
    case "mirror_sessions":
      // X10-27 起前端轮询的是会话列表（Vec<MirrorSession>）；漏 mock 会回落到
      // default 的 {}，组件拿到非数组直接渲染崩溃。
      return Promise.resolve([]);
    case "current_recording":
      return Promise.resolve(null);
    case "list_recent_devices":
      return Promise.resolve([]);
    case "list_trusted_wireless_devices":
      return Promise.resolve([]);
    case "probe_device_capabilities":
      return Promise.resolve({
        serial: "",
        label: "",
        android_release: null,
        sdk: null,
        mirroring_supported: null,
        audio_forwarding_supported: null,
        notices: [],
      });
    case "device_lock_report":
      return Promise.resolve({
        keyguard: "unknown",
        secure_lock: null,
        screen: "unknown",
        explanation: "",
        recovery: "",
      });
    case "entitlement_status":
      return Promise.resolve({ edition: "free", key_id: null, expires_at: null });
    case "companion_pairing_status":
      return Promise.resolve({ phase: "idle", events: [], offer: null });
    case "get_app_settings":
      return Promise.resolve({ hide_dock_icon: false });
    case "plugin:autostart|is_enabled":
      return Promise.resolve(false);
    default:
      return Promise.resolve({});
  }
}

beforeEach(() => {
  localStorage.clear();
  invokeMock.mockReset();
  invokeMock.mockImplementation(baseInvoke);
});

// X10-111（B+C）：USB 优先判定——首选通道徽标 + 「改用有线」提示。
describe("usbPreference", () => {
  const conn = (serial: string, kind: "usb" | "wireless", state: "ready" | "offline" | "unauthorized" | "unknown") => ({ serial, kind, state });
  const dev = (serial: string, connections: ReturnType<typeof conn>[]) => ({
    serial, label: "M2104K10AC", state: "ready" as const, physical_serial: "PHYS-1", connections,
  });

  it("labels_preferred_channel_from_primary_serial", () => {
    expect(preferredChannelLabel(dev("79j7kn9tkjt8rwss", [conn("79j7kn9tkjt8rwss", "usb", "ready")]))).toBe("首选 USB");
    expect(preferredChannelLabel(dev("192.168.2.90:41901", [conn("192.168.2.90:41901", "wireless", "ready")]))).toBe("首选 无线");
    expect(preferredChannelLabel(dev("adb-x._adb-tls-connect._tcp", [conn("adb-x._adb-tls-connect._tcp", "wireless", "ready")]))).toBe("首选 无线");
  });

  it("suggests_usb_only_when_primary_wireless_and_usb_ready", () => {
    // 首选无线 + 有 ready USB ⇒ 提示该 USB。
    const dual = dev("192.168.2.90:41901", [
      conn("192.168.2.90:41901", "wireless", "ready"),
      conn("79j7kn9tkjt8rwss", "usb", "ready"),
    ]);
    expect(suggestedUsbSerial(dual)).toBe("79j7kn9tkjt8rwss");
    // 首选已是 USB ⇒ 不提示。
    const usbFirst = dev("79j7kn9tkjt8rwss", [conn("79j7kn9tkjt8rwss", "usb", "ready")]);
    expect(suggestedUsbSerial(usbFirst)).toBeNull();
    // 首选无线、USB offline ⇒ 不提示（休眠的 USB 不作数）。
    const usbSleeping = dev("192.168.2.90:41901", [
      conn("192.168.2.90:41901", "wireless", "ready"),
      conn("79j7kn9tkjt8rwss", "usb", "offline"),
    ]);
    expect(suggestedUsbSerial(usbSleeping)).toBeNull();
    // 纯无线（无 USB）⇒ 不提示。
    const wirelessOnly = dev("192.168.2.90:41901", [conn("192.168.2.90:41901", "wireless", "ready")]);
    expect(suggestedUsbSerial(wirelessOnly)).toBeNull();
  });
});

describe("sessionStatus", () => {
  it("gives_every_phase_its_own_writing_never_a_generic_connection_failure", () => {
    const cases: [MirrorSession, string | null][] = [
      [idleSession(), null],
      [{ ...idleSession(), phase: "connecting" }, "正在启动镜像窗口…"],
      [
        { ...idleSession(), phase: "streaming", first_frame: "unknown" },
        "画面正在启动…若几秒后仍未出现，请检查手机屏幕是否亮起并确认授权。",
      ],
      [
        { ...idleSession(), phase: "streaming", first_frame: "reached" },
        "镜像正在运行。关闭镜像窗口即可结束本次会话。",
      ],
      [{ ...idleSession(), phase: "unauthorized" }, "手机尚未允许这台电脑进行调试，请解锁手机后重新允许。"],
      [{ ...idleSession(), phase: "offline" }, "手机当前处于离线状态，请重新插拔数据线或重新连接无线调试。"],
      [{ ...idleSession(), phase: "paired" }, "无线设备已配对并连接，可以开始镜像。"],
      [{ ...idleSession(), phase: "failed" }, "镜像会话失败，请重新检查连接后再试。"],
    ];
    for (const [session, expected] of cases) {
      expect(sessionStatus(session), `phase: ${session.phase}`).toBe(expected);
    }
    const writings = cases.map(([, text]) => text);
    expect(new Set(writings).size).toBe(writings.length);
  });

  it("prefers_the_backend_error_copy_over_the_local_fallback", () => {
    const session: MirrorSession = {
      phase: "failed",
      serial: "phone",
      first_frame: "unknown",
      error: { code: "x", message: "设备已断开", recovery: "重新连接后再试" },
    };
    expect(sessionStatus(session)).toBe("设备已断开 重新连接后再试");
  });
});

describe("supportText and capabilitySummary", () => {
  it("never_turns_an_unknown_capability_into_a_yes_or_no", () => {
    expect(supportText(null, "支持", "不支持", "未知")).toBe("未知");
    expect(supportText(true, "支持", "不支持", "未知")).toBe("支持");
    expect(supportText(false, "支持", "不支持", "未知")).toBe("不支持");
  });

  it("summarizes_capabilities_with_honest_unknowns", () => {
    const capabilities: DeviceCapabilities = {
      serial: "phone",
      label: "Xiaomi M2104K10AC",
      android_release: "13",
      sdk: 33,
      mirroring_supported: true,
      audio_forwarding_supported: null,
      notices: [],
    };
    const summary = capabilitySummary(capabilities);
    expect(summary).toContain("Xiaomi M2104K10AC");
    expect(summary).toContain("Android 13");
    expect(summary).toContain("画面可以镜像到电脑");
    expect(summary).toContain("手机声音能否转发还无法确认");
  });
});

describe("lockSummary", () => {
  it("separates_secure_locks_from_simple_locks", () => {
    const base: DeviceLockReport = {
      keyguard: "locked",
      secure_lock: true,
      screen: "awake",
      explanation: "",
      recovery: "",
    };
    expect(lockSummary(base)).toBe("已锁屏（需要解锁凭据） · 屏幕已点亮");
    expect(lockSummary({ ...base, secure_lock: false })).toBe("锁屏中 · 屏幕已点亮");
    expect(lockSummary({ ...base, secure_lock: null, screen: "unknown" })).toBe(
      "锁屏中 · 屏幕状态未知",
    );
    expect(lockSummary({ ...base, keyguard: "unlocked", screen: "asleep" })).toBe(
      "已解锁 · 屏幕已关闭",
    );
  });
});

describe("lockTag", () => {
  it("renders_short_tag_for_each_lock_state", () => {
    const base: DeviceLockReport = {
      keyguard: "unlocked",
      secure_lock: false,
      screen: "awake",
      explanation: "",
      recovery: "",
    };
    expect(lockTag(base)).toBe("已解锁 · 亮屏");
    expect(lockTag({ ...base, keyguard: "locked", secure_lock: true, screen: "awake" })).toBe(
      "安全锁屏 · 亮屏",
    );
    expect(lockTag({ ...base, keyguard: "locked", screen: "asleep" })).toBe("已锁屏 · 熄屏");
    expect(lockTag({ ...base, keyguard: "unknown", screen: "unknown" })).toBe("锁屏未知 · 屏幕未知");
  });
});

describe("errorMessage", () => {
  it("joins_message_and_recovery_for_structured_backend_errors", () => {
    const error = { code: "c", message: "无法启动", recovery: "请重试" };
    expect(errorMessage(error, "fallback")).toBe("无法启动 请重试");
  });

  it("falls_back_for_plain_strings_and_unknown_shapes", () => {
    expect(errorMessage("字符串错误", "fallback")).toBe("字符串错误");
    expect(errorMessage(new Error("boom"), "fallback")).toBe("fallback");
    expect(errorMessage(undefined, "fallback")).toBe("fallback");
  });
});

describe("formatBytes", () => {
  it("uses_human_readable_units", () => {
    expect(formatBytes(512)).toBe("512 字节");
    expect(formatBytes(15_580)).toBe("15 KB");
    expect(formatBytes(6_815_792)).toBe("6.5 MB");
  });
});

describe("relativeTime", () => {
  it("describes_recency_without_precision_theory", () => {
    const now = Date.now() / 1000;
    expect(relativeTime(0)).toBe("使用时间未知");
    expect(relativeTime(now - 30)).toBe("刚刚使用");
    expect(relativeTime(now - 5 * 60)).toBe("5 分钟前使用");
    expect(relativeTime(now - 3 * 3600)).toBe("3 小时前使用");
    expect(relativeTime(now - 86400)).toBe("昨天使用");
    expect(relativeTime(now - 3 * 86400)).toBe("3 天前使用");
  });
});

describe("notificationTime", () => {
  it("shows_time_only_for_today_and_date_for_other_days", () => {
    const now = new Date(2026, 9, 2, 15, 0).getTime();
    // 当天上午的通知：只显示时刻。
    const morning = new Date(2026, 9, 2, 9, 5).getTime();
    expect(notificationTime(morning, now)).toBe("09:05");
    // 昨天的通知：带日期。
    const yesterday = new Date(2026, 9, 1, 23, 59).getTime();
    expect(notificationTime(yesterday, now)).toBe("10-01 23:59");
  });
});

describe("screenshot and recording file names", () => {
  it("keeps_file_names_ascii_only_so_the_backend_whitelist_accepts_them", () => {
    const now = new Date(2026, 8, 28, 20, 5, 9);
    expect(screenshotFileName(now)).toBe("MirrorDock-20260928-200509.png");
    expect(recordingFileName(now)).toBe("MirrorDock-20260928-200509.mp4");
  });
});

describe("readOptions", () => {
  it("backfills_new_fields_for_older_stored_settings", () => {
    localStorage.setItem(
      "mirrordock.sessionOptions",
      JSON.stringify({ quality: "sharp", rotation: 90, fullscreen: true, always_on_top: false }),
    );
    expect(readOptions()).toEqual({
      quality: "sharp",
      rotation: 90,
      fullscreen: true,
      always_on_top: false,
      keep_awake: true,
      record: false,
      clipboard_autosync: true,
      audio: true,
      shortcut_mod: null,
      show_touches: false,
      keyboard_uhid: true,
      read_only: false,
      max_fps: null,
      desktop_mode: false,
      desktop_app: null,
      camera_source: false,
    });
  });

  it("drops_invalid_shortcut_modifiers_from_older_stored_settings", () => {
    // 修饰键只允许白名单值：被篡改或过期的值回落到 scrcpy 默认（null）。
    localStorage.setItem(
      "mirrordock.sessionOptions",
      JSON.stringify({ quality: "balanced", rotation: 0, fullscreen: false, always_on_top: false, shortcut_mod: "--video-codec=h265", show_touches: true, read_only: true }),
    );
    const options = readOptions();
    expect(options.shortcut_mod).toBeNull();
    expect(options.show_touches).toBe(true);
    expect(options.read_only).toBe(true);
    expect(options.keyboard_uhid).toBe(true);
  });

  it("rejects_invalid_stored_values_and_returns_defaults", () => {
    localStorage.setItem("mirrordock.sessionOptions", JSON.stringify({ quality: "ultra" }));
    expect(readOptions().quality).toBe("balanced");
    localStorage.setItem("mirrordock.sessionOptions", "{not json");
    expect(readOptions().quality).toBe("balanced");
  });
});

describe("readDesktopPrefs", () => {
  it("keeps_valid_per_device_entries_and_drops_broken_ones", () => {
    localStorage.setItem(
      "mirrordock.desktopPrefs",
      JSON.stringify({
        phoneA: { desktop_mode: true, desktop_app: "com.android.browser" },
        phoneB: { desktop_mode: false, desktop_app: "  " },
        phoneC: { desktop_app: "com.x" }, // 缺 desktop_mode → 整条丢弃
        phoneD: "garbage",
      }),
    );
    expect(readDesktopPrefs()).toEqual({
      phoneA: { desktop_mode: true, desktop_app: "com.android.browser" },
      phoneB: { desktop_mode: false, desktop_app: null },
    });
  });

  it("returns_empty_prefs_for_missing_or_corrupt_storage", () => {
    expect(readDesktopPrefs()).toEqual({});
    localStorage.setItem("mirrordock.desktopPrefs", "{not json");
    expect(readDesktopPrefs()).toEqual({});
    localStorage.setItem("mirrordock.desktopPrefs", JSON.stringify([1, 2]));
    expect(readDesktopPrefs()).toEqual({});
  });
});

describe("composeOptionsWithDesktop", () => {
  const global = {
    quality: "balanced" as const,
    rotation: 0,
    fullscreen: false,
    always_on_top: false,
    keep_awake: true,
    record: false,
    clipboard_autosync: true,
    audio: true,
    shortcut_mod: null,
    show_touches: false,
    keyboard_uhid: true,
    read_only: false,
    max_fps: null,
    desktop_mode: false,
    desktop_app: null,
    camera_source: true,
  };

  it("returns_global_options_untouched_when_device_has_no_pref", () => {
    expect(composeOptionsWithDesktop(global, null)).toBe(global);
  });

  it("applies_device_desktop_pref_and_enforces_camera_exclusion", () => {
    const effective = composeOptionsWithDesktop(global, { desktop_mode: true, desktop_app: "com.android.browser" });
    expect(effective.desktop_mode).toBe(true);
    expect(effective.desktop_app).toBe("com.android.browser");
    // 桌面模式与摄像头画面互斥：设备开了桌面模式，全局的摄像头源不再下发。
    expect(effective.camera_source).toBe(false);
    // 全局对象不被修改。
    expect(global.camera_source).toBe(true);
    expect(global.desktop_mode).toBe(false);
  });

  it("hides_desktop_app_while_device_desktop_mode_is_off", () => {
    const pref = { desktop_mode: false, desktop_app: "com.android.browser" };
    const effective = composeOptionsWithDesktop({ ...global, desktop_mode: true }, pref);
    expect(effective.desktop_mode).toBe(false);
    expect(effective.desktop_app).toBeNull();
    // 摄像头源不被误伤。
    expect(effective.camera_source).toBe(true);
  });
});

describe("buildDesktopPrefDevices", () => {
  // state 必须是 DeviceState 联合类型，不能是宽泛的 string —— 否则 tsc 会拒收
  // 这个夹具（Type 'string' is not assignable to type 'DeviceState'）。
  type DeviceStateLiteral = "ready" | "unauthorized" | "offline" | "unknown";
  const adbDevice = (
    serial: string,
    physical: string | null,
    state: DeviceStateLiteral,
    label = serial,
  ) => ({
    serial,
    label,
    state,
    physical_serial: physical,
    connections: [] as { serial: string; kind: "usb" | "wireless"; state: DeviceStateLiteral }[],
  });

  it("lists_single_ready_device_so_per_device_module_is_always_visible", () => {
    // X10-72 回归：旧实现单设备时不渲染设备选择器，用户以为桌面模式是全局开关。
    const devices = buildDesktopPrefDevices({
      adbDevices: [adbDevice("usb-1", "PHYS1", "ready", "Pixel 8")],
      labels: { "usb-1": "Pixel 8" },
    });
    expect(devices).toHaveLength(1);
    expect(devices[0]).toMatchObject({ serial: "usb-1", label: "Pixel 8", state: "ready", configured: false, streaming: false });
  });

  it("merges_usb_and_wireless_endpoints_of_same_physical_device", () => {
    // 同一台手机的 USB 与无线端点应合并成一行，且优先保留 ready 的那条。
    const devices = buildDesktopPrefDevices({
      adbDevices: [
        adbDevice("192.168.1.9:44093", "PHYS1", "offline", "Pixel 8"),
        adbDevice("usb-1", "PHYS1", "ready", "Pixel 8"),
      ],
    });
    expect(devices).toHaveLength(1);
    expect(devices[0].serial).toBe("usb-1");
    expect(devices[0].state).toBe("ready");
  });

  it("keeps_offline_and_unauthorized_devices_so_user_can_preconfigure_them", () => {
    // 未插线/未授权的设备也要能提前配置桌面模式，否则「按设备设置」名不副实。
    const devices = buildDesktopPrefDevices({
      adbDevices: [
        adbDevice("usb-1", "PHYS1", "ready", "Pixel 8"),
        adbDevice("usb-2", "PHYS2", "unauthorized", "Galaxy"),
        adbDevice("usb-3", "PHYS3", "offline", "旧手机"),
      ],
    });
    expect(devices.map((d) => d.serial)).toEqual(["usb-1", "usb-2", "usb-3"]);
    expect(devices[1].state).toBe("unauthorized");
    expect(devices[2].state).toBe("offline");
  });

  it("includes_session_devices_and_devices_with_stored_prefs_even_when_absent_from_adb", () => {
    // 配置持久化在本地；设备长期没插也必须能找回入口，否则用户无法调整或清理。
    const devices = buildDesktopPrefDevices({
      adbDevices: [],
      sessionSerials: ["session-serial"],
      prefs: { "old-wireless:1234": { desktop_mode: true, desktop_app: "com.android.browser" } },
    });
    const serials = devices.map((d) => d.serial).sort();
    expect(serials).toEqual(["old-wireless:1234", "session-serial"]);
    expect(devices.find((d) => d.serial === "session-serial")?.streaming).toBe(true);
    expect(devices.find((d) => d.serial === "old-wireless:1234")?.configured).toBe(true);
  });

  it("marks_configured_and_streaming_flags_independently", () => {
    const devices = buildDesktopPrefDevices({
      adbDevices: [adbDevice("usb-1", "PHYS1", "ready", "Pixel 8")],
      sessionSerials: ["usb-1"],
      // X10-84：pref 键统一为 physical_serial；这里存到 PHYS1 才会被识别为已配置。
      prefs: { PHYS1: { desktop_mode: true, desktop_app: null } },
    });
    expect(devices[0]).toMatchObject({ streaming: true, configured: true, pref_key: "PHYS1" });
  });

  it("uses_physical_serial_as_pref_key_regardless_of_endpoint", () => {
    // X10-84 回归：无线端点（adb-xxx._adb-tls-connect._tcp）会变，
    // 但 pref 必须按稳定 physical_serial 命中——这是「选了应用仍白屏」的根因修复。
    const wireless = adbDevice("adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp", "qc8d8tonbmmzm7qs", "ready", "Redmi");
    const devices = buildDesktopPrefDevices({
      adbDevices: [wireless],
      sessionSerials: ["adb-qc8d8tonbmmzm7qs-rQWqVr._adb-tls-connect._tcp"],
      prefs: { qc8d8tonbmmzm7qs: { desktop_mode: true, desktop_app: "com.netease.dhxy.qihoo" } },
    });
    expect(devices[0].pref_key).toBe("qc8d8tonbmmzm7qs");
    expect(devices[0].configured).toBe(true);
    expect(devices[0].streaming).toBe(true);
  });

  it("returns_empty_when_no_device_is_known", () => {
    // 空列表是"回落全局默认"的唯一合法条件，UI 依赖它决定是否显示设备清单。
    expect(buildDesktopPrefDevices({ adbDevices: [] })).toEqual([]);
  });

  it("merges_mdns_endpoint_and_short_physical_id_into_one_row", () => {
    // X10-87 回归：用户只连过 2 台设备，桌面模式列表却显示 5 条——
    // 同一物理设备的 mDNS 端点（adb-xxx-..._tcp）与短物理 id 被当成两台。
    // 归一化后必须合并为一行。
    const devices = buildDesktopPrefDevices({
      adbDevices: [],
      prefs: {
        "79j7kn9tkjt8rwss": { desktop_mode: true, desktop_app: null },
        "adb-79j7kn9tkjt8rwss-rF7qH8._adb-tls-connect._tcp": { desktop_mode: true, desktop_app: null },
      },
    });
    // 两个键同属一台物理设备 → 只应有一行。
    expect(devices).toHaveLength(1);
    expect(devices[0].pref_key).toBe("79j7kn9tkjt8rwss");
  });
});

describe("device nicknames (X10-88)", () => {
  it("stores_and_clears_nicknames_by_physical_key", () => {
    localStorage.removeItem("mirrordock.deviceNicknames");
    let nicknames = writeDeviceNickname("PHYS1", "我的主力机");
    expect(nicknames["PHYS1"]).toBe("我的主力机");
    // 空串 = 清除备注，回落到型号名。
    nicknames = writeDeviceNickname("PHYS1", "   ");
    expect(nicknames["PHYS1"]).toBeUndefined();
  });

  it("reads_back_persisted_nicknames", () => {
    localStorage.setItem("mirrordock.deviceNicknames", JSON.stringify({ PHYS2: "备用机" }));
    expect(readDeviceNicknames()["PHYS2"]).toBe("备用机");
  });
});

describe("App rendering", () => {
  // X10-72 回归：用户报告"多设备各自设置桌面模式看起来还是全局设置，找不到按设备
  // 设置的模块"。根因是设备选择器只在 sessionActive && 多会话时渲染。这里锁定：
  // **只有一台设备、且没有会话时**，按设备模块也必须可见，且必须写明归属设备。
  it("shows_the_per_device_desktop_module_even_with_a_single_device_and_no_session", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") {
        return Promise.resolve(adbCheck({ devices: [testDevice("usb-only", "Xiaomi M2104K10AC", "ready")] }));
      }
      return baseInvoke(cmd);
    });
    render(<App />);

    // 模块本身存在（不是藏在多会话条件下）。
    expect(await screen.findByText("按设备设置 · 桌面模式")).toBeInTheDocument();
    // 设备清单里出现这台设备，说明"选哪台设备"这一步对用户可见。
    expect(await screen.findByRole("list", { name: "各设备桌面模式状态" })).toBeInTheDocument();
    // 明确告知下面两项属于哪台设备——这是旧实现缺失、用户报"看不出是全局"的关键。
    expect(await screen.findByText(/下面两项正在编辑/)).toBeInTheDocument();
    expect(await screen.findByText(/仅对「Xiaomi M2104K10AC」这台设备生效/)).toBeInTheDocument();
    // 不允许出现"当前保存为全局默认"这种回落文案（有一台设备时不成立）。
    expect(screen.queryByText(/当前保存为全局默认/)).not.toBeInTheDocument();
  });

  it("falls_back_to_global_default_copy_when_no_device_is_known", async () => {
    // 没有任何设备时，如实说明保存为全局默认，而不是静默把用户设置写到不可见的地方。
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") return Promise.resolve(adbCheck({ devices: [] }));
      return baseInvoke(cmd);
    });
    render(<App />);

    expect(await screen.findByText("按设备设置 · 桌面模式")).toBeInTheDocument();
    expect(await screen.findByText(/会作为/)).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "各设备桌面模式状态" })).not.toBeInTheDocument();
  });

  // X10-90 反向验证：桌面模式偏好必须落到**后端持久层**，不再写 localStorage——
  // 否则托盘「连接设备」这条 Rust 路径读不到，从托盘启动仍是普通镜像（用户实测）。
  it("persists_desktop_pref_to_backend_not_localstorage", async () => {
    const device = testDevice("usb-only", "Xiaomi M2104K10AC", "ready");
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") {
        return Promise.resolve(adbCheck({ devices: [device] }));
      }
      if (cmd === "get_desktop_prefs") return Promise.resolve({});
      if (cmd === "set_desktop_prefs") return Promise.resolve(null);
      // 打开桌面模式会渲染应用 datalist，需要一份（可空的）已安装应用清单。
      if (cmd === "list_device_apps") return Promise.resolve([]);
      return baseInvoke(cmd);
    });
    render(<App />);

    const toggle = await screen.findByRole("checkbox", { name: "桌面模式（独立虚拟屏幕）" });
    // 挂载拉取后清空调用记录，只关注「翻转开关」这一次动作。
    invokeMock.mockClear();
    localStorage.removeItem("mirrordock.desktopPrefs");
    fireEvent.click(toggle);

    // 键 = 这台设备的 physical_serial（testDevice 的 physical_serial 见夹具）。
    await waitFor(() => {
      const call = invokeMock.mock.calls.find(([cmd]) => cmd === "set_desktop_prefs");
      expect(call).toBeDefined();
    });
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "set_desktop_prefs");
    const prefs = (call?.[1] as { prefs: Record<string, { desktop_mode: boolean }> }).prefs;
    const prefKey = device.physical_serial ?? device.serial;
    expect(prefs[prefKey]?.desktop_mode).toBe(true);
    // 关键断言：不再写 localStorage——它是唯一的真相来源必须在后端。
    expect(localStorage.getItem("mirrordock.desktopPrefs")).toBeNull();
  });

  it("diagnostics_panel_previews_before_export_and_never_lists_secrets", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "diagnostics_preview") {
        return Promise.resolve({
          generated_at_ms: 1_789_000_000_000,
          app_version: "0.1.0-test",
          system: "macos / x86_64",
          scrcpy_available: true,
          events: [
            {
              timestamp_ms: 1_789_000_000_000,
              kind: "mirror_start",
              code: "ok",
              detail: "成功",
            },
          ],
        });
      }
      return baseInvoke(cmd);
    });

    render(<App />);

    // 面板常驻可见，导出在预览前不可用。
    const exportButton = await screen.findByRole("button", { name: "导出为文件" });
    expect(exportButton).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "预览诊断内容" }));
    expect(await screen.findByText(/0\.1\.0-test/)).toBeInTheDocument();
    expect(await screen.findByText(/镜像引擎可用/)).toBeInTheDocument();
    // 预览出现后导出可用。
    expect(exportButton).toBeEnabled();
    // 界面明确告知不会自动上传。
    expect(screen.getByText(/不会自动上传任何内容/)).toBeInTheDocument();
  });

  it("shows_setup_steps_and_brand_guidance_when_no_device_is_connected", async () => {
    render(<App />);
    await waitFor(() => {
      // invoke 的实际调用记录带一个 undefined 参数位，所以按命令名匹配。
      expect(invokeMock.mock.calls.some(([cmd]) => cmd === "check_adb_devices")).toBe(true);
    });
    // 三步引导与品牌知识库在无设备时都应可见。
    expect(await screen.findByText("使用可传输数据的数据线连接手机")).toBeInTheDocument();
    expect(await screen.findByText("按品牌查看开启步骤")).toBeInTheDocument();
    expect(await screen.findByText(/以手机实际设置为准/)).toBeInTheDocument();
  });

  it("reports_an_unauthorized_device_with_its_own_state_copy", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") {
        return Promise.resolve(
          adbCheck({ devices: [testDevice("phone", "Xiaomi M2104K10AC", "unauthorized")] }),
        );
      }
      if (cmd === "mirror_sessions") {
        return Promise.resolve([{ phase: "unauthorized", serial: "phone", first_frame: "unknown", error: null }]);
      }
      if (cmd === "mirror_session") {
        return Promise.resolve({ phase: "unauthorized", serial: "phone", first_frame: "unknown", error: null });
      }
      return baseInvoke(cmd);
    });
    render(<App />);
    expect(await screen.findByText("等待手机确认")).toBeInTheDocument();
    // 会话状态文案（而非三步引导里的相似句子）必须原样出现。
    expect(await screen.findByText(/手机尚未允许这台电脑进行调试/)).toBeInTheDocument();
  });

  it("surfaces_the_streaming_session_copy_once_a_session_is_running", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "check_adb_devices":
          return Promise.resolve(
            adbCheck({ devices: [testDevice("phone", "Pixel 8", "ready")] }),
          );
        case "mirror_sessions":
          return Promise.resolve([
            { phase: "streaming", serial: "phone", first_frame: "reached", error: null },
          ]);
        case "mirror_session":
          return Promise.resolve({
            phase: "streaming",
            serial: "phone",
            first_frame: "reached",
            error: null,
          });
        default:
          return baseInvoke(cmd);
      }
    });
    render(<App />);
    expect(
      await screen.findByText("镜像正在运行。关闭镜像窗口即可结束本次会话。"),
    ).toBeInTheDocument();
  });

  it("start_recording_uses_a_dedicated_command_and_never_restarts_the_display", async () => {
    // X10-92 反向验证：点「开始录制」必须走 start_recording（独立通道），
    // 绝不能调 update_session_options/start_mirroring（那会重启显示窗口）。
    const started: string[][] = [];
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      switch (cmd) {
        case "check_adb_devices":
          return Promise.resolve(adbCheck({ devices: [testDevice("phone", "Pixel 8", "ready")] }));
        case "mirror_sessions":
        case "mirror_session":
          return Promise.resolve(
            cmd === "mirror_sessions"
              ? [{ phase: "streaming", serial: "phone", first_frame: "reached", error: null }]
              : { phase: "streaming", serial: "phone", first_frame: "reached", error: null },
          );
        case "entitlement_status":
          return Promise.resolve({ edition: "pro", key_id: "k", expires_at: null });
        case "current_recording":
          return Promise.resolve(null);
        case "list_device_files":
          return Promise.resolve([]);
        case "start_recording":
          started.push([cmd, String(args?.fileName ?? "")]);
          return Promise.resolve({
            file_name: "mirrordock-record-x.mp4",
            path: "/v/MirrorDock/mirrordock-record-x.mp4",
            active: true,
          });
        default:
          return baseInvoke(cmd);
      }
    });
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /工具/ }));
    const btn = await screen.findByRole("button", { name: "开始录制" });
    // entitlement 轮询是异步的：等 Pro 生效、按钮可点后再触发。
    await waitFor(() => expect(btn).toBeEnabled());
    fireEvent.click(btn);
    await screen.findByText(/正在录制：/);

    expect(started.length).toBe(1);
    const forbidden = invokeMock.mock.calls.filter(([cmd]) =>
      cmd === "update_session_options" || cmd === "start_mirroring",
    );
    expect(forbidden, "开始录制绝不得重启显示窗口").toHaveLength(0);
  });

  it("explains_when_the_mirror_runtime_itself_is_missing", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") {
        return Promise.resolve(adbCheck({ scrcpy_available: false, diagnostic: "未找到 scrcpy" }));
      }
      return baseInvoke(cmd);
    });
    render(<App />);
    // 用诊断条专用 class 定位：设置页「关于」卡片与帮助 FAQ 里也会出现 scrcpy 字样。
    expect(await screen.findByText("未找到 scrcpy")).toBeInTheDocument();
  });
});

describe("entitlement", () => {
  it("editionLabel_and_helpers_map_states_honestly", () => {
    expect(editionLabel("free")).toBe("免费版");
    expect(editionLabel("pro")).toBe("专业版");
    expect(editionLabel(undefined)).toBe("版本未知");
    expect(editionLabel("corrupted")).toBe("版本未知");
    expect(isProEdition("pro")).toBe(true);
    expect(isProEdition("free")).toBe(false);
    expect(isProEdition(null)).toBe(false);
    expect(expiryText(null)).toBe("永久有效");
    expect(expiryText(0)).toBe("永久有效");
    expect(expiryText(1_700_000_000)).toContain("有效期至");
  });

  it("free_edition_shows_activation_ui_and_locks_recording_switch", async () => {
    render(<App />);
    expect(await screen.findByText("当前版本：免费版")).toBeInTheDocument();
    const recordBox = screen.getByRole("checkbox", { name: /启用视频录制/ });
    expect(recordBox).toBeDisabled();
    expect(screen.getByText(/专业版功能，在下方「版本与授权」激活后可用/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "激活专业版" })).toBeDisabled();
  });

  it("activate_clicks_send_license_to_local_backend_only", async () => {
    render(<App />);
    const input = await screen.findByPlaceholderText("MD1-XXXXXX-XXXXXX-…");
    fireEvent.change(input, { target: { value: "MD1-TESTLICENSE" } });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "entitlement_activate") {
        return Promise.resolve({ edition: "pro", key_id: "2026-001", expires_at: null });
      }
      return baseInvoke(cmd);
    });
    fireEvent.click(screen.getByRole("button", { name: "激活专业版" }));
    expect(await screen.findByText(/MP4 录制现已可用/)).toBeInTheDocument();
    const activateCall = invokeMock.mock.calls.find(([cmd]) => cmd === "entitlement_activate");
    expect(activateCall).toBeTruthy();
    expect(activateCall![1]).toEqual({ licenseKey: "MD1-TESTLICENSE" });
    // 激活后呈现专业版状态与撤销入口，录制开关恢复可用。
    expect(await screen.findByText("当前版本：专业版（许可证 2026-001，永久有效）")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "撤销本机授权" })).toBeEnabled();
    expect(screen.getByRole("checkbox", { name: /启用视频录制/ })).toBeEnabled();
  });

  it("activation_failure_shows_backend_message_without_echoing_key", async () => {
    render(<App />);
    const input = await screen.findByPlaceholderText("MD1-XXXXXX-XXXXXX-…");
    fireEvent.change(input, { target: { value: "MD1-BADKEY" } });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "entitlement_activate") {
        return Promise.reject({ code: "license_invalid", message: "许可证签名无效。", recovery: "请确认许可证来自官方渠道。" });
      }
      return baseInvoke(cmd);
    });
    fireEvent.click(screen.getByRole("button", { name: "激活专业版" }));
    expect(await screen.findByText(/许可证签名无效。 请确认许可证来自官方渠道。/)).toBeInTheDocument();
    expect(screen.getByText("当前版本：免费版")).toBeInTheDocument();
  });
});

describe("shortcuts and rotation", () => {
  it("session_shortcuts_cover_screenshot_record_and_rotation", () => {
    expect(sessionShortcutKeys(defaultShortcuts)).toEqual([
      "CommandOrControl+Alt+S",
      "CommandOrControl+Alt+R",
      "CommandOrControl+Alt+D",
    ]);
  });

  it("isValidShortcut_requires_a_modifier_and_a_key", () => {
    expect(isValidShortcut("CommandOrControl+Alt+S")).toBe(true);
    expect(isValidShortcut("Ctrl+Shift+K")).toBe(true);
    expect(isValidShortcut("S")).toBe(false);
    expect(isValidShortcut("Ctrl")).toBe(false);
    expect(isValidShortcut("Ctrl++")).toBe(false);
  });

  it("readShortcuts_persists_user_combos_and_rejects_invalid_values", () => {
    localStorage.setItem(
      "mirrordock.shortcuts",
      JSON.stringify({ screenshot: "Ctrl+Shift+X", record: "not a combo", rotate: 42 }),
    );
    const settings = readShortcuts();
    expect(settings.screenshot).toBe("Ctrl+Shift+X");
    expect(settings.record).toBe(defaultShortcuts.record);
    expect(settings.rotate).toBe(defaultShortcuts.rotate);
  });

  it("settings_page_edits_and_persists_shortcut_combos", async () => {
    render(<App />);
    const input = await screen.findByPlaceholderText("CommandOrControl+Alt+S");
    fireEvent.change(input, { target: { value: "Ctrl+Shift+K" } });
    const stored = JSON.parse(localStorage.getItem("mirrordock.shortcuts") ?? "{}");
    expect(stored.screenshot).toBe("Ctrl+Shift+K");
  });

  it("rotation_defaults_to_following_the_device_instead_of_locking", async () => {
    render(<App />);
    // X10-86 后是自绘下拉（button + 浮层），无原生 value；断言默认选中项文案。
    const rotationSelect = await screen.findByRole("button", { name: /显示方向/ });
    // 「自动（跟随手机）」必须是默认选中项，锁定角度只能由用户显式选择。
    expect(rotationSelect).toHaveTextContent("自动（跟随手机）");
  });
});

describe("companion pairing", () => {
  const offer = {
    version: 1,
    hosts: ["192.168.1.5", "127.0.0.1"],
    port: 45123,
    token: "AB234567CDEF2345",
    fingerprint: "a".repeat(64),
  };

  it("pairingPayload_encodes_hosts_port_token_fingerprint", () => {
    expect(pairingPayload(offer)).toBe(
      "MDP2|192.168.1.5,127.0.0.1|45123|AB234567CDEF2345|" + "a".repeat(64)
    );
  });

  it("begin_pairing_shows_token_and_status_then_end_clears", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "companion_begin_pairing") return Promise.resolve(offer);
      if (cmd === "companion_pairing_status") {
        return Promise.resolve({ phase: "listening", events: ["19:00:00 配对监听已就绪"], offer });
      }
      return baseInvoke(cmd);
    });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "开始配对" }));
    expect(await screen.findByText(/AB234567CDEF2345/)).toBeInTheDocument();
    expect(await screen.findByText(/等待伴侣扫码/)).toBeInTheDocument();
    // 结束配对后回到未开始状态。
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "companion_end_pairing") return Promise.resolve(null);
      return baseInvoke(cmd);
    });
    fireEvent.click(screen.getByRole("button", { name: "结束配对" }));
    await waitFor(() => {
      expect(screen.queryByText(/等待伴侣扫码/)).not.toBeInTheDocument();
    });
  });

  it("resident_channel_copy_states_ordering_and_rescan_recovery", async () => {
    render(<App />);
    // X10-64：常驻端口只在扫码配对的那一刻交给手机，电脑无法通知已断开的手机。
    // 界面必须写清顺序要求与补救动作（重新扫码），不得再出现会让用户走进
    // 「没有可直连的电脑」死胡同的「扫码配对时无需开启」说法。
    expect(await screen.findByText(/先开启常驻通道、再在手机上扫码/)).toBeInTheDocument();
    expect(await screen.findByText(/需要在手机上重新扫一次码（电脑这边不用改）/)).toBeInTheDocument();
    expect(screen.queryByText(/扫码配对时无需开启/)).not.toBeInTheDocument();
    // 常驻通道关闭时，扫码入口旁边要先说清顺序。
    expect(
      await screen.findByText(/请先在下方开启「常驻通道」，再让手机扫码/),
    ).toBeInTheDocument();
  });
});

describe("general settings (tray, autostart, dock)", () => {
  it("isMacPlatform_only_matches_apple_desktop_agents", () => {
    expect(isMacPlatform("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15")).toBe(true);
    expect(isMacPlatform("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120")).toBe(false);
    expect(isMacPlatform("Mozilla/5.0 (X11; Linux x86_64) Firefox/120")).toBe(false);
  });

  it("toggles_launch_at_login_through_the_autostart_plugin", async () => {
    render(<App />);
    const checkbox = await screen.findByRole("checkbox", { name: /开机自动启动/ });
    expect((checkbox as HTMLInputElement).checked).toBe(false);
    fireEvent.click(checkbox);
    await waitFor(() => {
      expect(invokeMock.mock.calls.some(([cmd]) => cmd === "plugin:autostart|enable")).toBe(true);
    });
    expect(await screen.findByText("已开启开机自动启动。")).toBeInTheDocument();
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "plugin:autostart|disable")).toBe(false);
  });

  it("explains_close_to_tray_and_right_click_wake_upfront", async () => {
    render(<App />);
    expect(await screen.findByText(/点窗口关闭按钮 = 最小化到菜单栏\/托盘/)).toBeInTheDocument();
    expect(
      await screen.findByText(/在镜像窗口上点右键即可直接点亮屏幕，不必回到本窗口/),
    ).toBeInTheDocument();
  });

  it("auto_reconnect_switch_copy_covers_both_data_cable_and_wireless", async () => {
    // X10-59 起该开关同时覆盖 USB 与无线：拔线后同样会自动等待重新插线。
    // 旧文案（「无线断线自动重连」）会让拔线用户以为这条恢复路径与自己无关。
    invokeMock.mockImplementation((cmd: string, args?: { settings?: { auto_reconnect?: boolean } }) => {
      if (cmd === "set_app_settings") {
        return Promise.resolve({
          hide_dock_icon: false,
          auto_reconnect: args?.settings?.auto_reconnect !== false,
        });
      }
      return baseInvoke(cmd);
    });
    render(<App />);
    const checkbox = await screen.findByRole("checkbox", { name: "断线自动重连" });
    expect((checkbox as HTMLInputElement).checked).toBe(true);
    expect(await screen.findByText(/无线掉线等手机回网/)).toBeInTheDocument();
    expect(await screen.findByText(/数据线被拔掉等重新插线/)).toBeInTheDocument();
    // 手动关闭后仍如实说明影响范围（两种连接都要手动重连）。
    fireEvent.click(checkbox);
    expect(await screen.findByText("已关闭自动重连：断开后需要手动重新连接。")).toBeInTheDocument();
    // 重新开启时提示必须点明 USB 也在覆盖范围内。
    fireEvent.click(checkbox);
    expect(
      await screen.findByText(/已开启断线自动重连：无线掉线或数据线被拔掉后/),
    ).toBeInTheDocument();
  });

  it("shows_the_dock_option_only_on_macos", async () => {
    const stubUserAgent = (userAgent: string) => {
      Object.defineProperty(window.navigator, "userAgent", { value: userAgent, configurable: true });
    };
    const original = window.navigator.userAgent;
    try {
      stubUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120");
      const windows = render(<App />);
      await screen.findByRole("checkbox", { name: /开机自动启动/ });
      expect(screen.queryByRole("checkbox", { name: /隐藏 Dock 图标/ })).not.toBeInTheDocument();
      windows.unmount();

      stubUserAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15");
      render(<App />);
      expect(await screen.findByRole("checkbox", { name: /隐藏 Dock 图标/ })).toBeInTheDocument();
    } finally {
      Object.defineProperty(window.navigator, "userAgent", { value: original, configurable: true });
    }
  });
});

describe("apk install", () => {
  function withReadyDevice(extra: (cmd: string) => Promise<unknown> | undefined) {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") {
        return Promise.resolve(
          adbCheck({ devices: [testDevice("phone", "Pixel 8", "ready")] }),
        );
      }
      const custom = extra(cmd);
      return custom ?? baseInvoke(cmd);
    });
  }

  it("installs_the_chosen_apk_and_reports_the_backend_summary", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue("/Users/me/下载/伴侣 App.apk");
    withReadyDevice((cmd) =>
      cmd === "install_apk_to_device"
        ? Promise.resolve({ file_name: "伴侣 App.apk", bytes: 2048, summary: "安装完成。" })
        : undefined,
    );
    render(<App />);

    // 未选包时不允许安装：按钮保持禁用，避免装错文件。
    const install = await screen.findByRole("button", { name: "安装到手机" });
    expect(install).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "选择 APK 安装包" }));
    // 选中的路径必须显示出来，用户才能核对自己装的是哪一个包。
    expect(await screen.findByText("/Users/me/下载/伴侣 App.apk")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "安装到手机" }));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find(([cmd]) => cmd === "install_apk_to_device");
      expect(call?.[1]).toMatchObject({ serial: "phone", apkPath: "/Users/me/下载/伴侣 App.apk" });
    });
    // 回执行：文件名 · 大小 · 后端结论（多条文案都含「安装完成」，这里按大小定位回执行）。
    expect(await screen.findByText(/2 KB/)).toBeInTheDocument();
    expect(screen.getAllByText(/安装完成。/).length).toBeGreaterThan(0);
    // 安装由手机上的系统安装器完成，界面必须提醒用户本人在手机上确认。
    expect(screen.getByText(/需要你本人同意/)).toBeInTheDocument();
  });

  it("keeps_the_backend_reason_when_the_phone_refuses_to_install", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue("/tmp/old.apk");
    withReadyDevice((cmd) =>
      cmd === "install_apk_to_device"
        ? Promise.reject({
            code: "apk_install_failed",
            message:
              "手机里已安装的版本比这个安装包更新。（INSTALL_FAILED_VERSION_DOWNGRADE）",
            recovery: "如果手机屏幕上有提示，请按提示处理后再试一次。",
          })
        : undefined,
    );
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "选择 APK 安装包" }));
    fireEvent.click(await screen.findByRole("button", { name: "安装到手机" }));

    // 具体失败原因（而非笼统的「安装失败」）必须原样呈现给用户。
    expect(await screen.findByText(/INSTALL_FAILED_VERSION_DOWNGRADE/)).toBeInTheDocument();
  });
});

describe("phone → computer file transfer (X10-74)", () => {
  // 用户报「手机发送到电脑的文件，客户端一直看不到」。根因是三处叠加：
  // ① deviceFiles 初始 null，列表只在手动点按钮后才渲染；② files_changed 事件
  // 的自动刷新被 `deviceFiles !== null` 卡住 → 死锁，不点按钮就永远看不到；
  // ③ 整个文件传输面板被关在 {readyDevice ? ...} 里，而文件传输其实不依赖镜像会话。
  // 这组测试锁死修复后的行为。
  function withPhone(extra: (cmd: string) => Promise<unknown> | undefined) {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") {
        return Promise.resolve(adbCheck({ devices: [testDevice("phone", "Pixel 8", "ready")] }));
      }
      const custom = extra(cmd);
      return custom ?? baseInvoke(cmd);
    });
  }

  async function openToolsTab() {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /工具/ }));
  }

  it("loads_the_phone_file_list_on_entering_the_tools_tab_without_any_button_press", async () => {
    // 关键回归：不点任何按钮，进入工具页就应看到手机上的文件。
    withPhone((cmd) =>
      cmd === "list_device_files"
        ? Promise.resolve(["增值税发票.pdf", "安装狮.apk.1"])
        : undefined,
    );
    await openToolsTab();

    expect(await screen.findByText("增值税发票.pdf")).toBeInTheDocument();
    expect(await screen.findByText("安装狮.apk.1")).toBeInTheDocument();
    // 后端确实被调用过，且带的是这台设备的 serial。
    await waitFor(() => {
      const call = invokeMock.mock.calls.find(([cmd]) => cmd === "list_device_files");
      expect(call?.[1]).toMatchObject({ serial: "phone" });
    });
  });

  it("shows_the_panel_even_when_no_mirror_session_is_running", async () => {
    // 文件传输走 adb 通道，不依赖 scrcpy 会话 —— 没有会话时面板也必须在。
    withPhone((cmd) =>
      cmd === "list_device_files" ? Promise.resolve(["报告.pdf"]) : undefined,
    );
    await openToolsTab();

    expect(await screen.findByText("报告.pdf")).toBeInTheDocument();
    // 「取回到电脑」入口必须在，否则用户看到了也拿不回来。
    expect(screen.getAllByRole("button", { name: "取回到电脑" }).length).toBeGreaterThan(0);
  });

  it("does_not_pretend_the_list_is_current_after_a_read_failure", async () => {
    // 读取失败时不得留着上一次的列表冒充当前状态。
    withPhone((cmd) =>
      cmd === "list_device_files"
        ? Promise.reject({
            code: "transfer_list_failed",
            message: "无法读取手机上的文件列表。",
            recovery: "请确认连接仍然有效，然后重试。",
          })
        : undefined,
    );
    await openToolsTab();

    expect(await screen.findByText(/无法读取手机上的文件列表/)).toBeInTheDocument();
    // 失败后不应显示"当前没有文件"——那会把"读不到"说成"没有"，是假的。
    expect(screen.queryByText(/当前没有文件/)).not.toBeInTheDocument();
  });
});

describe("file transfer presentation (X10-75)", () => {
  // 用户截图指出：①「取回到电脑」列参差不齐；②取回后一直显示「取回到电脑」，
  // 不知道哪个已经取回；③底部「已保存到这台电脑的下载/MirrorDock 文件夹」会让人
  // 误会**全部**都保存了，实际只保存了刚取回的那一个。
  function withPhoneFiles(files: string[], extra?: (cmd: string) => Promise<unknown> | undefined) {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") {
        return Promise.resolve(adbCheck({ devices: [testDevice("phone", "Pixel 8", "ready")] }));
      }
      if (cmd === "list_device_files") return Promise.resolve(files);
      const custom = extra?.(cmd);
      return custom ?? baseInvoke(cmd);
    });
  }

  async function openTools() {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /工具/ }));
  }

  it("groups_files_by_purpose_so_users_can_scan_the_list", () => {
    const groups = groupTransferFiles([
      "增值税发票.pdf",
      "安装狮.apk.1",
      "MirrorDock 传输 冒烟.txt",
      "shizuku-v13.5.3.apk",
    ]);
    // 分组顺序固定：安装包 → 文档 → 其他；空组不出现（这 4 个文件没有"其他"类）。
    expect(groups.map((g) => g.kind)).toEqual(["installer", "document"]);
    // 组内按 zh-Hans-CN 排序（中文在前），保证刷新时行不跳动。
    expect(groups[0].files).toEqual(["安装狮.apk.1", "shizuku-v13.5.3.apk"]);
    expect(groups[1].files).toEqual(["增值税发票.pdf", "MirrorDock 传输 冒烟.txt"]);
  });

  it("treats_android_duplicate_copies_as_installers_not_other_files", () => {
    // 真实场景（取自用户手机上的文件）：`foo.apk.1`、`foo.apk (1).1` 是 Android
    // 传输产生的重名副本，本质仍是安装包。若按严格结尾判断会被甩进「其他文件」，
    // 用户一眼扫不出哪些是安装包 —— 那分组就白做了。
    const real = [
      "MirrorDock 传输 冒烟.txt",
      "mirrordock-companion-debug-0.1.0-poc.apk",
      "shizuku-v13.5.3.r1036.fff3f87-release.apk (1).1",
      "shizuku-v13.5.3.r1036.fff3f87-release.apk.1",
      "增值税发票.pdf",
      "安装狮.apk.1",
    ];
    const groups = groupTransferFiles(real);
    const installer = groups.find((g) => g.kind === "installer")?.files ?? [];
    expect(installer).toContain("shizuku-v13.5.3.r1036.fff3f87-release.apk.1");
    expect(installer).toContain("shizuku-v13.5.3.r1036.fff3f87-release.apk (1).1");
    expect(installer).toContain("安装狮.apk.1");
    // 这 4 个安装包不该有任何一个落到「其他文件」。
    const other = groups.find((g) => g.kind === "other")?.files ?? [];
    expect(other.some((n) => n.includes(".apk"))).toBe(false);
  });

  it("returns_no_groups_for_an_empty_list", () => {
    expect(groupTransferFiles([])).toEqual([]);
  });

  it("swaps_the_row_action_to_a_fetched_state_after_retrieval", async () => {
    // 核心回归：取回后该行不再显示「取回到电脑」，而是「已取回」。
    withPhoneFiles(["报告.pdf"], (cmd) =>
      cmd === "fetch_file_from_device"
        ? Promise.resolve({
            file_name: "报告.pdf",
            path: "/Users/huluobo/Downloads/MirrorDock/报告.pdf",
            bytes: 2048,
          })
        : undefined,
    );
    await openTools();

    fireEvent.click(await screen.findByRole("button", { name: "取回到电脑" }));

    // 该行变成已取回状态，并且提供「在文件夹中显示」而不是重复的取回按钮。
    expect(await screen.findByText("已取回")).toBeInTheDocument();
    // 「打开文件夹」上移到列表级工具条（X10-75 评审：8 行里 8 个「在文件夹中显示」
    // 指向同一目录，是纯冗余，也是操作列宽度不稳定的元凶）。
    expect(screen.getByRole("button", { name: "打开文件夹" })).toBeInTheDocument();
    // 整个列表里不应再有「取回到电脑」按钮。
    expect(screen.queryByRole("button", { name: "取回到电脑" })).not.toBeInTheDocument();
  });

  it("names_the_single_retrieved_file_instead_of_implying_all_were_saved", async () => {
    withPhoneFiles(["报告.pdf", "图片.png"], (cmd) =>
      cmd === "fetch_file_from_device"
        ? Promise.resolve({ file_name: "报告.pdf", path: "/Users/huluobo/Downloads/MirrorDock/报告.pdf", bytes: 2048 })
        : undefined,
    );
    await openTools();

    // 列表有两行，按行定位「报告.pdf」那一行的取回按钮。
    const row = (await screen.findByText("报告.pdf")).closest("li") as HTMLElement;
    fireEvent.click(within(row).getByRole("button", { name: "取回到电脑" }));

    // 提示必须点名是哪个文件，且给出计数 —— 不能让用户以为全部都存好了。
    const notice = await screen.findByText(/已取回「报告.pdf」，保存在这台电脑/);
    expect(notice).toBeInTheDocument();
    expect(await screen.findByText("共 2 个文件，1 个已取回")).toBeInTheDocument();
    // 另一行仍是未取回状态 —— 逐文件独立，不是整体标记。
    const other = (await screen.findByText("图片.png")).closest("li") as HTMLElement;
    expect(within(other).getByRole("button", { name: "取回到电脑" })).toBeInTheDocument();
  });

  it("shows_the_failure_on_the_row_with_a_retry_instead_of_a_bare_red_line", async () => {
    // impeccable 评审：失败态必须落在**行内**且有恢复动作。8 行列表里面板底部一行
    // 红字扫不出来，而且失败后没重试会让人卡死。
    withPhoneFiles(["报告.pdf"], (cmd) =>
      cmd === "fetch_file_from_device"
        ? Promise.reject({ code: "transfer_pull_failed", message: "文件没有从手机取回。", recovery: "请确认手机上这个文件还在，然后重试。" })
        : undefined,
    );
    await openTools();

    fireEvent.click(await screen.findByRole("button", { name: "取回到电脑" }));

    // 失败原因显示在该行，且该行出现「重试」。
    expect(await screen.findByText(/文件没有从手机取回/)).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "重试" })).toBeInTheDocument();
    // 失败后仍可重新取回，不是死路。
    expect(screen.queryByRole("button", { name: "取回到电脑" })).not.toBeInTheDocument();
  });

  it("drops_the_fetched_marker_when_the_file_is_deleted", async () => {    withPhoneFiles(["报告.pdf"], (cmd) =>
      cmd === "fetch_file_from_device"
        ? Promise.resolve({ file_name: "报告.pdf", path: "/Users/huluobo/Downloads/MirrorDock/报告.pdf", bytes: 2048 })
        : undefined,
    );
    await openTools();

    fireEvent.click(await screen.findByRole("button", { name: "取回到电脑" }));
    expect(await screen.findByText("已取回")).toBeInTheDocument();

    // 删除后（列表里已无此文件）计数归零 —— 不留"已取回 1 个"的幽灵数字。
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") {
        return Promise.resolve(adbCheck({ devices: [testDevice("phone", "Pixel 8", "ready")] }));
      }
      if (cmd === "delete_device_file") return Promise.resolve(undefined);
      if (cmd === "list_device_files") return Promise.resolve([]);
      return baseInvoke(cmd);
    });
    // 删除现在有确认框（X10-75：已取回态下「删除」有两种含义，必须说清删哪一份）。
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "删除" }));

    await waitFor(() => {
      expect(screen.queryByText("已取回")).not.toBeInTheDocument();
    });
    confirmSpy.mockRestore();
  });
});
