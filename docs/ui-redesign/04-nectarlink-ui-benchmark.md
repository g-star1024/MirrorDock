# NectarLink UI 基准对照（2026-11-25）

> 调研对象：`github.com/thtbee/nectarlink` v0.1.5，README 内嵌 12 张界面截图（已存 `nectarlink-ref/`）。
> **许可证边界**：NectarLink 桌面/Android 应用为 GPL-3.0-or-later、core 为 MPL-2.0。本对照**只抽象信息架构与交互模式，不复制任何代码、文案片段或视觉资产**。截图仅供内部设计参考，不进入 MirrorDock 发布产物。

## 一、NectarLink 的通用 UI 骨架

| 层 | 做法 | 与 MirrorDock 预览对比 |
|---|---|---|
| 最左导航 | ~88px 竖栏：图标在上、小字标签在下（Home / Messages / Calls / Photos / Apps / Deck / Guide / Settings），底部 `+` | 我们是 232px 侧栏 + 56px 折叠态，**更宽但承载 19 视图**；不照搬 |
| 顶栏 | 左 logo + 当前页名；右 `Commands` / `Timeline` / 刷新 / 窗口控制 | 我们 44px 顶栏有 ⌘K + 设备 pill + 主题菜单；**命令面板方向一致**，缺 Timeline 入口 |
| 内容区 | 卡片按 2:1 分栏（左主任务、右辅助能力），卡片淡底无边框、~12px 圆角 | 我们已是这套卡语言（X10-142），**分层逻辑一致** |
| 主题 | Hive / Bloom / Glass / Graphite × 浅深，Theme + Mode(System/Light/Dark) + Color 圆点 | 我们 5 主题含浅深（Soft/Midnight/Prism/Paper/Aurora），**无 System 跟随**（未实现，不承诺） |
| 安全叙事 | 每功能独立每设备开关、敏感默认关；能力矩阵保证「界面永不提供用不了的功能」 | 我们已做 readyDevice 门禁，**需升级为能力矩阵式显隐** |

## 二、逐页对照

### 2.1 主页 Home

NectarLink：大号**设备 Hero 卡**（手机缩略图 + 设备名 + Android 版本 + `USB · 2ms` 连接质量徽章 + 电量 + 媒体播放器小卡）→ 4 个等宽快捷卡（Mirror screen / Send files / Send clipboard / Find phone）→ 左「Notifications」大卡（分组条目 + `History` + `Clear all` + 单条 `...` `×` + 内联动作 Reply/Mark as read/**Copy code**）→ 右「Phone controls / Continuity Camera / Phone as webcam / Type on phone / Timeline」卡。

MirrorDock 预览：三张等权 device-tile → 6 个带描述 tool-card → 「最近任务」演示表 → 安全边界 alert。

| 决策 | 内容 | 理由 |
|---|---|---|
| ✅ 采纳 | 主设备升级为 Hero 卡（手机框 + 名称 + 系统 + 连接方式/延迟徽章 + 电量），其它设备保持小卡 | 一屏建立「我连的是谁、连得怎样」的视觉锚点；我们的连接状态分散在多处 |
| ✅ 采纳 | 通知卡卡头加计数 badge + `清空` 卡脚操作 | 与 NectarLink `Notifications · 3 / Clear all` 同构 |
| ⏸ P1 | 主页 Timeline 卡（最近活动） | 需真实传输记录数据源支撑；演示态可先出形态，真实落地等同 P1 |
| ❌ 不采纳 | Phone controls（DND/手电筒/WiFi/BT/热点）、Continuity Camera、Phone as webcam | 功能范围外，且需要额外权限/虚拟设备 |
| ❌ 不采纳 | Hive 动态蜂窝背景、蜂蜜色 | 与「安静的诊断台」设计语言冲突 |

### 2.2 连接/配对 Pairing

NectarLink：居中白卡，QR 码 + 三步指引（Install / Tap 'Pair with PC' / Scan this code）+ 倒计时提示 + **`Pair nearby instead` 备用路径**，失败/无摄像头有明确降级。

MirrorDock 预览：三 Tab（USB / 无线 / 配对码）+ steps + 配对码表单 + 受信设备表。

| 决策 | 内容 | 理由 |
|---|---|---|
| ✅ 采纳 | 每个连接 Tab 末尾增加「无法连接时的替代方案」提示行 | 对齐产品规划 §「检测→解释→请求操作→验证→恢复」，我们不展示 QR 但要有等价降级 |
| ❌ 不采纳 | QR 配对主流程 | 我们用 USB/adb 显式授权，通道不同；强加 QR 会误导 |

### 2.3 通知 Notifications

NectarLink：**验证码通知条目下有 `Copy code` 按钮**（截图实例：验证码 22395）；单条 `×` 清除、批量 `Clear 5`、分组 `Show less`；消息类通知有 `Reply / Mark as read / Mute` 内联动作。

MirrorDock 预览：notify-row（图标 + 来源 + 正文 + 时间 + 快捷回复 details）+ 卡脚本机保留开关。

| 决策 | 内容 | 理由 |
|---|---|---|
| ✅ 采纳（P0-1） | 验证码通知渲染 `复制验证码` 按钮 | 真实需求；现有链路已把通知 `text` 送到前端，通道可行 |
| ✅ 采纳（P0-2） | 单条 `×` 清除 + 卡头 `清空` | 需新增下行 `notification_clear`，真实落地同步做 |
| ⏸ P1 | `Mark as read` / `Mute` 内联动作 | 需伴侣端通知动作通道，范围外 |
| ❌ 不采纳 | 多应用消息聚合收件箱 | 需短信/第三方聊天权限，范围外 |

### 2.4 设置 Settings

NectarLink：左侧二级分组导航（Appearance / General / Devices / Camera & Audio / Notifications / Files & Sync / Shortcuts & Rules / Storage & Privacy / Connection & About），右侧**单卡内行式设置**（左标题+说明，右控件，细分隔线）。

MirrorDock 预览：5 张平铺卡（外观主题 / 通用 / 镜像 / 传输 / 隐私）。

| 决策 | 内容 | 理由 |
|---|---|---|
| ✅ 采纳 | 设置页改为左侧分组导航 + 右侧单卡行式设置；分组对应我们现有 5 组 + 设备/连接 | 信息密度和可达性明显更好；纯视觉改造，不触碰数据 |
| ❌ 不采纳 | Mode System/Light/Dark 切换 | 未实现系统主题跟随，按能力矩阵原则不展示不可用功能 |
| ❌ 不采纳 | Color 主题色圆点 / Mica backdrop | 我们有自己的 5 主题语言，够用 |

### 2.5 投屏 Mirroring

NectarLink：手机画面是**独立浮动窗口**（顶部录制计时徽章），不与主界面嵌套；主界面同时显示 Timeline 中的投屏记录。

MirrorDock：scrcpy 本来就是独立窗口——**架构一致，无需改**。预览的 16:9 canvas 用于「会话状态说明」，保留。

### 2.6 应用 Apps / 消息 Messages（不采纳）

- Apps：手机 App 在 PC 上开独立窗口，依赖 Elevated（无线调试特权 + 辅助进程），权限路线与工程量大——**P2 专项**。
- Messages：短信/RCS/WhatsApp/Telegram 聚合收件箱，权限重且厂商差异大——**P2 专项**。
- 反向控制（触控板/摄像头/麦克风/Deck）超出 MirrorDock 产品边界——**不采纳**。

## 三、本轮升级范围

1. 预览主页：设备 Hero 卡 + 通知卡计数/清空形态。
2. 预览通知页：Copy code 按钮 + 单条清除形态（P0 真实功能同步落地）。
3. 预览设置页：分组导航 + 单卡行式设置。
4. 预览连接页：每 Tab 的替代方案提示。
5. 真实客户端：P0 三件套（Copy code / 双向清除 / 剪贴板常驻同步可行性评估与实现）。

## 四、升级落点（已完成，2026-11-25 17:25）

| 文件 | 改动 |
|---|---|
| `mirrordock-redesign-preview.html` | ① vHome 设备 Hero 卡（手机框+名称+系统+`USB 已授权 · 38 ms` 徽章+电量），其余设备降权小卡；② vNotify 卡头计数+清空、验证码条目「复制验证码」按钮、单条 `×` 清除（提示同步手机）、卡头清空文案改「清空并同步手机」；③ vSettings 改 6 组左侧导航 + 右侧单卡行式设置，新增「设备与能力」组（通知镜像/双向清除/剪贴板常驻同步/能力矩阵）；④ vConnect 每条路径加「替代方案」提示行；⑤ 配套 CSS + 演示数据 `验证码 · 126 邮箱`；⑥ runAct 新增 notify-copy / notify-dismiss / settings-group。 |
| 验证 | 内嵌 JS `node --check` 通过；浏览器走查 4 页：Hero 卡、复制验证码按钮、4 条单条清除、6 组导航切换、替代方案提示均正常，无溢出；控制台仅 favicon 404。 |

**注意**：预览中的「按设备显隐 / 剪贴板常驻同步」是设计形态，真实状态以 P0 实现结果为准，不做演示态伪装。

以上均不打 tag、不推远端、不触发 CI。
