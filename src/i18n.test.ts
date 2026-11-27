import { beforeEach, describe, expect, it } from "vitest";
import {
  applyLocale,
  readLocale,
  translateText,
  writeLocale,
} from "./i18n";

describe("interface language", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.lang = "";
    document.body.innerHTML = "";
  });

  it("defaults to Chinese and persists an explicit choice", () => {
    expect(readLocale()).toBe("zh");
    writeLocale("en");
    expect(readLocale()).toBe("en");
    expect(document.documentElement.lang).toBe("en");
    writeLocale("zh");
    expect(readLocale()).toBe("zh");
    expect(document.documentElement.lang).toBe("zh-CN");
  });

  it("translates exact UI strings but keeps unknown strings unchanged", () => {
    expect(translateText("外观主题")).toBe("Appearance");
    expect(translateText("按品牌查看开启步骤")).toBe("Enable steps by brand");
    expect(translateText("Redmi 手机")).toBe("Redmi 手机");
  });

  it("never rewrites user data that merely contains a common Chinese word", () => {
    expect(translateText("我的小米设备")).toBe("我的小米设备");
    expect(translateText("照片/设置/备份")).toBe("照片/设置/备份");
  });

  it("handles templates with interpolated values", () => {
    expect(translateText("当前版本：0.4.38（测试版）")).toBe(
      "Current version: 0.4.38 (beta)",
    );
    expect(translateText("当前：清晨主题")).toBe("Current: 清晨主题");
  });

  it("switches rendered text and accessible names, then restores them", () => {
    const root = document.createElement("div");
    root.innerHTML = '<p>外观主题</p><button aria-label="重新检查">重新检查</button>';
    document.body.appendChild(root);

    expect(applyLocale(root, "en")).toBeGreaterThanOrEqual(3);
    expect(root.querySelector("p")?.textContent).toBe("Appearance");
    expect(root.querySelector("button")?.textContent).toBe("Recheck");
    expect(root.querySelector("button")?.getAttribute("aria-label")).toBe("Recheck");

    applyLocale(root, "zh");
    expect(root.querySelector("p")?.textContent).toBe("外观主题");
    expect(root.querySelector("button")?.textContent).toBe("重新检查");
    expect(root.querySelector("button")?.getAttribute("aria-label")).toBe("重新检查");
  });
});
