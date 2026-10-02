import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { helpArticles } from "./helpContent";

// 帮助文档是纯内容层：这里守住结构不变量与「寻求帮助」条目的关键承诺，
// 防止后续改动把支持分流的关键信息（诊断包、Issue 渠道、防钓鱼提示）改丢。
describe("helpContent", () => {
  it("every_article_has_unique_id_title_and_nonempty_paragraphs", () => {
    const ids = new Set<string>();
    for (const article of helpArticles) {
      expect(article.id.length).toBeGreaterThan(0);
      expect(ids.has(article.id)).toBe(false);
      ids.add(article.id);
      expect(article.title.length).toBeGreaterThan(0);
      expect(article.paragraphs.length).toBeGreaterThan(0);
      for (const paragraph of article.paragraphs) {
        expect(paragraph.trim().length).toBeGreaterThan(0);
      }
    }
  });

  it("getting_help_article_covers_diagnostics_issue_channel_and_anti_phishing", () => {
    const article = helpArticles.find((entry) => entry.id === "getting-help");
    expect(article).toBeDefined();
    const text = article?.paragraphs.join("\n") ?? "";
    // 支持分流的三个关键点：先导出诊断包、到 GitHub 仓库提反馈、提醒不交出敏感内容。
    expect(text).toContain("诊断包");
    expect(text).toContain("GitHub 仓库");
    expect(text).toContain("g-star1024/MirrorDock");
    expect(text).toContain("请提高警惕");
    // 与脱敏承诺一致：帮助文案必须如实说明诊断包不含序列号/配对码/路径。
    expect(text).toContain("抹去");
  });

  it("faq_explains_companion_one_tap_reconnect_needs_resident_enabled_first", () => {
    const faq = helpArticles.find((entry) => entry.id === "faq");
    expect(faq).toBeDefined();
    const text = faq?.paragraphs.join("\n") ?? "";
    // X10-64：常驻端口只在扫码那一刻交给手机，顺序反了会走进「没有可直连的
    // 电脑」的死胡同。帮助文案必须给出顺序要求与补救动作（重新扫码），
    // 并且不能出现「不必开启」这类会让用户踩坑的说法。
    expect(text).toContain("常驻通道");
    expect(text).toContain("先开启");
    expect(text).toContain("重新扫一次码");
    expect(text).not.toContain("扫码配对时无需开启");
  });

  it("faq_explains_recovery_for_both_the_data_cable_and_wireless_drops", () => {
    const faq = helpArticles.find((entry) => entry.id === "faq");
    expect(faq).toBeDefined();
    const text = faq?.paragraphs.join("\n") ?? "";
    // X10-59 起「断线自动重连」同时覆盖数据线与无线；帮助文案两种都要说清，
    // 并指明可在设置里关闭——否则用户既不知道拔线也会恢复，也找不到开关。
    expect(text).toContain("断线自动重连");
    expect(text).toContain("数据线");
    expect(text).toContain("无线");
    expect(text).toContain("设置 → 通用");
  });

  it("tools_article_covers_drag_routing_and_send_zone_management", () => {
    const article = helpArticles.find((entry) => entry.id === "tools");
    expect(article).toBeDefined();
    const text = article?.paragraphs.join("\n") ?? "";
    // X10-63 / X10-60：拖拽会把 .apk 送去安装、其他文件送去手机发送区；发送区可
    // 查看、取回、逐条删除。这两条是新功能里最容易被问到的路径，必须写清。
    expect(text).toContain("拖到");
    expect(text).toContain("发送区");
    expect(text).toContain("取回到电脑");
    expect(text).toContain("删除");
    // 反向通道（伴侣 App 发送文件到电脑）也要说清，否则手机侧入口在帮助里无迹可寻。
    expect(text).toContain("发送文件到电脑");
  });

  it("notifications_article_states_prerequisites_privacy_and_off_switch", () => {
    const article = helpArticles.find((entry) => entry.id === "notifications");
    expect(article).toBeDefined();
    const text = article?.paragraphs.join("\n") ?? "";
    // X10-66：开关在手机端、需系统「读取通知」授权；内容仅内存、不入日志、不上云；
    // 并如实给出「离线不补发」与常驻通知不转发的边界，以及停止方式。
    expect(text).toContain("读取通知");
    expect(text).toContain("清空");
    expect(text).toContain("不经过任何云端");
    expect(text).toContain("不会补发");
    expect(text).toContain("通知镜像");
  });

  it("readme_declares_the_same_help_article_count_as_shipped", () => {
    // README 对外声明帮助文档篇数；新增/删除文章时必须同步，否则等于向用户报了
    // 一个错数字（历史缺口：README 写 8 篇、实际 9 篇，X10-67 修正为 10 篇）。
    // 注意：jsdom 环境下 import.meta.url 是 http:// 地址，须按工作目录解析真实路径。
    const readme = readFileSync(resolve(process.cwd(), "README.md"), "utf8");
    const match = readme.match(/(\d+)\s*篇完整帮助文档/);
    expect(match).not.toBeNull();
    expect(Number(match?.[1])).toBe(helpArticles.length);
  });
});
