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
});
