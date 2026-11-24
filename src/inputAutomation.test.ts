import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  INJECTABLE_KEYCODES,
  clampWaitMs,
  describeAction,
  describeStep,
  isValidCoord,
  isValidDuration,
  isValidKeycode,
  newLocalId,
  normalizePhysicalKey,
  physicalKeyLabel,
  readKeymaps,
  readRecordPlans,
  readReplays,
  validateReplayScript,
  writeKeymaps,
  writeRecordPlans,
  writeReplays,
  dateKey,
  isRecordPlanDue,
  planTimeMinutes,
  parseRecordPlans,
  isValidRecordPlanDuration,
  type KeymapAction,
  type KeymapProfile,
  type ReplayScript,
} from "./inputAutomation";

describe("injectable keycodes", () => {
  it("keeps_the_whitelist_unique_and_uppercase", () => {
    const values = INJECTABLE_KEYCODES.map((item) => item.value);
    expect(new Set(values).size).toBe(values.length);
    for (const value of values) {
      expect(value).toMatch(/^KEYCODE_[A-Z0-9_]+$/);
    }
  });

  it("accepts_only_whitelisted_keycodes", () => {
    expect(isValidKeycode("KEYCODE_BACK")).toBe(true);
    expect(isValidKeycode("keycode_back")).toBe(false);
    expect(isValidKeycode("KEYCODE_BACK; shutdown")).toBe(false);
    expect(isValidKeycode("")).toBe(false);
  });
});

describe("coordinate and duration validation", () => {
  it("accepts_integer_coords_within_0_9999_only", () => {
    expect(isValidCoord(0)).toBe(true);
    expect(isValidCoord(9999)).toBe(true);
    expect(isValidCoord(-1)).toBe(false);
    expect(isValidCoord(10_000)).toBe(false);
    expect(isValidCoord(1.5)).toBe(false);
  });

  it("accepts_durations_within_50_60000_only", () => {
    expect(isValidDuration(50)).toBe(true);
    expect(isValidDuration(60000)).toBe(true);
    expect(isValidDuration(49)).toBe(false);
    expect(isValidDuration(60001)).toBe(false);
    expect(isValidDuration(0)).toBe(false);
  });

  it("clamps_wait_into_0_60000", () => {
    expect(clampWaitMs(-5)).toBe(0);
    expect(clampWaitMs(123.6)).toBe(124);
    expect(clampWaitMs(99_999)).toBe(60000);
    expect(clampWaitMs(Number.NaN)).toBe(0);
  });
});

describe("physical key capture", () => {
  it("normalizes_letter_and_digit_keys_to_primary_positions", () => {
    expect(normalizePhysicalKey({ code: "KeyW", key: "w" })).toBe("w");
    expect(normalizePhysicalKey({ code: "KeyW", key: "W" })).toBe("w");
    expect(normalizePhysicalKey({ code: "Digit3", key: "3" })).toBe("3");
  });

  it("keeps_known_special_keys_and_rejects_ime_composites", () => {
    expect(normalizePhysicalKey({ code: "ArrowUp", key: "ArrowUp" })).toBe("ArrowUp");
    expect(normalizePhysicalKey({ code: "Space", key: " " })).toBe("Space");
    expect(normalizePhysicalKey({ key: "组" })).toBeNull();
    expect(normalizePhysicalKey({ code: "SomeWeirdCode", key: "x" })).toBeNull();
  });

  it("renders_readable_labels_for_labels_and_letters", () => {
    expect(physicalKeyLabel("w")).toBe("W");
    expect(physicalKeyLabel("ArrowUp")).toBe("↑");
    expect(physicalKeyLabel("Space")).toBe("空格");
    expect(physicalKeyLabel("MediaKeys")).toBe("MediaKeys");
  });
});

describe("descriptions", () => {
  it("describes_actions_and_steps_in_readable_chinese", () => {
    const key: KeymapAction = { kind: "key", keycode: "KEYCODE_BACK" };
    const tap: KeymapAction = { kind: "tap", x: 540, y: 1200 };
    const swipe: KeymapAction = { kind: "swipe", x1: 540, y1: 1600, x2: 540, y2: 400, duration: 300 };
    expect(describeAction(key)).toContain("返回");
    expect(describeAction(tap)).toBe("点击 (540, 1200)");
    expect(describeAction(swipe)).toContain("· 300ms");
    expect(describeStep({ ...tap, waitMs: 500 })).toContain("前等 500ms");
    expect(describeStep({ ...tap, waitMs: 0 })).not.toContain("前等");
  });
});

describe("replay script validation", () => {
  it("rejects_empty_or_oversized_scripts", () => {
    expect(validateReplayScript([])).toContain("还没有任何步骤");
    const oversized: ReplayScript["steps"] = Array.from({ length: 501 }, () => ({ kind: "tap", x: 1, y: 1, waitMs: 0 }));
    expect(validateReplayScript(oversized)).toContain("最多 500 步");
  });

  it("points_out_the_first_broken_step", () => {
    expect(validateReplayScript([{ kind: "key", keycode: "NOT_A_KEY", waitMs: 0 }])).toContain("第 1 步");
    expect(validateReplayScript([
      { kind: "tap", x: 1, y: 1, waitMs: 0 },
      { kind: "tap", x: -1, y: 1, waitMs: 0 },
    ])).toContain("第 2 步");
    expect(validateReplayScript([{ kind: "swipe", x1: 0, y1: 0, x2: 100, y2: 100, duration: 10, waitMs: 0 }])).toContain("第 1 步");
  });

  it("accepts_a_valid_script", () => {
    expect(validateReplayScript([
      { kind: "tap", x: 540, y: 1200, waitMs: 300 },
      { kind: "key", keycode: "KEYCODE_HOME", waitMs: 0 },
      { kind: "swipe", x1: 540, y1: 1600, x2: 540, y2: 400, duration: 250, waitMs: 1000 },
    ])).toBeNull();
  });
});

describe("local persistence", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  it("round_trips_keymap_profiles_and_drops_broken_bindings", () => {
    const profile: KeymapProfile = {
      id: "p1",
      name: "游戏",
      bindings: [
        { physicalKey: "w", label: "W", action: { kind: "swipe", x1: 540, y1: 1600, x2: 540, y2: 400, duration: 300 } },
        // 这条会被丢弃：按键码不在白名单。
        { physicalKey: "x", label: "X", action: { kind: "key", keycode: "NOT_A_KEY" } },
      ],
    };
    writeKeymaps([profile], "p1");
    const restored = readKeymaps();
    expect(restored.activeId).toBe("p1");
    expect(restored.profiles).toHaveLength(1);
    expect(restored.profiles[0].bindings).toHaveLength(1);
    expect(restored.profiles[0].bindings[0].physicalKey).toBe("w");
  });

  it("falls_back_to_empty_collections_on_broken_json", () => {
    window.localStorage.setItem("mirrordock.keymaps", "{not-json");
    window.localStorage.setItem("mirrordock.replays", "42");
    expect(readKeymaps()).toEqual({ profiles: [], activeId: null });
    expect(readReplays()).toEqual([]);
  });

  it("ignores_an_active_id_that_no_longer_exists", () => {
    writeKeymaps([{ id: "p1", name: "方案 1", bindings: [] }], "ghost");
    expect(readKeymaps().activeId).toBeNull();
  });

  it("round_trips_replay_scripts_and_clamps_wait", () => {
    const script: ReplayScript = {
      id: "s1",
      name: "签到",
      steps: [
        { kind: "tap", x: 100, y: 200, waitMs: 500 },
        { kind: "swipe", x1: 0, y1: 0, x2: 300, y2: 400, duration: 250, waitMs: 99_999 },
      ],
    };
    writeReplays([script]);
    const restored = readReplays();
    expect(restored).toHaveLength(1);
    expect(restored[0].steps[0]).toEqual({ kind: "tap", x: 100, y: 200, waitMs: 500 });
    expect(restored[0].steps[1].waitMs).toBe(60000);
  });

  it("generates_unique_ids", () => {
    const ids = new Set(Array.from({ length: 50 }, () => newLocalId()));
    expect(ids.size).toBe(50);
  });
});

describe("record plans (X10-131)", () => {
  const basePlan = { id: "r1", name: "晨会", time: "09:30", durationMin: 15, enabled: true };

  it("parses_and_drops_invalid_plans", () => {
    const plans = parseRecordPlans([
      basePlan,
      { id: "", name: "无 id", time: "09:30", durationMin: 10, enabled: true },
      { id: "r2", name: "坏时间", time: "25:00", durationMin: 10, enabled: true },
      { id: "r3", name: "坏时长", time: "09:30", durationMin: 0, enabled: true },
      { id: "r4", name: "无开关", time: "09:30", durationMin: 10 },
      "not-an-object",
    ]);
    expect(plans).toEqual([basePlan]);
  });

  it("round_trips_plans_through_local_storage", () => {
    writeRecordPlans([basePlan, { ...basePlan, id: "r2", name: "下班", time: "18:00", durationMin: 30, enabled: false }]);
    expect(readRecordPlans()).toEqual([
      basePlan,
      { id: "r2", name: "下班", time: "18:00", durationMin: 30, enabled: false },
    ]);
    expect(readRecordPlans()).not.toEqual([basePlan]);
  });

  it("bounds_plan_duration", () => {
    expect(isValidRecordPlanDuration(1)).toBe(true);
    expect(isValidRecordPlanDuration(240)).toBe(true);
    expect(isValidRecordPlanDuration(241)).toBe(false);
    expect(isValidRecordPlanDuration(0)).toBe(false);
    expect(isValidRecordPlanDuration(15.5)).toBe(false);
  });

  it("converts_time_to_minutes", () => {
    expect(planTimeMinutes("09:30")).toBe(570);
    expect(planTimeMinutes("00:00")).toBe(0);
    expect(planTimeMinutes("23:59")).toBe(1439);
    expect(planTimeMinutes("24:00")).toBeNull();
  });

  it("triggers_only_within_the_due_window_and_once_per_day", () => {
    const due = new Date(2026, 10, 24, 9, 31); // 09:31，计划 09:30 → 窗口内
    expect(isRecordPlanDue(basePlan, due, undefined)).toBe(true);
    // 同一天已触发过 → 不再触发
    expect(isRecordPlanDue(basePlan, due, "2026-11-24")).toBe(false);
    // 窗口外（提前 10 分钟 / 迟到 6 分钟）→ 不触发
    expect(isRecordPlanDue(basePlan, new Date(2026, 10, 24, 9, 20), undefined)).toBe(false);
    expect(isRecordPlanDue(basePlan, new Date(2026, 10, 24, 9, 36), undefined)).toBe(false);
    // 停用的计划永不触发
    expect(isRecordPlanDue({ ...basePlan, enabled: false }, due, undefined)).toBe(false);
  });

  it("uses_local_date_keys", () => {
    expect(dateKey(new Date(2026, 10, 24))).toBe("2026-11-24");
    expect(dateKey(new Date(2026, 0, 3))).toBe("2026-1-3");
  });
});
