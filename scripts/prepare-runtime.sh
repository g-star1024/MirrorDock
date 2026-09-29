#!/usr/bin/env bash
# 本地打包前置：把 scrcpy v4.1 运行时从 .tools/ 复制进 tauri bundle 资源目录。
#
# 背景（X10-19）：CI 打包作业会自行下载并校验 scrcpy 官方包再塞进
# src-tauri/resources/scrcpy/（A1-08），但本地 `pnpm tauri build` 没有这一步，
# 打出的 .app 缺 scrcpy/adb，镜像、连接等全部功能不可用。
# 本脚本复用本地已校验的 .tools 运行时，对齐 CI 的资源布局，使本地构建
# 与 CI 构建产物一致。
#
# 用法：在仓库根目录执行（由 tauri.conf.json beforeBuildCommand 自动调用）。
# 平台差异（对齐 A1-08）：
#   macOS / Windows  打包必须随带运行时：本地 .tools 缺失时若 DEST 已由
#                    外部（CI 下载步）准备好则跳过，否则报错终止。
#   Linux            无官方包，设计上不随带运行时，直接跳过（回退 PATH）。
#
# 注意：脚本必须兼容 macOS 自带的 bash 3.2——全角标点紧跟 $变量 会被
# 并入变量名导致 "unbound variable"（X10-19 实测），因此所有含变量的
# 输出一律使用 ASCII 标点。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$REPO_ROOT/src-tauri/resources/scrcpy"

OS_ARCH="$(uname -s)-$(uname -m)"
case "$OS_ARCH" in
  Darwin-x86_64)      SRC="$REPO_ROOT/.tools/scrcpy/macos-x86_64" ;;
  Darwin-arm64)       SRC="$REPO_ROOT/.tools/scrcpy/macos-aarch64" ;;
  MINGW*|MSYS*|Windows*) SRC="$REPO_ROOT/.tools/scrcpy/windows-x86_64" ;;
  Linux*)
    # A1-08：Linux 无官方包，不随带运行时，交给用户 PATH。
    echo "prepare-runtime: Linux build skips bundled runtime (PATH fallback per A1-08)"
    exit 0
    ;;
  *)
    echo "prepare-runtime: unknown platform $OS_ARCH, skipping (runtime falls back to PATH)"
    exit 0
    ;;
esac

if [ -d "$SRC" ]; then
  mkdir -p "$DEST"
  # 全量同步官方包内容（scrcpy、scrcpy-server、adb 及附带文件）。
  rm -rf "${DEST:?}"/*
  cp -R "$SRC"/. "$DEST"/
  echo "prepare-runtime: copied runtime from $SRC to $DEST"
  exit 0
fi

# CI 打包作业会先行下载官方运行时放进 DEST：此时无需本地 .tools。
if [ -f "$DEST/scrcpy" ] || [ -f "$DEST/scrcpy.exe" ]; then
  echo "prepare-runtime: runtime already prepared at $DEST, skip copying"
  exit 0
fi

echo "prepare-runtime: local runtime not found at $SRC" >&2
echo "  prepare .tools/scrcpy first (official package + SHA-256 check)." >&2
exit 1
