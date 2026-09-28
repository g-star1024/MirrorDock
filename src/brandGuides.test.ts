// 品牌识别的知识库测试：识别正确、识别不出要如实返回 null，
// 关键词命中顺序不得让后注册的品牌抢先。
import { describe, expect, it } from "vitest";
import { brandGuides, detectBrand } from "./brandGuides";

describe("detectBrand", () => {
  it("recognizes_each_supported_brand_from_a_typical_device_label", () => {
    const cases: [string[], string][] = [
      [["Google", "Pixel 8"], "pixel"],
      [["samsung", "SM-S9210"], "samsung"],
      [["Xiaomi", "M2104K10AC"], "xiaomi"],
      [["Redmi", "K60"], "xiaomi"],
      [["OPPO", "Find X7"], "oppo"],
      [["realme", "真我 GT5"], "oppo"],
      [["vivo", "iQOO 12"], "vivo"],
      [["OnePlus", "一加 Ace 3"], "oneplus"],
    ];
    for (const [labels, expectedKey] of cases) {
      expect(detectBrand(labels)?.key, `labels: ${labels.join("/")}`).toBe(expectedKey);
    }
  });

  it("matches_partial_keywords_inside_a_longer_model_name", () => {
    // SM- 前缀即使厂商字段缺失也能识别三星。
    expect(detectBrand(["SM-G991B"])?.key).toBe("samsung");
    // POCO 归入小米知识库。
    expect(detectBrand(["POCO F5"])?.key).toBe("xiaomi");
  });

  it("combines_manufacturer_and_model_labels_into_one_search_space", () => {
    expect(detectBrand(["Xiaomi", "23049PCD8G"])?.key).toBe("xiaomi");
  });

  it("returns_null_for_unknown_brands_instead_of_guessing", () => {
    expect(detectBrand(["ZTE 中兴 Axon 60"])).toBeNull();
    expect(detectBrand([])).toBeNull();
    expect(detectBrand(["unknown-device-123"])).toBeNull();
  });

  it("keeps_the_knowledge_base_nonempty_and_honest", () => {
    // 每个品牌都必须带"以实际设置为准"性质的注意点或提示结构，
    // 防止未来有人把绝对化的路径写进知识库却没有任何免责说明。
    expect(brandGuides.length).toBeGreaterThanOrEqual(6);
    for (const guide of brandGuides) {
      expect(guide.name.length).toBeGreaterThan(0);
      expect(guide.openDeveloperOptions).toContain("设置");
      expect(guide.usbDebugging).toContain("USB 调试");
      expect(guide.notes.length).toBeGreaterThan(0);
    }
  });
});
