# MirrorDock

MirrorDock 是一个本地优先的 Android 桌面镜像工具，目标是让非技术用户在 Windows、macOS 和 Linux 上安全连接自己的手机。

当前处于第 0 阶段 POC。画面、音频和控制仅在电脑与已授权手机之间传输；不会默认上传到云端。

## 当前可运行能力

- 检测 ADB 设备、USB 调试授权和离线状态，并给出中文恢复说明。
- 仅对已授权设备通过固定参数直接启动 scrcpy。
- Android 11+ 无线调试：一次性配对码、独立连接地址、本机可信设备的重新连接与忘记。

无线连接需要手机与电脑处于同一 Wi-Fi。手机“无线调试”页面中的配对地址和连接地址可能不同；配对码不会保存或写入日志。

## 本地开发

前置条件：Node.js、pnpm、Rust、Tauri 桌面依赖，以及 Android Platform Tools（`adb`）。

```sh
pnpm install
pnpm tauri dev
```

开发检查：

```sh
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
```

开发期 scrcpy 运行时位于忽略的 `.tools/`；发行版打包、签名、SBOM 和三渠道验证尚未完成，详见 `DEVELOPMENT_TASKS.md`。

## 工程约束

每次开发前先阅读 [AGENTS.md](AGENTS.md) 与 [DEVELOPMENT_TASKS.md](DEVELOPMENT_TASKS.md)。产品边界、发布要求与团队协作规则以 AGENTS.md 为准；内部产品规划文档维护于仓库外、不随本仓库分发，外部贡献者请以 README 与 DEVELOPMENT_TASKS.md 为准。本项目以 [Apache-2.0](LICENSE) 许可发布。
