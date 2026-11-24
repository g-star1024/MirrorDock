// 桌面端外观主题（X10-127）：只保存本机视觉偏好，不涉及云同步、账号或设备数据。
// 默认跟随系统的浅色/深色倾向；用户一旦在设置里选定，就持久化到 localStorage。
export type UiThemeKey = "a" | "b" | "c" | "d" | "e";

export type UiThemeMeta = {
  key: UiThemeKey;
  label: string;
  description: string;
  colorScheme: "light" | "dark";
};

export const UI_THEME_STORAGE_KEY = "mirrordock.uiTheme";

export const UI_THEMES: readonly UiThemeMeta[] = [
  { key: "a", label: "云鲸桌台", description: "柔和浅色，大圆角轻阴影", colorScheme: "light" },
  { key: "b", label: "深空控制台", description: "深色紧凑，等宽数据感", colorScheme: "dark" },
  { key: "c", label: "彩色磁贴", description: "彩色语义磁贴，最像手机助手", colorScheme: "light" },
  { key: "d", label: "极简留白", description: "细线无阴影，内容优先", colorScheme: "light" },
  { key: "e", label: "玻璃 HUD", description: "毛玻璃，渐变光斑", colorScheme: "light" },
] as const;

export function isUiThemeKey(value: unknown): value is UiThemeKey {
  return UI_THEMES.some((theme) => theme.key === value);
}

/**
 * 读回本机主题偏好。任何损坏/过期值都回落到默认，不让设置页崩溃。
 * localStorage 不可用（隐私模式、存储被禁用）时同样安全返回默认。
 */
export function readUiTheme(): UiThemeKey {
  try {
    const raw = localStorage.getItem(UI_THEME_STORAGE_KEY);
    return isUiThemeKey(raw) ? raw : "a";
  } catch {
    return "a";
  }
}

export function writeUiTheme(value: UiThemeKey): void {
  try {
    localStorage.setItem(UI_THEME_STORAGE_KEY, value);
  } catch {
    // 存储不可用时本次会话仍生效，不打断用户。
  }
}

export function themeMeta(key: UiThemeKey): UiThemeMeta {
  return UI_THEMES.find((theme) => theme.key === key) ?? UI_THEMES[0];
}

/**
 * 把主题写回文档根节点：CSS 变量由根节点的 data-theme 驱动；
 * color-scheme 只影响原生控件与滚动条，不覆盖作者样式。
 */
export function applyUiTheme(key: UiThemeKey): void {
  const meta = themeMeta(key);
  document.documentElement.dataset.theme = key;
  document.documentElement.style.colorScheme = meta.colorScheme;
  let schemeMeta = document.querySelector('meta[name="color-scheme"]');
  if (!schemeMeta) {
    schemeMeta = document.createElement("meta");
    schemeMeta.setAttribute("name", "color-scheme");
    document.head.appendChild(schemeMeta);
  }
  schemeMeta.setAttribute("content", meta.colorScheme);
}
