# MirrorDock NAS 版

立项场景：**NAS 常开机 + USB 直连 + 24h 录制归档 + 远程 Web 访问**。

把手机长期挂在 NAS 上，用浏览器随时**看实时画面、远程操控、随时录**，录像直接落在 NAS 的大容量存储池里。

## 功能

- **实时镜像**：浏览器里看手机实时画面（WebCodecs 解码），在画面上点按/拖动即可远程操控，工具栏含返回/主页/多任务/电源/旋转。多标签页可同时看同一台。
- **录制归档**：三档画质（流畅/标准/高清）长时间录像，落盘即可在线拖进度条回放。
- **截图画廊**：远程抓取手机当前界面 PNG，缩略图网格浏览、点图放大、单张删除。
- **手机文件双向传输**：浏览目录、上传文件到手机、从手机下载、删除手机文件。
- **应用管理**：列出第三方/系统应用、上传 APK 安装。
- **归档保留策略**：按天数自动回收最旧录像与截图（见环境变量 `MIRRORDOCK_RETENTION_DAYS`）。

---

## 一分钟上手（飞牛 fnOS）

1. fnOS 里打开「Docker → Compose → 新建项目」
2. 把 `docker-compose.yml` 的内容粘贴进去
3. 手机用数据线插到 NAS 的 USB 口，手机上点「允许 USB 调试」（**只需一次**）
4. 浏览器打开 `http://<NAS 的 IP>:8080`

> 首次连接手机时，手机上会弹出 RSA 授权确认。授权密钥持久化在 `./adbkeys`，
> 容器重建后不必重新授权——这是 NAS 场景的关键，因为 NAS 通常离人很远。

**镜像架构**：`linux/amd64` 与 `linux/arm64` 双架构，`docker pull` 会自动挑匹配的那份。
若你的 NAS 架构特殊，可在 NAS 上用 `docker compose build` 本地原生构建
（注意构建上下文必须是仓库根目录，不是 `nas/`）。

> ⚠️ 镜像全名是 `ghcr.io/g-star1024/mirrordock/mirrordock-nas`——中间的 `mirrordock`
> 是仓库名，GHCR 的命名规则是 `ghcr.io/<owner>/<仓库名>/<包名>`，**不能省**。
> 少了它会报 `Head ".../mirrordock-nas/manifests/latest": denied`。

## 目录结构

```
nas/
├── Dockerfile          # 三阶段构建：scrcpy 运行时 → Rust 服务端 → 精简运行时
├── docker-compose.yml  # 部署清单（privileged + USB 透传 + 密钥持久化）
├── web/index.html      # Web 界面（单文件，无构建步骤）
└── server/             # Rust 服务端（axum），依赖 crates/md-core
```

## 为什么是这样

| 决策 | 理由 |
|---|---|
| **与桌面端共用 `crates/md-core`** | 设备去重、USB/无线通道优先级、结构化错误等判据，两端必须给出同一份答案。用户在两端看到不同的优先级是最难排查的问题。 |
| **src-tauri 被 workspace 排除** | 保持 `src-tauri/target/` 路径不变，CI 里所有依赖该路径的打包与发版步骤一行都不用改。 |
| **实时画面走 Rust 原生 scrcpy 协议 + WebCodecs** | 容器没有显示器，SDL 窗口开不了。我们在服务端用 Rust 实现了 scrcpy 4.1 的**客户端协议**（`scrcpy_proto.rs`，逐字节对照 `doc/develop.md`），把设备编码的 H.264 Annex-B 流经 WebSocket 推给浏览器，前端用 WebCodecs 解码渲染。容器零新增运行时依赖（不 fork scrcpy、不引入 Node/ffmpeg），复用 vanilla scrcpy-server。 |
| **录制停止用 SIGINT，绝不 SIGKILL** | scrcpy 写出 MP4 收尾索引（moov）只在正常退出路径执行。强杀会让录像缺索引变砖——这是桌面端 X10-95 用真机验证过的教训，NAS 端沿用。 |
| **scrcpy/SDL3/adb 的版本与哈希和 CI 完全一致** | 不在容器里引入第二套来源，避免两端跑不同版本的引擎。 |

## 已验证 / 未验证（如实记录）

**已真机验证（2026-10-11，macOS 实机 + 真实飞牛 fnOS 部署）：**
- `GET /api/devices` 正确识别已连接的手机，并给出与桌面端一致的首选通道判定（USB）
- 录制启停产出 **156 KB 的 MP4，moov 索引完整，可正常播放**
- `GET /api/screenshot` 真机截图成功（PNG）
- `GET /api/files` 正确读出 `/sdcard` 目录
- 路径穿越防护生效（`../etc/passwd` 被拒）
- **飞牛 fnOS compose 部署成功**，浏览器 `http://<NAS IP>:8080` 访问正常
- **USB 透传验证通过**：真机在 NAS 容器内被识别（首选 USB / ready），
  录制启停真机跑通
- **连续录制 15 分钟通过**（2026-10-11 上午，飞牛真机）：启停、落盘、播放全正常
- **录制停止链路（真容器端到端门禁）**：`scripts/nas-record-stop-gate.sh`
  红绿对照三组全通过——旧镜像 15.0s 超时报错（复现故障）/ 新镜像 0.8s 成功且
  产物完整 / 忽略信号的对照组如实报错

**待真机验证（沙箱与当前环境无法覆盖）：**
- **实时镜像端到端**：协议解析与视频扇出已过合成流集成测试（fake scrcpy-server），
  但「浏览器真看到画面 + 触控真注入手机」需真机点验。重点验：H.264 Annex-B 解码
  是否出画面（若黑屏多为 codec profile 声明或 SPS/PPS 时机问题）、触控坐标是否对准、
  多标签页是否共看一路。
- 超过 15 分钟的长时间录制稳定性（已在 15 分钟量级验证通过；24h 待跑）
- 磁盘规划：按码率上限估算，Balanced(8M) ≈ 3.6 GB/小时 ≈ 86 GB/天，
  Smooth(2M) ≈ 22 GB/天，Sharp(16M) ≈ 172 GB/天。实际用 VBR，
  静态画面远低于上限——请以实测文件大小为准
- 无线调试（mDNS 发现）在飞牛面板网络环境下的表现

## 实时镜像接口

| 接口 | 方法 | 说明 |
|---|---|---|
| `/api/stream/ws?serial=<序列号>&max_size=<边长>` | GET（WS 升级） | 实时镜像通道。视频/会话事件经文本帧 JSON 下行，触控/键鼠经文本帧 JSON 上行 |
| `/api/stream/stop?serial=<序列号>` | POST | 主动停止某设备镜像（关广播，设备端 server 随连接断开退出） |

## 文件 / 应用 / 截图接口（P1）

| 接口 | 方法 | 说明 |
|---|---|---|
| `/api/files?serial=&path=` | GET | 列出手机目录 |
| `/api/files/upload?serial=&path=` | POST(multipart) | 上传文件到手机目录（字段名 `file`，≤2GB） |
| `/api/files/download?serial=&remote=` | GET | 从手机下载文件（经 NAS inbox 落盘，返回可下载 URL） |
| `/api/files/delete?serial=&remote=` | POST | 删除手机上一个文件 |
| `/api/apps?serial=&system=1` | GET | 应用列表（默认只第三方；`system=1` 含系统） |
| `/api/apps/install?serial=` | POST(multipart) | 上传 `.apk` 安装（字段名 `file`） |
| `/api/screenshots` | GET | 截图画廊列表 |
| `/api/screenshots/{name}` | DELETE | 删除单张截图 |

WS 报文详见 `server/src/mirror_ws.rs` 顶部注释。视频为 H.264 Annex-B 裸流，
base64 装在 `video` 帧的 `data` 字段；前端参考实现见 `web/index.html`。

> 浏览器需支持 WebCodecs（较新的 Chrome/Edge）。Firefox/Safari 对 H.264
> VideoDecoder 支持尚不完整，可能无法显示画面。

## 环境变量

| 变量 | 默认值 | 说明 |
|---|---|---|
| `PORT` | `8080` | Web 端口 |
| `MIRRORDOCK_DATA` | `/var/lib/mirrordock` | 数据目录（录像/截图/adb 密钥） |
| `MIRRORDOCK_WEB` | `/opt/mirrordock/web` | 前端静态站目录 |
| `MIRRORDOCK_RETENTION_DAYS` | `7` | 归档保留天数。`0` 表示不自动清理；非整数值按「不清理」处理（清错不可逆，磁盘满只是可恢复告警）。每 6 小时扫一次 `recordings/*.mp4` 与 `screenshots/*.png`，删除超过 N 天的文件 |
| `ADB` | `/opt/scrcpy/adb` | adb 路径 |
| `SCRCPY` | `/opt/scrcpy/scrcpy` | scrcpy 路径 |

## 本地开发

```bash
# 服务端（可用本机 adb/scrcpy 覆盖路径）
cd nas/server && cargo run -p mirrordock-nas

# 单元测试
cargo test -p mirrordock-nas
```
