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
# 平台目录映射：
#   macOS (x86_64 / aarch64)  .tools/scrcpy/macos-x86_64 或 macos-aarch64
#   Windows                   .tools/scrcpy/windows-x86_64（如已准备）
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$REPO_ROOT/src-tauri/resources/scrcpy"

case "$(uname -s)-$(uname -m)" in
  Darwin-x86_64) SRC="$REPO_ROOT/.tools/scrcpy/macos-x86_64" ;;
  Darwin-arm64)  SRC="$REPO_ROOT/.tools/scrcpy/macos-aarch64" ;;
  MINGW*|MSYS*|Windows*) SRC="$REPO_ROOT/.tools/scrcpy/windows-x86_64" ;;
  Linux*) SRC="$REPO_ROOT/.tools/scrcpy/linux-x86_64" ;;
  *) echo "prepare-runtime: 未知平台 $(uname -s)-$(uname -m)，跳过（运行时将回退 PATH）"; exit 0 ;;
esac

if [ ! -d "$SRC" ]; then
  echo "prepare-runtime: 找不到本机运行时 $SRC" >&2
  echo "  请先按供应链流程准备 .tools/scrcpy（官方包 + SHA-256 校验）。" >&2
  exit 1
fi

mkdir -p "$DEST"
# 全量同步官方包内容（scrcpy、scrcpy-server、adb 及附带文件）。
rm -rf "${DEST:?}"/*
cp -R "$SRC"/. "$DEST"/
echo "prepare-runtime: 已复制 $(ls "$DEST" | wc -l | tr -d ' ') 个文件 -> ${DEST#"$REPO_ROOT"/}"
