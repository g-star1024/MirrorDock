import type { ReactElement, ReactNode } from "react";

/**
 * 爱思风格手机助手视图层（X10-128）。
 *
 * 这个文件只负责「原型同款」的导航结构与通用页面组件，不持有设备/会话状态。
 * 具体数据由 App.tsx 注入，避免把真实状态变成演示数据。
 */

export type AssistantViewKey =
  | "home" | "connect" | "info" | "multi"
  | "mirror" | "present" | "keymap" | "schedule" | "replay"
  | "files" | "shot" | "record" | "album" | "install" | "apps"
  | "notify" | "settings" | "help" | "pro";

export type AssistantModuleKey = "device" | "tools" | "content" | "system";

export type AssistantViewMeta = {
  key: AssistantViewKey;
  title: string;
  subtitle: string;
  action: string;
};

export const ASSISTANT_MODULES: {
  key: AssistantModuleKey;
  label: string;
  group: string;
  view: AssistantViewKey;
}[] = [
  { key: "device", label: "我的设备", group: "设备", view: "home" },
  { key: "tools", label: "智能工具", group: "镜像与会话", view: "mirror" },
  { key: "content", label: "资料管理", group: "内容与传输", view: "files" },
  { key: "system", label: "系统", group: "系统", view: "notify" },
];

export const ASSISTANT_GROUPS: {
  name: string;
  ids: AssistantViewKey[];
  openByDefault: boolean;
}[] = [
  { name: "设备", ids: ["home", "connect", "info", "multi"], openByDefault: true },
  { name: "镜像与会话", ids: ["mirror", "present", "keymap", "schedule", "replay"], openByDefault: true },
  { name: "内容与传输", ids: ["files", "shot", "record", "album", "install", "apps"], openByDefault: true },
  { name: "系统", ids: ["notify", "settings", "help", "pro"], openByDefault: true },
];

export const ASSISTANT_VIEWS: Record<AssistantViewKey, AssistantViewMeta> = {
  home: { key: "home", title: "设备中心", subtitle: "本机设备总览 · 真实连接状态", action: "连接设备" },
  connect: { key: "connect", title: "连接与授权", subtitle: "USB / 无线 ADB / 扫码配对", action: "连接设备" },
  info: { key: "info", title: "设备信息", subtitle: "只读诊断摘要 · 不上传", action: "复制诊断摘要" },
  multi: { key: "multi", title: "多端", subtitle: "当前设备与会话并发状态", action: "刷新设备" },

  mirror: { key: "mirror", title: "镜像控制", subtitle: "画面 · 键鼠直输 · 画质预设", action: "开始镜像" },
  present: { key: "present", title: "投屏演示", subtitle: "就绪检查 · 一键开始演示", action: "开始演示" },
  keymap: { key: "keymap", title: "按键映射", subtitle: "键位映射注入方案 · 本机保存", action: "打开键盘设置" },
  schedule: { key: "schedule", title: "定时录制", subtitle: "本机录制计划 · 应用运行时触发", action: "添加计划" },
  replay: { key: "replay", title: "操作回放", subtitle: "动作序列编排与回放 · 本机保存", action: "开始回放" },

  files: { key: "files", title: "文件传输", subtitle: "手机发送区与电脑之间互传", action: "刷新文件" },
  shot: { key: "shot", title: "截图", subtitle: "保存到本机 · 不自动上传", action: "立即截图" },
  record: { key: "record", title: "录像", subtitle: "手动启停 · 不重启镜像", action: "开始录制" },
  album: { key: "album", title: "相册", subtitle: "相册目录浏览与取回 · 本机保存", action: "刷新相册" },
  install: { key: "install", title: "批量安装", subtitle: "多选 APK · 逐个安装到手机", action: "选择 APK" },
  apps: { key: "apps", title: "应用管理", subtitle: "只读应用清单", action: "刷新清单" },

  notify: { key: "notify", title: "通知中心", subtitle: "实时镜像 · 正文默认不持久化", action: "清空通知" },
  settings: { key: "settings", title: "设置", subtitle: "通用 · 镜像 · 传输 · 隐私", action: "偏好设置" },
  help: { key: "help", title: "帮助中心", subtitle: "默认折叠 · 点击标题展开", action: "展开全部" },
  pro: { key: "pro", title: "Pro / 激活", subtitle: "本机校验 · 不需要账号", action: "输入激活码" },
};

export function assistantModuleForView(view: AssistantViewKey): AssistantModuleKey {
  const group = ASSISTANT_GROUPS.find((item) => item.ids.includes(view));
  return ASSISTANT_MODULES.find((item) => item.group === group?.name)?.key ?? "device";
}

export type AssistantAction = {
  label: string;
  onClick: () => void;
  primary?: boolean;
  danger?: boolean;
  disabled?: boolean;
};

export type AssistantRow = { cells: ReactNode[]; filterText?: string };

export type AssistantSection =
  | { kind: "devices"; title?: string; hint?: string; devices: ReactNode[] }
  | { kind: "tools"; title?: string; hint?: string; tools: ReactNode[] }
  | { kind: "table"; title: string; columns: string[]; rows: AssistantRow[]; tone?: number; empty?: string }
  | { kind: "kv"; title: string; items: [string, ReactNode][]; tone?: number }
  | { kind: "actions"; actions: AssistantAction[]; tone?: number }
  | { kind: "note"; text: string; tone?: number }
  | { kind: "steps"; title: string; items: string[]; tone?: number }
  | { kind: "details"; title: string; items: [string, string][]; tone?: number }
  | { kind: "phone"; title: string; tone?: number; children?: ReactNode }
  | { kind: "custom"; children: ReactNode };

function toneClass(tone: number | undefined): string {
  return `assistant-tone-${tone ?? 1}`;
}

function AssistantSectionView({ section, filter }: { section: AssistantSection; filter: string }) {
  const query = filter.trim().toLowerCase();
  switch (section.kind) {
    case "devices":
      return (
        <section className="assistant-block">
          <div className="assistant-block-head">
            <h3>{section.title ?? "设备列表"}</h3>
            {section.hint && <span>{section.hint}</span>}
          </div>
          <div className="assistant-device-grid">{section.devices}</div>
        </section>
      );
    case "tools":
      return (
        <section className="assistant-block">
          <div className="assistant-block-head">
            <h3>{section.title ?? "常用工具"}</h3>
            {section.hint && <span>{section.hint}</span>}
          </div>
          <div className="assistant-tool-grid">{section.tools}</div>
        </section>
      );
    case "table": {
      const rows = query
        ? section.rows.filter((row) => (row.filterText ?? "").toLowerCase().includes(query))
        : section.rows;
      return (
        <section className={`assistant-card ${toneClass(section.tone)}`}>
          <h4>{section.title}</h4>
          <div className="assistant-table-wrap" role="region" aria-label={section.title} tabIndex={0}>
            <table className="assistant-table">
              <thead>
                <tr>{section.columns.map((column) => <th scope="col" key={column}>{column}</th>)}</tr>
              </thead>
              <tbody>
                {rows.length === 0 ? (
                  <tr><td colSpan={section.columns.length}>{section.empty ?? "没有匹配内容"}</td></tr>
                ) : rows.map((row, index) => (
                  <tr key={index}>{row.cells.map((cell, cellIndex) => (
                    cellIndex === 0 ? <th scope="row" key={cellIndex}>{cell}</th> : <td key={cellIndex}>{cell}</td>
                  ))}</tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      );
    }
    case "kv":
      return (
        <section className={`assistant-card ${toneClass(section.tone)}`}>
          <h4>{section.title}</h4>
          <div className="assistant-kv-grid">
            {section.items.map(([key, value], index) => (
              <div className="assistant-kv" key={`${key}-${index}`}>
                <b>{key}</b><span>{value}</span>
              </div>
            ))}
          </div>
        </section>
      );
    case "actions":
      return (
        <div className="assistant-action-row">
          {section.actions.map((action) => (
            <button
              key={action.label}
              type="button"
              className={`assistant-btn${action.primary ? " primary" : ""}${action.danger ? " danger" : ""}`}
              disabled={action.disabled}
              onClick={action.onClick}
            >
              {action.label}
            </button>
          ))}
        </div>
      );
    case "note":
      return <p className="assistant-note">{section.text}</p>;
    case "steps":
      return (
        <section className={`assistant-card ${toneClass(section.tone)}`}>
          <h4>{section.title}</h4>
          <ol className="assistant-steps">
            {section.items.map((item) => <li key={item}>{item}</li>)}
          </ol>
        </section>
      );
    case "details":
      return (
        <section className={`assistant-card ${toneClass(section.tone)}`}>
          <h4>{section.title}</h4>
          <div className="assistant-details">
            {section.items.map(([question, answer]) => (
              <details key={question}>
                <summary>{question}</summary>
                <p>{answer}</p>
              </details>
            ))}
          </div>
        </section>
      );
    case "phone":
      return (
        <section className={`assistant-card ${toneClass(section.tone)}`}>
          <h4>{section.title}</h4>
          <div className="assistant-phone-wrap">{section.children ?? <span className="assistant-muted">没有可显示的实时画面</span>}</div>
        </section>
      );
    case "custom":
      return <>{section.children}</>;
    default:
      return null;
  }
}

export function AssistantViewPage({
  view,
  sections,
  filter,
}: {
  view: AssistantViewKey;
  sections: AssistantSection[];
  filter: string;
}): ReactElement {
  const meta = ASSISTANT_VIEWS[view];
  return (
    <section className="assistant-view" data-view={view} aria-label={meta.title}>
      <div className="assistant-view-head">
        <div>
          <h3>{meta.title}</h3>
          <p>{meta.subtitle}</p>
        </div>
      </div>
      <div className="assistant-view-stack">
        {sections.map((section, index) => (
          <AssistantSectionView key={`${view}-${index}`} section={section} filter={filter} />
        ))}
      </div>
    </section>
  );
}
