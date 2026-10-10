# MirrorDock NAS 版

立项场景：**NAS 常开机 + USB 直连 + 24h 录制归档 + 远程 Web 访问**。

把手机长期挂在 NAS 上，用浏览器随时看、随时录，录像直接落在 NAS 的大容量存储池里。

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
| **不提供实时画面** | 容器没有显示器，scrcpy 的 SDL 窗口无法开窗。本版用「录制 + 截图」覆盖远程查看需求；实时画面需要实装 scrcpy 协议转发（WebCodecs），已登记为后续任务。 |
| **录制停止用 SIGINT，绝不 SIGKILL** | scrcpy 写出 MP4 收尾索引（moov）只在正常退出路径执行。强杀会让录像缺索引变砖——这是桌面端 X10-95 用真机验证过的教训，NAS 端沿用。 |
| **scrcpy/SDL3/adb 的版本与哈希和 CI 完全一致** | 不在容器里引入第二套来源，避免两端跑不同版本的引擎。 |

## 已验证 / 未验证（如实记录）

**已真机验证（2026-10-11，macOS + 实机）：**
- `GET /api/devices` 正确识别已连接的手机，并给出与桌面端一致的首选通道判定（USB）
- 录制启停产出 **156 KB 的 MP4，moov 索引完整，可正常播放**
- `GET /api/screenshot` 真机截图成功（PNG）
- `GET /api/files` 正确读出 `/sdcard` 目录
- 路径穿越防护生效（`../etc/passwd` 被拒）

**待真机验证（沙箱与当前环境无法覆盖）：**
- USB 设备透传进容器后 adb 能否稳定识别（需要真 NAS + 真机）
- 飞牛 fnOS 的 Docker 面板实际部署表现
- 24 小时连续录制的稳定性与磁盘占用
- 手机侧首次 RSA 授权在无显示器场景下的完整流程

## 环境变量

| 变量 | 默认值 | 说明 |
|---|---|---|
| `PORT` | `8080` | Web 端口 |
| `MIRRORDOCK_DATA` | `/var/lib/mirrordock` | 数据目录（录像/截图/adb 密钥） |
| `MIRRORDOCK_WEB` | `/opt/mirrordock/web` | 前端静态站目录 |
| `ADB` | `/opt/scrcpy/adb` | adb 路径 |
| `SCRCPY` | `/opt/scrcpy/scrcpy` | scrcpy 路径 |

## 本地开发

```bash
# 服务端（可用本机 adb/scrcpy 覆盖路径）
cd nas/server && cargo run -p mirrordock-nas

# 单元测试
cargo test -p mirrordock-nas
```
