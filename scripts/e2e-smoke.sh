#!/usr/bin/env bash
# MirrorDock 端到端冒烟：在真机上验证 adb 数据通道的三条链路。
# 用法: scripts/e2e-smoke.sh <serial> [--with-recording]
#   --with-recording  额外跑一次 5 秒无窗录制，并校验 MP4 的 moov 索引存在。
# 报告写入 test-runs/e2e-smoke-<时间戳>.txt（该目录不入 Git）。
set -u

SERIAL="${1:-}"
WITH_RECORDING="${2:-}"
if [ -z "$SERIAL" ]; then
  echo "用法: $0 <serial> [--with-recording]" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ADB="${MIRRORDOCK_ADB:-adb}"
SCRCPY="${MIRRORDOCK_SCRCPY_PATH:-$ROOT/.tools/scrcpy/macos-x86_64/scrcpy}"
OUT_DIR="$ROOT/test-runs"
STAMP="$(date +%Y%m%d-%H%M%S)"
REPORT="$OUT_DIR/e2e-smoke-$STAMP.txt"
DEVICE_DIR="/sdcard/Download/MirrorDock"
mkdir -p "$OUT_DIR"

pass=0
fail=0
note() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$REPORT"; }
check() { # check <描述> <命令...>
  local desc="$1"; shift
  if "$@" >>"$REPORT" 2>&1; then
    note "PASS  $desc"; pass=$((pass + 1))
  else
    note "FAIL  $desc"; fail=$((fail + 1))
  fi
}

note "=== MirrorDock E2E 冒烟 ($STAMP) ==="
note "设备: $SERIAL"

# 1. 设备在线且已授权
state="$("$ADB" -s "$SERIAL" get-state 2>/dev/null)"
if [ "$state" = "device" ]; then
  note "PASS  设备状态为 device（已授权）"; pass=$((pass + 1))
else
  note "FAIL  设备状态为 '$state'，需要 device（检查授权与连接）"; fail=$((fail + 1))
fi

# 2. 截图链路：exec-out screencap 返回完整 PNG（校验 8 字节文件头）
TMP_SHOT="$(mktemp /tmp/mirrordock-e2e-XXXXXX.png)"
"$ADB" -s "$SERIAL" exec-out screencap -p > "$TMP_SHOT" 2>>"$REPORT"
header="$(xxd -p -l 8 "$TMP_SHOT" 2>/dev/null)"
if [ "$header" = "89504e470d0a1a0a" ]; then
  note "PASS  截图链路：PNG 文件头完整（$(wc -c < "$TMP_SHOT" | tr -d ' ') 字节）"; pass=$((pass + 1))
else
  note "FAIL  截图链路：PNG 文件头异常（$header），疑似二进制被改写"; fail=$((fail + 1))
fi

# 3. 传输链路：mkdir → push（中文名）→ ls 可见 → pull 回来 → md5 一致
SRC="$(mktemp -d)/MirrorDock 传输 冒烟.txt"
printf 'mirrordock e2e %s\n' "$STAMP" > "$SRC"
"$ADB" -s "$SERIAL" shell mkdir -p "$DEVICE_DIR" 2>>"$REPORT"
check "传输链路：push（中文与空格文件名）" "$ADB" -s "$SERIAL" push "$SRC" "$DEVICE_DIR/"
listing="$("$ADB" -s "$SERIAL" shell ls -1 "$DEVICE_DIR" 2>/dev/null | tr -d '\r')"
if printf '%s\n' "$listing" | grep -qF "MirrorDock 传输 冒烟.txt"; then
  note "PASS  传输链路：设备端列表可见"; pass=$((pass + 1))
else
  note "FAIL  传输链路：设备端列表缺少推送文件（$listing）"; fail=$((fail + 1))
fi
DST="$(mktemp /tmp/mirrordock-e2e-pull-XXXXXX)"
if "$ADB" -s "$SERIAL" pull "$DEVICE_DIR/MirrorDock 传输 冒烟.txt" "$DST" >>"$REPORT" 2>&1; then
  if [ "$(md5 -q "$SRC")" = "$(md5 -q "$DST")" ]; then
    note "PASS  传输链路：pull 回读 md5 一致"; pass=$((pass + 1))
  else
    note "FAIL  传输链路：pull 回读 md5 不一致"; fail=$((fail + 1))
  fi
else
  note "FAIL  传输链路：pull 失败"; fail=$((fail + 1))
fi
# 清理设备端测试痕迹
"$ADB" -s "$SERIAL" shell rm "$DEVICE_DIR/MirrorDock 传输 冒烟.txt" >/dev/null 2>&1

# 4.（可选）录制链路：5 秒无窗录制，SIGTERM 收尾，校验 moov 索引存在
if [ "$WITH_RECORDING" = "--with-recording" ]; then
  if [ ! -x "$SCRCPY" ]; then
    note "FAIL  录制链路：找不到 scrcpy（$SCRCPY）"; fail=$((fail + 1))
  else
    REC="$(mktemp /tmp/mirrordock-e2e-XXXXXX.mp4)"
    "$SCRCPY" -s "$SERIAL" --no-window --no-control --no-audio \
      --record="$REC" --record-format=mp4 >>"$REPORT" 2>&1 &
    PID=$!
    sleep 5
    kill -TERM $PID 2>/dev/null
    wait $PID 2>/dev/null
    if grep -aq moov "$REC"; then
      note "PASS  录制链路：SIGTERM 收尾后 moov 索引存在（$(wc -c < "$REC" | tr -d ' ') 字节）"; pass=$((pass + 1))
    else
      note "FAIL  录制链路：moov 索引缺失，文件无法播放"; fail=$((fail + 1))
    fi
    rm -f "$REC"
  fi
fi

rm -rf "$(dirname "$SRC")" "$DST" "$TMP_SHOT" 2>/dev/null
note "=== 结果: $pass 通过 / $fail 失败，报告: $REPORT ==="
[ "$fail" -eq 0 ]
