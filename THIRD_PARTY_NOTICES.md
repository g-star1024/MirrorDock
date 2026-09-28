# Third-party notices

## scrcpy

- **用途：** MirrorDock POC 使用本机已安装的 `scrcpy` 进程启动已授权 Android 设备的镜像窗口。当前采用进程调用，不嵌入、修改或分发其源代码/二进制。
- **上游：** [Genymobile/scrcpy](https://github.com/Genymobile/scrcpy)
- **开发基线：** v4.1；macOS x86_64 官方静态包 SHA-256 为 `ee2a7223bc8dbdc4f482db1134bcf441178dafb833492b71ca4c22090c58ce72`。下载后必须在解包前校验。
- **许可证：** Apache License 2.0。
- **运行时约束：** 开发模式只从经校验的 `.tools/scrcpy` 或显式 `MIRRORDOCK_SCRCPY_PATH` 查找；发行模式不得依赖开发目录。以固定 `--serial <serial>` 参数直接执行，不使用 shell，不执行设备或网络提供的内容。
- **发布前要求：** 固定并记录分发版本与哈希，将完整 Apache-2.0 许可证、适用 NOTICE 和 SBOM 随每个渠道发行物交付，并在升级后重新验证镜像、输入、音频与许可证义务。
