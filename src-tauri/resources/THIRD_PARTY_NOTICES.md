# MirrorDock 随包第三方组件说明

本文件随 MirrorDock 安装包分发（打包时位于应用资源目录）。
仓库根目录的 `THIRD_PARTY_NOTICES.md` 与本文件内容同步维护。

## scrcpy（随包分发）

- **用途：** 启动已授权 Android 设备的镜像窗口。MirrorDock 以固定参数
  直接调用本目录内的 `scrcpy` 可执行文件，不执行任何设备或网络提供的内容。
- **版本：** v4.1（开发基线固定）。
- **许可证：** Apache License 2.0。本目录随包携带的 `LICENSE` /
  `LICENSE.txt` 即 scrcpy 官方包内的许可证原件。
- **来源与完整性：** 构建流水线从 GitHub Releases（Genymobile/scrcpy）
  下载官方包，**SHA-256 校验通过后才打包**。三个平台的校验值见仓库根
  `THIRD_PARTY_NOTICES.md` 的哈希表；任何哈希不符都会直接终止构建。
- **平台差异：** Windows 与 macOS 官方包同时附带 `adb`；Linux 无官方
  预编译包，Linux 安装包不附带 scrcpy/adb，使用系统包管理器安装的版本
  （应用会自动回退到 PATH 查找）。

## Android adb（随包分发，仅 Windows / macOS）

`adb` 来自 scrcpy 官方包（见上），随 scrcpy 一起完成哈希校验与分发。
Android SDK platform-tools 本身以 Apache License 2.0 授权。
