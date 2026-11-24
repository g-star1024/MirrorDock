import { describe, expect, it } from "vitest";
import {
  ASSISTANT_GROUPS,
  ASSISTANT_MODULES,
  ASSISTANT_VIEWS,
  assistantModuleForView,
  type AssistantViewKey,
} from "./assistantViews";

describe("assistant view map (X10-128)", () => {
  it("exposes all 19 views from the i4-style prototype", () => {
    const keys = Object.keys(ASSISTANT_VIEWS) as AssistantViewKey[];
    expect(keys).toHaveLength(19);
    expect(new Set(keys).size).toBe(19);
    for (const key of keys) {
      expect(ASSISTANT_VIEWS[key].title).toBeTruthy();
      expect(ASSISTANT_VIEWS[key].subtitle).toBeTruthy();
    }
  });

  it("groups every view under exactly one of four modules", () => {
    const grouped = ASSISTANT_GROUPS.flatMap((group) => group.ids);
    expect(grouped).toHaveLength(19);
    expect(new Set(grouped).size).toBe(19);
    expect(ASSISTANT_MODULES).toHaveLength(4);

    for (const group of ASSISTANT_GROUPS) {
      for (const id of group.ids) {
        const module = ASSISTANT_MODULES.find((item) => item.group === group.name);
        expect(assistantModuleForView(id), id).toBe(module?.key);
      }
    }
  });

  it("keeps the prototype module entry views reachable", () => {
    expect(ASSISTANT_MODULES.map((module) => module.view)).toEqual(["home", "mirror", "files", "notify"]);
  });
});
