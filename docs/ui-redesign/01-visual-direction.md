# MirrorDock UI 重设计 · Phase 1 视觉方向

> 目标：让客户端从「爱思助手风格的高密度工具壳」升级为 2025–26 现代桌面应用气质（Linear / Raycast / Things / CleanMyMac / Arc 一脉）。
> 本文档具体到前端可以直接照抄实现单文件 HTML 预览。所有 HEX / 数值均为推荐值，可微调但**不得偏离对比度与层级意图**。

---

## 1. 现状视觉诊断

依据 `pc-audit/` 下 19 张 1440px 截图 + `src/App.css` 现行代码。

### 1.1 全局（所有主题、所有视图共有）

| # | 问题 | 证据 |
|---|------|------|
| G1 | **标题栏高饱和蓝色渐变**（`--top-1/2/3` 三段渐变 + 白色 inset 高光），视觉重量占了整屏顶部，且与浅蓝内容区色彩互相竞争。这是典型的 2018 年国产工具壳气质 | home/mirror/apps 等全部截图顶部 |
| G2 | **标题重复 3 次**：subbar 面包屑「我的设备 / 设备中心」→ work-head「设备中心 · 本机设备总览 · 真实连接状态」→ 卡片标题「设备中心」+ 卡片副标题再抄一遍。信息噪音大 | info-1440、mirror-1440、album-1440 尤其明显 |
| G3 | **右侧 236px 栏在 19 个视图里内容完全一样**（设备状态 / 常用工具 / APK 快速安装 / 最近任务 / 安全边界），且与主区内容大量重复：设备状态重复 kv 值、APK 快速安装重复「批量安装」视图入口。1000px 窗口下挤压主区 | 所有截图右栏 |
| G4 | **顶栏一级模块导航与侧栏分组重复**：「我的设备/智能工具/资料管理/系统」与侧栏的「设备/镜像与会话/内容与传输/系统」是同一信息的两套入口 | 所有截图 |
| G5 | **字号阶梯失控**：实测存在 8px（brand small）、9.5px（rail-tools）、10.5px、11px、11.5px、12、12.5、13、13.5、14、14.5、15、15.5、16、19、24px 共 16 档。表格 11.5px、状态条 11px 过小 | App.css 全文；settings-1440 |
| G6 | **卡片彩色左边框标题条 × 8 种色相**（blue/teal/purple/amber/green/cyan/pink/gray），页面色彩噪音大，彩色只承担「装饰」而非「语义」 | mirror-1440（青）、schedule-1440（黄）、apps-1440 |
| G7 | **按钮全彩**：primary 蓝、secondary 蓝软底、danger 红软底、ghost 又是另一种；页面同一屏 4–6 个彩色动作按钮，没有「中性次级」概念 | install-1440、shot-1440 |
| G8 | **说明横幅泛滥**：每页 1–3 条淡蓝/淡黄全宽 banner（左 3px 边框），提示文字与错误混排，错误信息用淡黄底看起来像提示 | home-1440（三条黄色错误横幅）、connect-1440 |
| G9 | **空态裸奔**：表格渲染出表头但 body 只有一句话「尚未读取应用清单」，无图标、无主 CTA、无下一步指引；「真实的空态」原则完全没被设计 | apps-1440、files-1440、notify-1440 |
| G10 | **大量留白断层**：内容 max-width 1020px 居中，多数视图只填满屏幕上 1/3，下 2/3 纯背景色，稀疏感与「高密度工具」定位矛盾 | mirror/present/keymap/replay/record/shot |
| G11 | **圆角失控**：代码里同时存在 4/5/6/7/8/9/10/12/13/14/16/18px 共 12 档圆角 | App.css 全文 |
| G12 | **主题切换只覆盖部分变量**：theme b 用等宽字体当正文（gimmick）；theme c 蓝渐变标题栏 + 紫色强调撞色；theme d 强调色=正常蓝但品牌块变黑；5 套主题没有一套有完整自洽的 token 体系 | App.css L465–526 |

### 1.2 逐主题

- **a 云鲸桌台（浅色蓝）**：问题最重的一套。渐变标题栏 + 蓝色一切（G1/G7），浅蓝 banner 层层叠叠（G8）。
- **b 深空控制台（深色）**：`font-family: var(--mono)` 13px 当全局字体是反功能设计——中文等宽渲染极差；深空表面偏蓝紫（#181f2c）不够中性；深色只有这一套且对比度未校准。
- **c 彩色磁贴**：紫 accent（#7c3aed）与蓝渐变标题栏同屏，tool icon 又是蓝紫渐变——三个色相打架；说是「彩色磁贴」实际只是换了个 accent。
- **d 极简留白**：方向最接近正确，但只有 shadow:none 一个差异，层级靠不上；brand-mark 变黑块突兀。
- **e 玻璃 HUD**：blur(18px)+saturate(1.35) 铺满所有容器性能重；radial 光斑两处 + 三段渐变背景，装饰过载；白色边框在浅背景上失效（`--line: rgba(255,255,255,.72)` 在卡片内不可见）。

### 1.3 逐区域

- **标题栏**：品牌 tagline「ANDROID DEVICE ASSISTANT」8px 大字距（G5）；150px 搜索框放标题栏；「主题·玻璃 HUD」「帮助」白描边按钮像网页顶栏不像桌面客户端。
- **subbar（40px）**：面包屑 + 设备 pill + 右侧「连接设备/重新检查」，与 G2/G4 重复；40px 高度可并入。
- **侧栏（220px）**：分组头 10.5px 字距 0.08em 可读性差；active 态 `accent-soft + inset 2px accent` 边框，与 hover 态几乎无区分；侧栏底部版权小字 10.5px 堆在角落。
- **工作区**：work-head sticky + 卡片双层结构（G2）；表格 sticky 表头 11.5px；空表（G9）。
- **右栏（236px）**：G3 整体保留价值低；rail-tools 9.5px 标签不可读。
- **状态条（28px）**：11px 灰字「本地连接 · 无云中继 · 版本 —」可以保留但需要字号/对比度归入统一阶梯。

---

## 2. 设计方向陈述

> **「安静的诊断台」（Quiet Instrument）**：像 Linear 一样用中性表面与发丝线构建层级，像 Raycast 一样让强调色只在「可执行的动作」上出现，像 Things 一样用字重与留白而非颜色来分组，像 CleanMyMac 一样给每个空态、加载态、错误态以完整的视觉回答，像 Arc 一样只保留一处克制的大胆（每套主题一个独立的「气质锚点」）。去掉渐变标题栏与彩色装饰边框，把色彩预算全部花在**状态语义**（运行中/成功/警告/错误）与**主行动**上——这正是「本地优先、数据不出本机」工具该有的可靠感：冷静、精密、不表演。

参考应用的具体移植点：
- **Linear**：中性色面板 + hairline 边框 + 1–2px 阴影；紧凑表格；chips/徽章小而轻。
- **Raycast**：命令搜索置顶（标题栏中央）；按钮小圆角、强调色克制；暗色主题作为一等公民。
- **Things**：字号阶梯极简（正文一档、标题一档）；分组靠留白与字重，不靠彩色分隔条。
- **CleanMyMac**：空态有插画感的图形位 + 明确的下一步主按钮；状态图形化。
- **Arc**：每套主题保留一个「签名瞬间」（如玻璃主题的光斑、留白主题的纯排印），其余部分严格统一。

---

## 3. 完整 Design Tokens

### 3.1 共享基座（全部主题共用，写在 `:root`，主题只覆盖语义变量）

```css
:root {
  /* 字体栈：系统栈，中文回退清晰 */
  --font-ui: -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC",
             "Microsoft YaHei", "Noto Sans SC", sans-serif;
  --font-mono: ui-monospace, "SF Mono", "Cascadia Code", Menlo, Consolas, monospace;

  /* 字号阶梯：7 档，收编现有 16 档。桌面 UI 基准 13px */
  --fs-11: 11px;   /* 状态条、辅助角标（仅限大写字母距场景） */
  --fs-12: 12px;   /* chips、徽章、表格元数据 */
  --fs-13: 13px;   /* 正文基准、表格、表单、按钮 */
  --fs-14: 14px;   /* 次级标题、导航项、强调正文 */
  --fs-16: 16px;   /* 卡片标题、页面标题（紧凑） */
  --fs-20: 20px;   /* 页面主标题 */
  --fs-28: 28px;   /* 数字大屏（镜像时长、传输速度） */

  /* 行高 */
  --lh-tight: 1.3;   /* 标题 */
  --lh-ui: 1.45;     /* UI 控件 */
  --lh-body: 1.6;    /* 说明文字、帮助正文 */

  /* 间距标尺（4px 基，收编全部现值） */
  --sp-1: 4px;  --sp-2: 8px;  --sp-3: 12px; --sp-4: 16px;
  --sp-5: 20px; --sp-6: 24px; --sp-8: 32px; --sp-10: 40px;

  /* 圆角 4 档（收编 12 档） */
  --r-xs: 5px;   /* chip、徽章 */
  --r-sm: 8px;   /* 按钮、输入框、小卡 */
  --r-md: 12px;  /* 卡片、面板 */
  --r-lg: 16px;  /* 对话框、弹层 */

  /* 描边与阴影统一由主题提供；动效统一 */
  --ease: cubic-bezier(.2, .8, .2, 1);
  --dur-fast: .12s; --dur: .18s;

  /* 焦点环 */
  --focus-ring: 0 0 0 2px var(--surface), 0 0 0 4px var(--accent);
}
```

### 3.2 五套主题完整变量表

每套主题必须覆盖**全部**以下变量（不许再出现「只覆盖一半」）：
`--bg, --surface, --surface-2, --surface-3, --text, --text-2, --muted, --line, --line-strong, --accent, --accent-strong, --accent-soft, --on-accent, --ok, --ok-soft, --warn, --warn-soft, --danger, --danger-soft, --shadow-1, --shadow-2, --radius-shift, --titlebar-bg, --titlebar-text, --mask`

> **`--mask`（2025-xx 裁决补充）**：对话框/抽屉遮罩背景，正式收编为第 25 个规范变量。浅色主题（soft/prism/paper/aurora）：`--mask: rgba(12,16,24,.45)`；深色主题（midnight）：`--mask: rgba(0,0,0,.55)`。禁止硬编码，弹层一律引用此变量。

文字层级约定：`--text` 主文字 / `--text-2` 次级（表格值、导航）/ `--muted` 辅助（占位、说明）。**全部主题要求 --text 对 --surface ≥ 7:1，--muted ≥ 4.5:1。**

#### 主题 A ·「云鲸 Soft Light」—— 温和浅色（默认主题）

气质：Keep 平静的生产力浅色；中性灰白表面 + 一个稳的蓝。
```css
:root[data-theme="soft"] {
  color-scheme: light;
  --bg: #f5f6f8;            /* 工作区底 */
  --surface: #ffffff;       /* 卡片、侧栏 */
  --surface-2: #f1f3f6;     /* 卡内分区、表头 */
  --surface-3: #fafbfc;     /* 嵌套小块 */
  --text: #1b222b;  --text-2: #49525f;  --muted: #8a93a2;
  --line: #e4e8ee;  --line-strong: #d2d8e1;
  --accent: #3469e4; --accent-strong: #2a54bd; --accent-soft: #e8eefc;
  --on-accent: #ffffff;
  --ok: #1d8a54;   --ok-soft: #e3f4eb;
  --warn: #b06a10; --warn-soft: #fcf1e0;
  --danger: #cd3d3d; --danger-soft: #fbe9e9;
  --shadow-1: 0 1px 2px rgba(23,32,48,.05);            /* 卡片默认 */
  --shadow-2: 0 8px 24px rgba(23,32,48,.12);           /* 弹层/toast */
  --radius-shift: 0;                                    /* 不缩放圆角 */
  --titlebar-bg: var(--surface); --titlebar-text: var(--text);
}
```

#### 主题 B ·「深空 Midnight」—— 现代深色（替代 mono 字体 gimmick）

气质：Raycast / Linear Dark：近黑中性底、发丝分隔、亮蓝强调。系统字体，不再用等宽正文。
```css
:root[data-theme="midnight"] {
  color-scheme: dark;
  --bg: #101216;
  --surface: #171a1f;
  --surface-2: #1e222a;
  --surface-3: #13161b;
  --text: #e9ecf1; --text-2: #a6aeb9; --muted: #6e7683;
  --line: #262b33; --line-strong: #343b45;
  --accent: #4f8bff; --accent-strong: #7aa6ff; --accent-soft: rgba(79,139,255,.14);
  --on-accent: #0b1220;
  --ok: #3ecf8e;   --ok-soft: rgba(62,207,142,.14);
  --warn: #f0a63c; --warn-soft: rgba(240,166,60,.14);
  --danger: #ff6b6b; --danger-soft: rgba(255,107,107,.13);
  --shadow-1: 0 1px 2px rgba(0,0,0,.4);
  --shadow-2: 0 12px 32px rgba(0,0,0,.5);
  --radius-shift: 0;
  --titlebar-bg: var(--surface); --titlebar-text: var(--text);
}
```

#### 主题 C ·「磁贴 Prism」—— 彩色磁贴（彩色只给图标，不给结构）

气质：中性结构与主题 A 一致，**彩色预算花在功能图标与分类徽章**上；accent 换紫，保留「彩色磁贴」的人格但去掉撞色。
```css
:root[data-theme="prism"] {
  color-scheme: light;
  --bg: #f4f5f9;
  --surface: #ffffff;
  --surface-2: #f0f2f7;
  --surface-3: #fafafc;
  --text: #1d1f2b; --text-2: #4b4f63; --muted: #8a8ea0;
  --line: #e4e6ef; --line-strong: #d3d6e3;
  --accent: #6d4fe0; --accent-strong: #5a3fc6; --accent-soft: #ece8fc;
  --on-accent: #ffffff;
  --ok: #17855f;   --ok-soft: #e0f4ec;
  --warn: #b26a0f; --warn-soft: #fcf1df;
  --danger: #cc3d54; --danger-soft: #fbe8ec;
  /* 分类色（仅用于工具图标底与分组徽章，禁止用于边框/标题条） */
  --cat-blue: #2f6fe4; --cat-teal: #0e9488; --cat-violet: #6d4fe0;
  --cat-pink: #d6457e; --cat-amber: #c07a12; --cat-green: #2c9a52;
  --shadow-1: 0 1px 2px rgba(29,31,43,.06);
  --shadow-2: 0 8px 24px rgba(29,31,43,.13);
  --radius-shift: 0;
  --titlebar-bg: var(--surface); --titlebar-text: var(--text);
}
```

#### 主题 D ·「留白 Paper」—— 极简排印（黑色即强调色）

气质：Things / Arc Minimal：纯白、hairline、**主按钮是黑色**、无阴影；唯一的「大胆」是极致的排印。
```css
:root[data-theme="paper"] {
  color-scheme: light;
  --bg: #ffffff;
  --surface: #ffffff;
  --surface-2: #f7f7f8;
  --surface-3: #fafafa;
  --text: #17181a; --text-2: #45474c; --muted: #7c8087;
  --line: #ebebee; --line-strong: #dcdee1;
  --accent: #17181a; --accent-strong: #000000; --accent-soft: #f1f1f3;
  --on-accent: #ffffff;
  --ok: #1f7a46;   --ok-soft: #eaf4ee;
  --warn: #9a6206; --warn-soft: #faf3e3;
  --danger: #c23b3b; --danger-soft: #faebeb;
  --shadow-1: none;
  --shadow-2: 0 4px 16px rgba(23,24,26,.1);
  --radius-shift: -2px;   /* 圆角整体 -2px：10→8、12→10、16→14，更利落 */
  --titlebar-bg: var(--surface); --titlebar-text: var(--text);
}
```

#### 主题 E ·「玻璃 Aurora」—— 玻璃 HUD（收敛版）

气质：保留「签名瞬间」的桌面光斑与毛玻璃，但：blur 只用在**两个真正悬浮的层**（标题栏、弹层），不铺满所有容器；边框改为「暗色 hairline」保证浅底可见。
```css
:root[data-theme="aurora"] {
  color-scheme: light;
  --bg: #e9eef8;
  --surface: rgba(255,255,255,.78);
  --surface-2: rgba(255,255,255,.55);
  --surface-3: rgba(255,255,255,.42);
  --text: #1a2438; --text-2: #47536d; --muted: #6b7794;
  --line: rgba(20,32,60,.12);        /* 暗色 hairline，替代白描边 */
  --line-strong: rgba(20,32,60,.2);
  --accent: #3e63dd; --accent-strong: #2c4ab8; --accent-soft: rgba(62,99,221,.12);
  --on-accent: #ffffff;
  --ok: #1d8a54;   --ok-soft: rgba(29,138,84,.13);
  --warn: #b06a10; --warn-soft: rgba(176,106,16,.13);
  --danger: #cd3d3d; --danger-soft: rgba(205,61,61,.12);
  --shadow-1: 0 1px 3px rgba(25,40,80,.1);
  --shadow-2: 0 12px 36px rgba(25,40,80,.18);
  --radius-shift: +2px;  /* 圆角整体 +2px：更圆润呼应玻璃 */
  --titlebar-bg: var(--surface); --titlebar-text: var(--text);
  /* 签名背景（仅 body 一层，去掉双 radial + 三段渐变） */
  /* body::before { position:fixed; inset:0; z-index:-1; content:""; background:
     radial-gradient(1100px 500px at 12% -8%, rgba(93,140,255,.28), transparent 60%),
     linear-gradient(160deg, #eef2fb, #dfe6f3); } */
  /* backdrop-filter 只加给 .titlebar 与 .modal/.popover 两类容器 */
}
```

### 3.3 语义使用规则（防止旧病复发）

1. **accent 只出现在**：主按钮、当前导航项、焦点环、链接文字、运行中状态。其余一律中性。
2. **ok/warn/danger 只出现在**：状态点、状态徽章、对应语义的 banner/按钮。禁止装饰性使用。
3. 左边框彩条、彩色卡片标题条**全部废除**；分组靠 `--surface-2` 分区底 + 字重。
4. 深色主题 B 的 `--on-accent` 是深色文字（亮蓝底），不要想当然用白字。

---

## 4. 布局架构

### 4.1 外壳结构（解决 G1/G3/G4）

```
┌──────────────────────────────────────────────────────────────┐
│ Titlebar 44px：拖拽区 | 品牌 | ⌘K 搜索(居中,340px) | 设备pill·主题·帮助 │
├────────────┬─────────────────────────────────────────────────┤
│ Sidebar    │  Workspace                                       │
│ 232px      │  ┌ View Header 56px：页面标题 + 说明 + 视图级动作 ┐ │
│ (可折叠至   │  ├──────────────────────────────────────────────┤ │
│  56px)     │  │ Content：max-width 1080px，--sp-6 内边距      │ │
│            │  │ （有设备详情需求时右侧 280px Context Panel）   │ │
│ 4 组导航    │  └──────────────────────────────────────────────┘ │
├────────────┴─────────────────────────────────────────────────┤
│ Statusbar 28px：设备状态点 · 连接方式 · 版本 · 本地优先声明      │
└──────────────────────────────────────────────────────────────┘
```

关键取舍：

1. **标题栏去掉一级模块导航与渐变**（G1/G4）：4 个模块Tab整体删除，分组信息由侧栏承担。标题栏变为 44px 中性表面（`--titlebar-bg`），左侧品牌（20px 字号，无 8px tagline），中央 Raycast 式搜索框（快捷键 ⌘K/Ctrl+K，占位「搜索功能或输入命令…」），右侧：设备 pill（状态点+设备名/「未连接」）、主题切换下拉、帮助。标题栏不与内容争色。
2. **右栏（236px）从常驻列降级为按需 Context Panel**（G3）：仅 6 个设备强相关视图（设备信息/镜像控制/投屏演示/文件传输/应用管理/通知中心）可展开 280px 右侧面板（默认展开、可一键收起）；其余视图不渲染。原「常用工具/APK 快速安装/安全边界」三卡内容分别并入：设备中心首页的工具格、批量安装页的拖放区、帮助中心/设置的隐私说明。设备状态 kv 收进标题栏设备 pill 的 popover。
3. **subbar（40px 面包屑行）取消**：面包屑由侧栏选中态表达；「连接设备/重新检查」并入 View Header 右侧动作区。
4. **Statusbar 28px 保留**：11px → 12px（`--fs-12`），内容分组：左=设备状态点+名称；右=`本地处理 · 无遥测 · v?`（隐私气质置于明面）。

### 4.2 侧栏交互

- 宽度 232px；底部「折叠」按钮 + `⌘B`。折叠后 56px：只显示图标，hover 浮出 tooltip；组头收成 1px 分隔线。
- 分组（4 组，与现有一致）：**设备**（设备中心/连接与授权/设备信息/多端）、**镜像与会话**（镜像控制/投屏演示/按键映射/定时录制/操作回放）、**内容与传输**（文件传输/截图/录像/相册/批量安装/应用管理/通知中心）、**系统**（设置/帮助中心/Pro·激活）。
- 组头样式：`--fs-11` 大写字母距 0.05em、`--muted`、上边距 `--sp-4`；**可折叠**（点击组头收合子项）。
- 导航项：高 32px、`--fs-14`? 不，用 `--fs-13` weight 500、图标 16px `--muted`；hover `--surface-2`；active：`--accent-soft` 底 + `--accent` 图标与文字（weight 600）+ 左侧 2px accent 指示条（*唯一*保留的左指示条）。无设备可用时，设备相关项保持可点（进入后显示真实空态），不做禁用灰——功能入口不能丢。
- 侧栏底部：折叠按钮 + 「本地版 · 无需账号」一行 `--fs-12 --muted`。

### 4.3 View Header（新增，消灭 G2）

每个视图一个 56px header：左侧 `--fs-16` 标题 + 一句 `--fs-13 --muted` 说明（**只出现这一次**）；右侧视图级主动作（如「开始镜像」「刷新清单」）。sticky 于工作区顶部，底边 `--line`。卡片内不再重复标题。

---

## 5. 组件视觉规范

所有值均基于 3.1 基座变量；「默认/悬停/禁用」为必给状态。

### 按钮
```css
.btn        { height:32px; padding:0 14px; border-radius:var(--r-sm);
              font-size:var(--fs-13); font-weight:600; border:1px solid transparent;
              transition: background var(--dur-fast), border-color var(--dur-fast), box-shadow var(--dur-fast); }
.btn:focus-visible { box-shadow: var(--focus-ring); }
.btn:disabled      { opacity:.45; cursor:not-allowed; }

/* Primary：每屏至多 1 个 */
.btn-primary { background:var(--accent); color:var(--on-accent); }
.btn-primary:hover:not(:disabled) { background:var(--accent-strong); }

/* Secondary（默认按钮样式，中性！解决 G7） */
.btn-secondary { background:var(--surface); color:var(--text); border-color:var(--line-strong); }
.btn-secondary:hover:not(:disabled) { background:var(--surface-2); }

/* Danger */
.btn-danger { background:transparent; color:var(--danger); border-color:var(--danger); }
.btn-danger:hover:not(:disabled) { background:var(--danger-soft); }
.btn-danger-solid { background:var(--danger); color:#fff; }   /* 仅「清空全部」类不可逆操作 */

/* SM */
.btn-sm { height:26px; padding:0 10px; font-size:var(--fs-12); }
/* 文本按钮 */
.btn-ghost { height:auto; padding:2px 6px; background:none; border:0; color:var(--accent); font-weight:600; }
.btn-ghost:hover { background:var(--accent-soft); border-radius:var(--r-xs); }
```

### 卡片
```css
.card { background:var(--surface); border:1px solid var(--line);
        border-radius:var(--r-md); box-shadow:var(--shadow-1); }
.card-head { padding:var(--sp-4) var(--sp-5); border-bottom:1px solid var(--line);
             font-size:var(--fs-14); font-weight:600; }        /* 无彩条！ */
.card-body { padding:var(--sp-5); }
.card-section { background:var(--surface-2); border-radius:var(--r-sm); padding:var(--sp-4); }
```

### 表格
```css
.table { width:100%; font-size:var(--fs-13); border-collapse:collapse; }
.table th { padding:8px 12px; text-align:left; color:var(--muted); font-weight:500;
            font-size:var(--fs-12); background:var(--surface-2);
            border-bottom:1px solid var(--line); }
.table td { padding:10px 12px; border-bottom:1px solid var(--line); color:var(--text-2); }
.table tbody tr:hover { background:var(--surface-3); }
/* 空表不渲染表头，直接渲染空态（见下） */
```

### Chips / 徽章
```css
.chip { height:28px; padding:0 12px; border-radius:999px; font-size:var(--fs-12);
        border:1px solid var(--line-strong); background:var(--surface); color:var(--text-2); cursor:pointer; }
.chip:hover { border-color:var(--accent); color:var(--accent); }
.chip.active { background:var(--accent); border-color:var(--accent); color:var(--on-accent); }
.badge { padding:2px 8px; border-radius:var(--r-xs); font-size:var(--fs-12); font-weight:600; }
.badge.ok { background:var(--ok-soft); color:var(--ok); }
.badge.warn { background:var(--warn-soft); color:var(--warn); }
.badge.danger { background:var(--danger-soft); color:var(--danger); }
.badge.neutral { background:var(--surface-2); color:var(--muted); }
```

### 表单控件
```css
.input, .select { height:32px; padding:0 10px; border-radius:var(--r-sm);
                  border:1px solid var(--line-strong); background:var(--surface);
                  color:var(--text); font-size:var(--fs-13); }
.input:hover { border-color:var(--muted); }
.input:focus  { outline:none; border-color:var(--accent);
                box-shadow:0 0 0 3px var(--accent-soft); }
.input::placeholder { color:var(--muted); }
.input:disabled { background:var(--surface-2); color:var(--muted); }
.switch { width:36px; height:20px; border-radius:999px; }  /* 开：--accent；关：--line-strong，白点 16px */
```

### 标签页（视图内分段，如连接方式）
```css
.tabs { display:flex; gap:2px; background:var(--surface-2); padding:2px; border-radius:var(--r-sm); }
.tab { height:28px; padding:0 12px; border-radius:6px; font-size:var(--fs-13); color:var(--text-2); }
.tab.active { background:var(--surface); color:var(--text); font-weight:600; box-shadow:var(--shadow-1); }
```

### 空态（真实状态原则的核心组件）
```css
.empty { display:grid; justify-items:center; gap:var(--sp-3);
         padding:var(--sp-10) var(--sp-6); text-align:center; }
.empty-icon { width:56px; height:56px; border-radius:var(--r-md);
              display:grid; place-items:center;
              background:var(--surface-2); color:var(--muted); }  /* 28px 线性图标 */
.empty h3  { font-size:var(--fs-14); font-weight:600; }       /* 「还没有可管理的应用」 */
.empty p   { font-size:var(--fs-13); color:var(--muted); max-width:420px; line-height:var(--lh-body); }
.empty .btn { margin-top:var(--sp-2); }                        /* 主 CTA：连接设备/刷新清单 */
.empty.hint { padding:var(--sp-6); }                            /* 紧凑变体用于卡内 */
```
规则：**空态三件套 = 图形位 + 一句结论 + 一个主 CTA**；绝不再输出带表头的空表格。

### 加载态
```css
.skeleton { border-radius:var(--r-xs);
            background:linear-gradient(90deg, var(--surface-2) 25%, var(--surface-3) 50%, var(--surface-2) 75%);
            background-size:200% 100%; animation:shimmer 1.2s infinite linear; }
@keyframes shimmer { to { background-position:-200% 0; } }
/* 表格加载：5 行 skeleton 行高同真行；kv 加载：skeleton 条代替值 */
/* 设备探测中：状态点 --accent + pulse 动画 */
```

### 错误态 / Banner（解决 G8）
```css
.alert { display:flex; gap:var(--sp-3); padding:var(--sp-3) var(--sp-4);
         border-radius:var(--r-sm); font-size:var(--fs-13); line-height:var(--lh-body); }
.alert-warn   { background:var(--warn-soft); color:var(--warn); }
.alert-danger { background:var(--danger-soft); color:var(--danger); }
.alert-info   { background:var(--accent-soft); color:var(--accent-strong); }
/* 左侧 16px 语义图标；右侧可关闭 ×；同页多条错误时折叠为 1 条+「查看全部(n)」 */
/* 错误必须给「下一步」链接，如「重新检查」——不许只有一句描述 */
```

### Toast
```css
.toast { position:fixed; right:var(--sp-5); bottom:calc(28px + var(--sp-4)); z-index:80;
         display:flex; align-items:center; gap:var(--sp-3);
         padding:10px 14px; border-radius:var(--r-sm);
         background:var(--surface); border:1px solid var(--line);
         box-shadow:var(--shadow-2); font-size:var(--fs-13); }
.toast.ok { border-left:3px solid var(--ok); } .toast.error { border-left:3px solid var(--danger); }
/* 4s 自动消失；错误 toast 带「重试」ghost 按钮 */
```

### 对话框
```css
.modal { width:min(480px, calc(100vw - 48px)); border-radius:var(--r-lg);
         background:var(--surface); border:1px solid var(--line); box-shadow:var(--shadow-2);
         padding:var(--sp-6); }
.modal-mask { background:rgba(12,16,24,.45); }        /* 深色主题下 .55 */
.modal-foot { display:flex; justify-content:flex-end; gap:var(--sp-2); margin-top:var(--sp-5); }
/* 危险确认：主按钮用 .btn-danger-solid */
```

### 状态点
```css
.dot { width:8px; height:8px; border-radius:50%; }
.dot.ok { background:var(--ok); } .dot.warn { background:var(--warn); }
.dot.off { background:var(--muted); } .dot.busy { background:var(--accent); animation:pulse 1.2s infinite; }
```

---

## 6. 19 个视图逐页处理要点

标记：**[重构]** = 布局需要动；**[换皮]** = 只需套用新 tokens/组件。

| # | 视图 | 处理 | 要点 |
|---|------|------|------|
| 1 | 设备中心 home | **[重构]** | 页面分两态：无设备 → 全宽空态（图形位：手机+线缆线性图标；三步连接引导压缩为 3 张横排 step 卡；主 CTA「连接设备」）；有设备 → 概览仪表（设备卡横排 + 快捷工具格替换原右栏「常用工具」）。3 条黄色错误横幅改 `.alert-danger` 折叠组。品牌查询 chip 行保留但换 `.chip` 样式 |
| 2 | 连接与授权 connect | **[重构]** | 顶部 `.tabs` 分三法：USB 数据线 / 无线连接 / 配对码；每法一个引导卡（编号步骤用 `.card-section`），推荐法默认选中。已保存无线设备/受信设备用 `.table` + 空态。原 3 个彩色 eyebrow 卡头全部去除 |
| 3 | 设备信息 info | [换皮] | kv 值用 `.card-section` 双列 `dl`；「尚未探测」的值显示 skeleton 而非文案；底部隐私说明并入 `.alert-info`。可展开 Context Panel 放诊断历史 |
| 4 | 多端 multi | [换皮] | 设备表 + 镜像会话表共用 `.table`；「结束全部」用 `.btn-danger`；空态文案入 `.empty.hint` |
| 5 | 镜像控制 mirror | **[重构]** | 页面主体 = 16:9 镜像画面区（无会话时是 `.empty` 大图形位）；右侧 280px Context Panel 放会话参数（目标设备/通道/画质/桌面模式，用定义列表）；底部工具条放开始/结束按钮 + 保护内容提示 `.alert-warn`。这是全应用视觉锚点页 |
| 6 | 投屏演示 present | [换皮] | 复用 mirror 的画面区骨架；「就绪检查」4 项 kv 收成一行 4 个 check 项（通过=ok 点，未过=warn 点）；按钮逻辑不变 |
| 7 | 按键映射 keymap | **[重构]** | 方案列表从「+新建方案」裸文字升级为方案卡片网格（空态给 `.empty`）；编辑器为行式表单（动作+坐标用 `--font-mono` 13px）；快捷键行统一 kbd 样式（`--surface-2` 底、`--r-xs`、mono） |
| 8 | 定时录制 schedule | [换皮] | 计划从 chip 改为 `.table` 行（名称/时间/时长/状态 badge/启停/删除）；行内添加表单收进 `.card-section`；「还没有录制计划」用 `.empty.hint` |
| 9 | 操作回放 replay | [换皮] | 与 keymap 同骨架；脚本卡片网格 + 行式编辑；「开始回放/停止」固定在 View Header |
| 10 | 文件传输 files | **[重构]** | 顶部「发送到手机」拖放区（`.dropzone`：1px dashed `--line-strong`、hover 时 accent）+ 「取回」分组列表保留现有 grid 统一列宽方案；成功/失败用行首 2px 语义竖条（保留）但颜色换 token |
| 11 | 截图 shot | [换皮] | 「最近截图」改为横向缩略图条（88px 高、`--r-sm`、点击放大）；无截图时 `.empty`；「立即截图」进 View Header |
| 12 | 录像 record | [换皮] | 参数区（状态/文件/保存位置/授权）用 kv `.card-section`；与 shot 共用结构 |
| 13 | 相册 album | **[重构]** | 目录 chips（.tabs 或 .chip 行）保留；缩略图网格（140px 方格、`--r-sm`）为主视图；未连接时 `.empty`。只读提示并入 `.alert-info` |
| 14 | 批量安装 install | **[重构]** | 主视图 = 拖放/选择 APK 队列表（文件/大小/状态 badge/进度/结果）；顶部「选择 APK(可多选)」+ 「把 APK 拖到这里」dropzone 合并为一个入口区；队列空时 `.empty` |
| 15 | 应用管理 apps | **[重构]** | 工具条：搜索框 + 排序 select + 「安装 APK」副按钮；应用行：图标 32px + 名称 + 包名(mono, muted) + 大小 + 卸载 ghost；清单空态 `.empty`（CTA=连接设备）。行虚拟化建议（>300 应用） |
| 16 | 通知中心 notify | [换皮] | 通知行卡：来源图标 + 应用名 + 正文两行截断 + 时间 + 快捷回复（展开时）；清空按钮 `.btn-danger`；空态「开启镜像后，通知会实时出现在这里」+ 前往开启 CTA |
| 17 | 设置 settings | **[重构]** | 分组卡（外观主题/通用/镜像/传输/隐私）内行式设置（沿用现有 row 结构，换新 tokens）；主题选择器 swatch 重绘：5 个预览小卡（bg/accent/表面三色条 + 名称），active 态 accent 描边 2px；原生 checkbox 全部换 `.switch` |
| 18 | 帮助中心 help | [换皮] | 折叠文章列表保留，行高用 `--lh-body`；顶部加搜索框过滤；「展开全部」ghost 化 |
| 19 | Pro/激活 pro | [换皮] | 激活输入卡单独置顶（input + 主按钮）；版本/权益用 kv 行；「本地校验·不需要账号」做成 badge.ok 强调隐私；错误信息 `.alert-danger` + 重试 |

共性：所有视图删除卡片标题与页面标题的重复（G2）；所有「说明横幅」归入 `.alert` 体系并给关闭与下一步动作；所有空表格替换为 `.empty`。

---

## 7. Do / Don't 清单

**Do**
- Do 每屏最多 1 个 primary 按钮；其余动作全部 `.btn-secondary` / `.btn-ghost`。
- Do 用 `--surface-2` 分区底 + 字重分层，代替彩条与彩色标题。
- Do 所有状态（空/加载/错误/成功）都有专属组件，无设备时页面依然完整可看。
- Do 7 档字号、4 档圆角、4px 间距标尺——新样式只允许引用 token。
- Do 深色主题 B 与浅色主题同等完整度（焦点环、hover、disabled 全部校验）。
- Do 强调「本地优先」：状态条常驻 `本地处理 · 无遥测`；隐私说明用 `.alert-info` 而非灰色小字。

**Don't**
- Don't 恢复渐变标题栏、彩色左边框卡头、彩虹 tone 类（`.assistant-tone-*` 全删）。
- Don't 用蓝色表示一切（链接/chip/badge/active 不再加蓝底）；secondary 按钮禁止 accent 底色。
- Don't 低于 11px 的字号（8px tagline、9.5px rail-tools 一律消灭）；中文正文不小于 13px。
- Don't 空 table（有表头无数据）——直接渲染 `.empty`。
- Don't 玻璃 blur 超过 2 层容器；theme E 不再给侧栏/卡片整体 blur。
- Don't 等宽字体作正文字体（mono 仅用于坐标、路径、包名、快捷键）。
- Don't 演示数据填充空态——空态必须反映真实 adb 状态。
- Don't 在 1000px 窗口宽度下同时渲染 232px 侧栏 + 280px Context Panel（Context Panel 在 <1200px 自动改为工作区内折叠面板）。

---

## 8. 给工程师的最小实现顺序（单文件预览）

1. 先写 `:root` 基座 + 5 套主题变量块（可直接抄 §3），全局只有 token 引用。
2. 搭外壳 grid（§4.1）：titlebar 44 / sidebar 232(折叠 56) / workspace / statusbar 28。
3. 实现 §5 组件库（btn/card/table/chip/alert/empty/skeleton/toast/modal 各一个实例）。
4. 用「设备中心（无设备空态）」+「镜像控制（Context Panel 版）」+「应用管理（表格）」三页验证全部组件后再铺其余 16 页。

## §9 迭代 v2：圆润化与柔化（2026-11-25 用户反馈）

用户反馈：边角要更圆润、不要四方边框、色彩更柔和，并融合爱思风格的亲和感。以下调整已同步实现于预览文件，**与 §3.2 token 表冲突处以本节为准**：

1. **圆角档位（v2.2 终版，用户两轮反馈后锁定）**：`--r-xs 5→4 / --r-sm 8→7 / --r-md 12→9 / --r-lg 16→12`（paper 仍整体 -2px、aurora +2px）。卡片/弹层用 r-lg，磁贴/块用 r-md，控件用 r-sm。首版 9/13/17/22 过圆、二版 6/9/12/15 仍偏圆，4/7/9/12 为用户确认的利落档。
2. **去四方边框（分层策略按主题分化）**：
   - 浅色三主题（soft/prism/aurora）：卡片/磁贴/工具卡/右栏块/计划卡/弹层 **全部去整圈描边**，改「背景分层 + 双层柔影」；hover 用 accent-soft 或 surface-2 过渡。
   - midnight：卡片保留极淡描边（`--line`）+ 柔影（深色下阴影分层同样成立）。
   - paper：保留 1px 细线无阴影签名（这是该主题的身份），但圆角跟随新档位。
   - 仅功能性描边保留：输入框/搜索框（视觉弱化至 `--line`，hover 才到 `--line-strong`）、表格行分隔线、媒体画布。
3. **色彩柔化**：soft 强调色 `#3469e4 → #4a7cf3`（提亮降饱和），底色微调冷蓝灰 `#f6f7fb`；prism 强调 `#6d4fe0 → #7a5cf0`、六分类色全部粉彩化；aurora 强调 `#3e63dd → #4f74e8`；状态色统一降一档饱和（danger #cd3d3d→#d24d4d 等）。阴影全部改为「近距 + 远距扩散」双层结构，避免生硬投影。
4. **爱思亲和感**：顶栏底 `--titlebar-bg` 用极淡蓝调渐变（#fbfcff→#f4f6fc）；侧栏导航改 **999px 药丸**（去掉左侧竖条指示）；顶栏搜索、chips、徽章、分页 Tab 全部药丸形；徽章 padding 加大（3px 10px）。
5. **主按钮**：accent 纵向微渐变 + 同色柔影（`color-mix` 55% 透明度），hover 加深渐变并增强投影——保留「每屏至多一个 primary」约束。

## §10 迭代 v2.3：粉彩图标 + 全主题圆角统一（2026-11-25 用户反馈）

1. **磁贴 Prism 分类图标压淡一档**：由「彩底白图标」改为「淡彩底 + 同系深色图标」。新增 6 个 soft token：`--cat-blue-soft #e3ecfd / --cat-teal-soft #dbf2f0 / --cat-violet-soft #ece8fd / --cat-pink-soft #fbe6ee / --cat-amber-soft #faf0d9 / --cat-green-soft #def1e6`，图标色取对应分类色的深化档（如 c1 图标 `#3568cf`）。状态徽章本来就是 soft 底彩字，方向一致。
2. **全主题圆角统一为 4/7/9/12**：删除 paper 的 -2px 与 aurora 的 +2px 偏移（`--radius-shift` 全部归零），五套主题共用同一圆角档；paper 保留「细线无阴影」、aurora 保留「光斑 + blur」签名，仅圆角不再差异化。
3. **修复圆角塌陷 bug（重要）**：`--radius-shift` 原为无单位 `0`，`calc(12px + 0)` 在部分 Chromium 版本判定非法 → computed 圆角变 0px（正方形）。v1 起就存在此隐患；现全部改为 `--radius-shift: 0px`（带单位），浏览器探针与卡片实测均恢复 12px。**教训：token 参与长度运算时必须带单位。**
