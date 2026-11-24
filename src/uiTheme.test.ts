import { beforeEach, describe, expect, it } from "vitest";
import {
  UI_THEME_STORAGE_KEY,
  UI_THEMES,
  applyUiTheme,
  isUiThemeKey,
  readUiTheme,
  themeMeta,
  writeUiTheme,
} from "./uiTheme";

describe("ui theme preference", () => {
  beforeEach(() => {
    localStorage.clear();
    delete document.documentElement.dataset.theme;
    document.documentElement.style.colorScheme = "";
    document.querySelector('meta[name="color-scheme"]')?.setAttribute("content", "light dark");
  });

  it("falls back to the default theme for missing or unknown values", () => {
    expect(readUiTheme()).toBe("a");
    localStorage.setItem(UI_THEME_STORAGE_KEY, "unknown");
    expect(readUiTheme()).toBe("a");
    expect(isUiThemeKey("d")).toBe(true);
    expect(isUiThemeKey("z")).toBe(false);
  });

  it("round-trips every theme through localStorage", () => {
    for (const theme of UI_THEMES) {
      writeUiTheme(theme.key);
      expect(readUiTheme()).toBe(theme.key);
    }
  });

  it("keeps theme metadata unique and marks only the dark console as dark", () => {
    expect(new Set(UI_THEMES.map((theme) => theme.key)).size).toBe(UI_THEMES.length);
    expect(UI_THEMES.filter((theme) => theme.colorScheme === "dark").map((theme) => theme.key)).toEqual(["b"]);
    expect(themeMeta("d").label).toBe("极简留白");
  });

  it("reflects the selected theme on the document root", () => {
    applyUiTheme("b");
    expect(document.documentElement.dataset.theme).toBe("b");
    expect(document.documentElement.style.colorScheme).toBe("dark");
    expect(document.querySelector('meta[name="color-scheme"]')?.getAttribute("content")).toBe("dark");
    applyUiTheme("a");
    expect(document.documentElement.dataset.theme).toBe("a");
    expect(document.documentElement.style.colorScheme).toBe("light");
  });
});
