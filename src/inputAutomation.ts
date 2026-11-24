/**
 * 按键映射与操作回放的纯逻辑层（X10-130）。
 *
 * 这个文件不触碰设备、不做任何注入——它只负责数据形状、校验、可读描述与
 * 本机持久化。注入动作由 App.tsx 调用后端 `inject_device_*` 命令完成，后端
 * 还有第二道白名单校验；这里的校验是为了让用户在界面上就看到明确的错误，
 * 而不是等 adb 失败后才知道。
 *
 * 隐私边界：方案与脚本只保存在本机 localStorage，不上传、不进入诊断包。
 */

// ---------------------------------------------------------------------------
// 类型
// ---------------------------------------------------------------------------

/** 可注入动作（与后端 inject_device_* 命令一一对应）。 */
export type KeymapAction =
  | { kind: "key"; keycode: string }
  | { kind: "tap"; x: number; y: number }
  | { kind: "swipe"; x1: number; y1: number; x2: number; y2: number; duration: number };

/** 一条映射：把电脑键盘上的一个物理键绑定到一个注入动作。 */
export type KeymapBinding = {
  /** 归一化后的物理键标识（见 normalizePhysicalKey），如 "w"、"ArrowUp"。 */
  physicalKey: string;
  /** 展示用标签，如 "W"、"↑"。 */
  label: string;
  action: KeymapAction;
};

/** 一个映射方案。 */
export type KeymapProfile = {
  id: string;
  name: string;
  bindings: KeymapBinding[];
};

/** 回放脚本中的一步。`waitMs` 是执行本步**之前**的等待时间。 */
export type ReplayStep =
  | { kind: "key"; keycode: string; waitMs: number }
  | { kind: "tap"; x: number; y: number; waitMs: number }
  | { kind: "swipe"; x1: number; y1: number; x2: number; y2: number; duration: number; waitMs: number };

/** 一个回放脚本。 */
export type ReplayScript = {
  id: string;
  name: string;
  steps: ReplayStep[];
};

// ---------------------------------------------------------------------------
// 白名单与校验（与后端保持同一套边界）
// ---------------------------------------------------------------------------

/** 可注入的 Android 按键码（须与后端 ANDROID_KEYCODES 一致）。 */
export const INJECTABLE_KEYCODES: { value: string; label: string }[] = [
  { value: "KEYCODE_BACK", label: "返回" },
  { value: "KEYCODE_HOME", label: "主页" },
  { value: "KEYCODE_MENU", label: "菜单" },
  { value: "KEYCODE_APP_SWITCH", label: "最近任务" },
  { value: "KEYCODE_ENTER", label: "确认 / 回车" },
  { value: "KEYCODE_DEL", label: "退格" },
  { value: "KEYCODE_SPACE", label: "空格" },
  { value: "KEYCODE_TAB", label: "Tab" },
  { value: "KEYCODE_ESCAPE", label: "Esc" },
  { value: "KEYCODE_VOLUME_UP", label: "音量 +" },
  { value: "KEYCODE_VOLUME_DOWN", label: "音量 −" },
  { value: "KEYCODE_POWER", label: "电源键" },
  { value: "KEYCODE_WAKEUP", label: "唤醒屏幕" },
  { value: "KEYCODE_SLEEP", label: "熄屏" },
  { value: "KEYCODE_DPAD_UP", label: "方向 上" },
  { value: "KEYCODE_DPAD_DOWN", label: "方向 下" },
  { value: "KEYCODE_DPAD_LEFT", label: "方向 左" },
  { value: "KEYCODE_DPAD_RIGHT", label: "方向 右" },
  { value: "KEYCODE_DPAD_CENTER", label: "方向 中" },
  { value: "KEYCODE_HEADSETHOOK", label: "耳机线控" },
  { value: "KEYCODE_MEDIA_PLAY_PAUSE", label: "播放 / 暂停" },
  { value: "KEYCODE_CAMERA", label: "相机快门" },
];

export function isValidKeycode(value: string): boolean {
  return INJECTABLE_KEYCODES.some((item) => item.value === value);
}

export function isValidCoord(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= 9999;
}

export function isValidDuration(value: number): boolean {
  return Number.isInteger(value) && value >= 50 && value <= 60000;
}

/** 等待时间限定在 0..=60000 毫秒，越界值直接截到边界（而不是报错中断编排）。 */
export function clampWaitMs(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(60000, Math.max(0, Math.round(value)));
}

/** 校验整个回放脚本，返回第一个问题的可读描述；`null` 表示可以执行。 */
export function validateReplayScript(steps: ReplayStep[]): string | null {
  if (steps.length === 0) return "脚本还没有任何步骤。";
  if (steps.length > 500) return "脚本最多 500 步，请拆分后再执行。";
  for (let index = 0; index < steps.length; index += 1) {
    const step = steps[index];
    if (step.kind === "key" && !isValidKeycode(step.keycode)) {
      return `第 ${index + 1} 步的按键码不在可注入范围内。`;
    }
    if (step.kind === "tap" && (!isValidCoord(step.x) || !isValidCoord(step.y))) {
      return `第 ${index + 1} 步的坐标超出 0..=9999。`;
    }
    if (step.kind === "swipe") {
      if (![step.x1, step.y1, step.x2, step.y2].every(isValidCoord)) {
        return `第 ${index + 1} 步的坐标超出 0..=9999。`;
      }
      if (!isValidDuration(step.duration)) {
        return `第 ${index + 1} 步的滑动时长必须在 50..=60000 毫秒之间。`;
      }
    }
    if (!Number.isInteger(step.waitMs) || step.waitMs < 0 || step.waitMs > 60000) {
      return `第 ${index + 1} 步的等待时间无效。`;
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// 物理键的捕获与展示
// ---------------------------------------------------------------------------

const SPECIAL_KEY_LABELS: Record<string, string> = {
  ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→",
  Space: "空格", Enter: "回车", Escape: "Esc", Tab: "Tab", Backspace: "退格",
  ShiftLeft: "左Shift", ShiftRight: "右Shift", ControlLeft: "左Ctrl", ControlRight: "右Ctrl",
  AltLeft: "左Alt", AltRight: "右Alt",
};

/**
 * 把 KeyboardEvent 的 `code` 归一化成稳定的物理键标识。
 * 字母 / 数字取主键位（不区分大小写与修饰键），特殊键保留 code 原名。
 * 无法识别（如 IME 组合期间的合成事件）返回 `null`。
 */
export function normalizePhysicalKey(event: { code?: string; key: string }): string | null {
  const code = event.code;
  if (!code) return null;
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1].toLowerCase();
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1];
  if (SPECIAL_KEY_LABELS[code] !== undefined) return code;
  return null;
}

/** 物理键标识 → 展示标签。 */
export function physicalKeyLabel(physicalKey: string): string {
  if (SPECIAL_KEY_LABELS[physicalKey] !== undefined) return SPECIAL_KEY_LABELS[physicalKey];
  if (/^[a-z]$/.test(physicalKey)) return physicalKey.toUpperCase();
  return physicalKey;
}

// ---------------------------------------------------------------------------
// 可读描述
// ---------------------------------------------------------------------------

function keycodeLabel(keycode: string): string {
  const item = INJECTABLE_KEYCODES.find((entry) => entry.value === keycode);
  return item ? `${item.label}（${keycode}）` : keycode;
}

export function describeAction(action: KeymapAction): string {
  switch (action.kind) {
    case "key":
      return `按键 ${keycodeLabel(action.keycode)}`;
    case "tap":
      return `点击 (${action.x}, ${action.y})`;
    case "swipe":
      return `滑动 (${action.x1}, ${action.y1}) → (${action.x2}, ${action.y2}) · ${action.duration}ms`;
  }
}

export function describeStep(step: ReplayStep): string {
  const wait = step.waitMs > 0 ? `，前等 ${step.waitMs}ms` : "";
  if (step.kind === "key") return `按键 ${keycodeLabel(step.keycode)}${wait}`;
  if (step.kind === "tap") return `点击 (${step.x}, ${step.y})${wait}`;
  return `滑动 (${step.x1}, ${step.y1}) → (${step.x2}, ${step.y2}) · ${step.duration}ms${wait}`;
}

// ---------------------------------------------------------------------------
// 本机持久化（localStorage，损坏/缺失一律回落为空集，与 uiTheme 同策略）
// ---------------------------------------------------------------------------

export const KEYMAP_STORAGE_KEY = "mirrordock.keymaps";
export const REPLAY_STORAGE_KEY = "mirrordock.replays";

function readJson(key: string): unknown {
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return null;
    return JSON.parse(raw) as unknown;
  } catch {
    return null;
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // 存储不可用（隐私模式等）：内存态继续可用，不静默伪装已保存。
  }
}

function asString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function asCoord(value: unknown): number | null {
  return typeof value === "number" && isValidCoord(value) ? value : null;
}

function parseAction(value: unknown): KeymapAction | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (record.kind === "key") {
    const keycode = asString(record.keycode);
    return keycode && isValidKeycode(keycode) ? { kind: "key", keycode } : null;
  }
  if (record.kind === "tap") {
    const x = asCoord(record.x);
    const y = asCoord(record.y);
    return x !== null && y !== null ? { kind: "tap", x, y } : null;
  }
  if (record.kind === "swipe") {
    const x1 = asCoord(record.x1);
    const y1 = asCoord(record.y1);
    const x2 = asCoord(record.x2);
    const y2 = asCoord(record.y2);
    const duration = typeof record.duration === "number" && isValidDuration(record.duration) ? record.duration : null;
    return x1 !== null && y1 !== null && x2 !== null && y2 !== null && duration !== null
      ? { kind: "swipe", x1, y1, x2, y2, duration }
      : null;
  }
  return null;
}

function parseWaitMs(value: unknown): number {
  return typeof value === "number" ? clampWaitMs(value) : 0;
}

/** 读取映射方案；任何一条损坏的绑定都被丢弃，而不是让整个面板不可用。 */
export function readKeymaps(): { profiles: KeymapProfile[]; activeId: string | null } {
  const raw = readJson(KEYMAP_STORAGE_KEY);
  if (typeof raw !== "object" || raw === null) return { profiles: [], activeId: null };
  const record = raw as Record<string, unknown>;
  const rawProfiles = Array.isArray(record.profiles) ? record.profiles : [];
  const profiles: KeymapProfile[] = [];
  for (const item of rawProfiles) {
    if (typeof item !== "object" || item === null) continue;
    const entry = item as Record<string, unknown>;
    const id = asString(entry.id);
    const name = asString(entry.name);
    if (!id || !name) continue;
    const rawBindings = Array.isArray(entry.bindings) ? entry.bindings : [];
    const bindings: KeymapBinding[] = [];
    for (const rawBinding of rawBindings) {
      if (typeof rawBinding !== "object" || rawBinding === null) continue;
      const binding = rawBinding as Record<string, unknown>;
      const physicalKey = asString(binding.physicalKey);
      const label = asString(binding.label);
      const action = parseAction(binding.action);
      if (physicalKey && label && action) bindings.push({ physicalKey, label, action });
    }
    profiles.push({ id, name, bindings });
  }
  const activeId = asString(record.activeId);
  return {
    profiles,
    activeId: activeId && profiles.some((profile) => profile.id === activeId) ? activeId : null,
  };
}

export function writeKeymaps(profiles: KeymapProfile[], activeId: string | null): void {
  writeJson(KEYMAP_STORAGE_KEY, { profiles, activeId });
}

/** 读取回放脚本；坏步丢弃、坏脚本跳过。 */
export function readReplays(): ReplayScript[] {
  const raw = readJson(REPLAY_STORAGE_KEY);
  if (!Array.isArray(raw)) return [];
  const scripts: ReplayScript[] = [];
  for (const item of raw) {
    if (typeof item !== "object" || item === null) continue;
    const entry = item as Record<string, unknown>;
    const id = asString(entry.id);
    const name = asString(entry.name);
    if (!id || !name) continue;
    const rawSteps = Array.isArray(entry.steps) ? entry.steps : [];
    const steps: ReplayStep[] = [];
    for (const rawStep of rawSteps) {
      if (typeof rawStep !== "object" || rawStep === null) continue;
      const record = rawStep as Record<string, unknown>;
      const waitMs = parseWaitMs(record.waitMs);
      if (record.kind === "key") {
        const keycode = asString(record.keycode);
        if (keycode && isValidKeycode(keycode)) steps.push({ kind: "key", keycode, waitMs });
      } else if (record.kind === "tap") {
        const x = asCoord(record.x);
        const y = asCoord(record.y);
        if (x !== null && y !== null) steps.push({ kind: "tap", x, y, waitMs });
      } else if (record.kind === "swipe") {
        const parsed = parseAction({ ...record, kind: "swipe" });
        if (parsed && parsed.kind === "swipe") steps.push({ ...parsed, waitMs });
      }
    }
    scripts.push({ id, name, steps });
  }
  return scripts;
}

export function writeReplays(scripts: ReplayScript[]): void {
  writeJson(REPLAY_STORAGE_KEY, scripts);
}

/** 生成本地唯一 id（时间戳 + 随机尾巴，碰撞概率可忽略）。 */
export function newLocalId(): string {
  return `k${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

// ---------------------------------------------------------------------------
// 定时录制计划（X10-131）
// ---------------------------------------------------------------------------

/** 一条定时录制计划：到点自动开始录制，`durationMin` 分钟后自动结束。 */
export type RecordPlan = {
  id: string;
  name: string;
  /** "HH:MM"，24 小时制。 */
  time: string;
  /** 1..=240 分钟。 */
  durationMin: number;
  enabled: boolean;
};

export const RECORD_PLANS_STORAGE_KEY = "mirrordock.recordPlans";

/** 触发窗口：到点后 5 分钟内仍可触发（错过整点片刻不算丢计划）。 */
export const RECORD_PLAN_DUE_WINDOW_MIN = 5;

export const RECORD_PLAN_DURATION_MAX = 240;

function isValidPlanTime(time: unknown): time is string {
  return typeof time === "string" && /^([01]\d|2[0-3]):[0-5]\d$/.test(time);
}

export function isValidRecordPlanDuration(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 1 && value <= RECORD_PLAN_DURATION_MAX;
}

/** 逐条校验并丢弃非法计划；不抛错，坏数据直接跳过。 */
export function parseRecordPlans(value: unknown): RecordPlan[] {
  if (!Array.isArray(value)) return [];
  const plans: RecordPlan[] = [];
  for (const item of value) {
    if (typeof item !== "object" || item === null) continue;
    const entry = item as Record<string, unknown>;
    const id = asString(entry.id);
    const name = asString(entry.name);
    if (!id || !name || !isValidPlanTime(entry.time)) continue;
    if (!isValidRecordPlanDuration(entry.durationMin)) continue;
    if (typeof entry.enabled !== "boolean") continue;
    plans.push({ id, name, time: entry.time, durationMin: entry.durationMin, enabled: entry.enabled });
  }
  return plans;
}

export function readRecordPlans(): RecordPlan[] {
  return parseRecordPlans(readJson(RECORD_PLANS_STORAGE_KEY));
}

export function writeRecordPlans(plans: RecordPlan[]): void {
  writeJson(RECORD_PLANS_STORAGE_KEY, plans);
}

/** 计划时间转当天分钟数；非法时间返回 null。 */
export function planTimeMinutes(time: string): number | null {
  if (!isValidPlanTime(time)) return null;
  const [h, m] = time.split(":").map(Number);
  return h * 60 + m;
}

/**
 * 计划此刻是否该触发。`lastRunKey` 是该计划最近一次触发的日期键（"YYYY-M-D"），
 * 同一天不重复触发；触发窗口见 RECORD_PLAN_DUE_WINDOW_MIN。
 */
export function isRecordPlanDue(plan: RecordPlan, now: Date, lastRunKey: string | undefined): boolean {
  if (!plan.enabled || lastRunKey === dateKey(now)) return false;
  const due = planTimeMinutes(plan.time);
  if (due === null) return false;
  const nowMin = now.getHours() * 60 + now.getMinutes();
  return nowMin >= due && nowMin - due < RECORD_PLAN_DUE_WINDOW_MIN;
}

/** 日期键："2026-11-24" 这类按本地日期的字符串，用于「同一天只触发一次」。 */
export function dateKey(now: Date): string {
  return `${now.getFullYear()}-${now.getMonth() + 1}-${now.getDate()}`;
}
