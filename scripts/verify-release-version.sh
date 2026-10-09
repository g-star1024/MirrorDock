#!/usr/bin/env bash
# X10-98：tag 发版后核验「产物文件名版本」与 tag 一致。
#
# 为什么查文件名而不是二进制本体：version 只在 macOS 以 plist
# （CFBundleShortVersionString）嵌入二进制；Linux/Windows 的 version 只在
# bundle 产物文件名/元数据（deb 控制文件、msi 属性、文件名），不在 .bin/.exe
# 本体——对全平台 grep 二进制 version 串必然找不到（两轮 fail 的真因，非错版）。
#
# 用法: verify-release-version.sh <tag> <diag-log-path>
#   tag          例 v0.4.19-beta（会剥离 v 前缀与 -beta 后缀得 0.4.19）
#   diag-log     诊断输出路径（同时 echo 到 stdout 与写文件）
#
# 退出码: 0 = 全部产物文件名含版本（或未找到可核验产物，仅警告）;
#         1 = 存在版本不符的产物（真错版）。

# 强制 UTF-8 无关的 C locale，避免 runner 默认 locale 对 multibyte 的处理差异。
export LC_ALL=C

TAG="${1:?usage: verify-release-version.sh <tag> <diag-log>}"
DIAG="${2:?usage: verify-release-version.sh <tag> <diag-log>}"

EXPECT="${TAG#v}"        # v0.4.19-beta -> 0.4.19-beta
EXPECT="${EXPECT%%-*}"   # -> 0.4.19

{
  echo "== version check diag =="
  echo "TAG=$TAG EXPECT=$EXPECT"
  echo "== bundle artifacts produced =="
  find src-tauri/target -path '*/release/bundle/*' -type f \
    \( -name '*.dmg' -o -name '*.msi' -o -name '*.exe' -o -name '*.AppImage' \
       -o -name '*.deb' -o -name '*.rpm' -o -name '*.app.tar.gz' \) 2>/dev/null
} | tee "$DIAG"

# 只核验「文件名本就该带版本」的产物类型：dmg/msi/setup.exe/AppImage/deb/rpm——
# tauri 按 conf version 命名 MirrorDock_<ver>_*。.app.tar.gz（macOS updater
# 产物）文件名不带版本（版本在内部 .app），及其 .sig，均不纳入，避免误报。
ARTS=()
while IFS= read -r f; do ARTS+=("$f"); done < <(
  find src-tauri/target -path '*/release/bundle/*' -type f \
    \( -name '*.dmg' -o -name '*.msi' -o -name '*setup.exe' -o -name '*.AppImage' \
       -o -name '*.deb' -o -name '*.rpm' \) 2>/dev/null
)

if [ "${#ARTS[@]}" -eq 0 ]; then
  echo "::warning::未找到 bundle 产物文件名可核验（防错版由禁用缓存保障）" | tee -a "$DIAG"
  exit 0
fi

BAD=0
for f in "${ARTS[@]}"; do
  base="$(basename "$f")"
  case "$base" in
    *"$EXPECT"*) echo "OK  $base" | tee -a "$DIAG" ;;
    *) echo "::error::产物文件名不含版本 $EXPECT：$base" | tee -a "$DIAG"; BAD=1 ;;
  esac
done

if [ "$BAD" -ne 0 ]; then
  echo "::error::存在版本不符的产物（X10-98）。" | tee -a "$DIAG"
  exit 1
fi
echo "产物版本核验通过（${#ARTS[@]} 个文件全含 $EXPECT）" | tee -a "$DIAG"
exit 0
