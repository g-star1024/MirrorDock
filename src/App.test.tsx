// App 层纯函数与初始渲染的测试。
// 组件测试通过 vi.mock 拦截 Tauri 命令面：这里验证的是前端的**呈现契约**——
// 七种会话状态不得塌缩成一句"连接失败"、未知能力必须如实显示"未知"。
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  revealItemInDir: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));
// 全局快捷键插件在测试环境中没有 Tauri 运行时；App 内部会先探测
// __TAURI_INTERNALS__ 再注册，这里 mock 掉以保证双保险。
vi.mock("@tauri-apps/plugin-global-shortcut", () => ({
  register: vi.fn().mockResolvedValue(undefined),
  unregisterAll: vi.fn().mockResolvedValue(undefined),
}));

import App, {
  capabilitySummary,
  defaultShortcuts,
  isMacPlatform,
  isValidShortcut,
  pairingPayload,
  editionLabel,
  errorMessage,
  expiryText,
  formatBytes,
  isProEdition,
  lockSummary,
  readOptions,
  readShortcuts,
  recordingFileName,
  relativeTime,
  screenshotFileName,
  sessionShortcutKeys,
  sessionStatus,
  supportText,
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

// App 挂载时会并行轮询多条命令；每个命令都必须返回结构正确的值，
// 否则组件在渲染期崩溃——这个 helper 保证任何测试忘记 mock 的命令都有安全兜底。
function baseInvoke(cmd: string): Promise<unknown> {
  switch (cmd) {
    case "check_adb_devices":
      return Promise.resolve(adbCheck());
    case "mirror_session":
      return Promise.resolve(idleSession());
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
      read_only: false,
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
  });

  it("rejects_invalid_stored_values_and_returns_defaults", () => {
    localStorage.setItem("mirrordock.sessionOptions", JSON.stringify({ quality: "ultra" }));
    expect(readOptions().quality).toBe("balanced");
    localStorage.setItem("mirrordock.sessionOptions", "{not json");
    expect(readOptions().quality).toBe("balanced");
  });
});

describe("App rendering", () => {
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
          adbCheck({ devices: [{ serial: "phone", label: "Xiaomi M2104K10AC", state: "unauthorized" }] }),
        );
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
            adbCheck({ devices: [{ serial: "phone", label: "Pixel 8", state: "ready" }] }),
          );
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

  it("explains_when_the_mirror_runtime_itself_is_missing", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "check_adb_devices") {
        return Promise.resolve(adbCheck({ scrcpy_available: false, diagnostic: "未找到 scrcpy" }));
      }
      return baseInvoke(cmd);
    });
    render(<App />);
    expect(await screen.findByText(/scrcpy/)).toBeInTheDocument();
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
    const recordBox = screen.getByRole("checkbox", { name: /录制这一会话的画面/ });
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
    expect(screen.getByRole("checkbox", { name: /录制这一会话的画面/ })).toBeEnabled();
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
    const rotationSelect = await screen.findByLabelText(/显示方向/);
    expect(rotationSelect).toHaveValue("0");
    // 「自动（跟随手机）」必须是默认选项，锁定角度只能由用户显式选择。
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
      "MDP1|192.168.1.5,127.0.0.1|45123|AB234567CDEF2345|" + "a".repeat(64)
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
          adbCheck({ devices: [{ serial: "phone", label: "Pixel 8", state: "ready" }] }),
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
