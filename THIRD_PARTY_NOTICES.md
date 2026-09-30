# Third-party notices

## scrcpy

- **用途：** MirrorDock 使用 `scrcpy` 进程启动已授权 Android 设备的镜像窗口。以固定 `--serial <serial>` 参数直接执行，不使用 shell，不执行设备或网络提供的内容。
- **上游：** [Genymobile/scrcpy](https://github.com/Genymobile/scrcpy)
- **开发基线：** v4.1（固定）。**Windows 与 macOS 安装包随包分发该版本**；Linux 自 v0.3.0 起同样随包分发——由构建流水线从官方源码（固定 tag + 固定源码 SHA-256）编译，配套 scrcpy-server（按 GitHub 官方 release 元数据中的 SHA-256 digest 校验）与 Google platform-tools 的 adb（固定版本 + 固定 SHA-256），校验不符即终止构建。**scrcpy 4.x 依赖 SDL3**：构建流水线一并从官方源码（release-3.4.16 + 固定 SHA-256）编译；SDL3 与 FFmpeg 等运行时动态库不随包，由用户的发行版提供（见 README 的 Linux 运行库说明）。
- **许可证：** Apache License 2.0。随包分发的安装包内置 scrcpy 官方包内的 `LICENSE` 原件与 `THIRD_PARTY_NOTICES.md` 说明。
- **供应链校验（构建流水线强制执行，哈希不符即终止构建）：**

  | 平台 | 官方包 | SHA-256 |
  | --- | --- | --- |
  | Windows x64 | `scrcpy-win64-v4.1.zip` | `5b12172b3264b2889f4583ee64752ce832e29bc8b1089dca81093459697165db` |
  | macOS arm64 | `scrcpy-macos-aarch64-v4.1.tar.gz` | `20fd47c9014dd5e0fa77091f3cb7adbda8445a360c4584aeaa0150b5b3988ff3` |
  | macOS x86_64 | `scrcpy-macos-x86_64-v4.1.tar.gz` | `ee2a7223bc8dbdc4f482db1134bcf441178dafb833492b71ca4c22090c58ce72` |

  升级 scrcpy 版本时必须同时更新流水线中的 URL 与哈希，并在真机上重新验证镜像、输入、音频与许可证义务。
- **adb：** Windows / macOS 官方包内附带 `adb`（随上表同一校验链分发）；Android platform-tools 以 Apache-2.0 授权。
- **运行时查找顺序：** ① 显式 `MIRRORDOCK_SCRCPY_PATH` / `MIRRORDOCK_ADB`；② 开发模式下的 `.tools/scrcpy`；③ 随包资源目录（发行）；④ PATH。
- **签名状态（如实记录）：** MirrorDock 自身安装包当前未做代码签名（无证书），macOS 首次打开会提示未识别开发者、Windows 会触发 SmartScreen；scrcpy 供应链的完整性由上表哈希校验保证。正式分发前需按渠道补齐签名（A1-08 后续 / 渠道合规材料）。
