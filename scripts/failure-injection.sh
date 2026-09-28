#!/usr/bin/env bash
# MirrorDock B2-03 失败注入：在真机上验证 adb/scrcpy 传输通道对可注入故障的行为。
#
# 覆盖场景（可自动化子集）：
#   A. ADB 服务重启（adb kill-server）——运行中的镜像会话应退出而非挂死，恢复后可重新启动
#   B. 无线通道断开（adb disconnect）——无线镜像会话应退出
#   C. 锁屏（KEYCODE_POWER）——镜像会话不终止；唤醒后恢复
#
# 不在本脚本范围（需物理配合，见 DEVELOPMENT_TASKS.md B2-03 说明）：
#   - USB 拔线、撤销 USB 调试授权、FLAG_SECURE 保护内容页面
#
# 用法: scripts/failure-injection.sh <serial>
# 报告写入 test-runs/failure-injection-<时间戳>.txt（Git 忽略）。
set -u

SERIAL="${1:-}"
if [ -z "$SERIAL" ]; then
  echo "用法: $0 <serial>" >&2
  exit 2
fi
case "$SERIAL" in
  *[!A-Za-z0-9._:-]*|'') echo "序列号包含不支持的字符。" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ADB="${MIRRORDOCK_ADB:-adb}"
SCRCPY="${MIRRORDOCK_SCRCPY_PATH:-$ROOT/.tools/scrcpy/macos-x86_64/scrcpy}"
OUT_DIR="$ROOT/test-runs"
STAMP="$(date +%Y%m%d-%H%M%S)"
REPORT="$OUT_DIR/failure-injection-$STAMP.txt"
mkdir -p "$OUT_DIR"
TMPD="$(mktemp -d)" || exit 70
trap 'rm -rf "$TMPD"' EXIT

pass=0
fail=0
note() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$REPORT"; }
check() {
  local desc="$1"; shift
  if "$@" >>"$REPORT" 2>&1; then
    note "PASS  $desc"; pass=$((pass + 1))
  else
    note "FAIL  $desc"; fail=$((fail + 1))
  fi
}

note "=== MirrorDock 失败注入 ($STAMP) ==="
note "设备: $SERIAL"

state="$("$ADB" -s "$SERIAL" get-state 2>/dev/null)"
if [ "$state" = "device" ]; then
  note "PASS  前置：设备已授权在线"; pass=$((pass + 1))
else
  note "FAIL  前置：设备状态 '$state'，中止"; fail=$((fail + 1))
  note "=== 结果: $pass 通过 / $fail 失败，报告: $REPORT ==="
  exit 1
fi

wait_proc_exit() { # wait_proc_exit <pid> <最多秒数>
  local pid="$1" limit="$2" waited=0
  while kill -0 "$pid" 2>/dev/null; do
    sleep 1
    waited=$((waited + 1))
    [ "$waited" -ge "$limit" ] && return 1
  done
  return 0
}

# ---------------------------------------------------------------- 场景 A：ADB 服务重启
note "--- 场景 A：运行中重启 ADB 服务 ---"
REC_A="$TMPD/fi-A.mp4"
"$SCRCPY" -s "$SERIAL" --no-window --no-control --no-audio \
  --record="$REC_A" --record-format=mp4 >>"$REPORT" 2>&1 &
PID_A=$!
sleep 4
if kill -0 "$PID_A" 2>/dev/null; then
  note "PASS  场景 A：镜像会话已启动（PID ${PID_A}）"; pass=$((pass + 1))
else
  note "FAIL  场景 A：镜像会话未启动"; fail=$((fail + 1))
fi
T0=$(date +%s)
"$ADB" kill-server >>"$REPORT" 2>&1
if wait_proc_exit "$PID_A" 30; then
  T1=$(date +%s)
  note "PASS  场景 A：ADB 服务被杀后镜像会话在 $((T1 - T0))s 内退出（不挂死）"; pass=$((pass + 1))
else
  kill -9 "$PID_A" 2>/dev/null
  note "FAIL  场景 A：ADB 服务被杀 30s 后会话仍存活（挂死风险）"; fail=$((fail + 1))
fi
"$ADB" start-server >>"$REPORT" 2>&1
sleep 2
check "场景 A：ADB 服务恢复后设备重新可见" "$ADB" -s "$SERIAL" get-state
if [ "$("$ADB" -s "$SERIAL" get-state 2>/dev/null)" = "device" ]; then
  note "PASS  场景 A：恢复后状态仍为 device（授权保留）"; pass=$((pass + 1))
else
  note "FAIL  场景 A：恢复后状态异常（$("$ADB" -s "$SERIAL" get-state 2>/dev/null || echo offline)）"; fail=$((fail + 1))
fi
if grep -aq moov "$REC_A" 2>/dev/null; then
  note "PASS  场景 A：故障前录制段 moov 完整（$(wc -c < "$REC_A" | tr -d ' ') 字节）"; pass=$((pass + 1))
else
  note "INFO  场景 A：故障导致录制无 moov（进程被通道故障终止，非正常收尾，符合预期边界）"
fi
rm -f "$REC_A"

# ---------------------------------------------------------------- 场景 B：无线通道断开
WIRELESS="$("$ADB" devices | awk '/_adb-tls-connect/ && $2 == "device" {print $1; exit}')"
if [ -n "$WIRELESS" ]; then
  note "--- 场景 B：无线通道断开（${WIRELESS}）---"
  REC_B="$TMPD/fi-B.mp4"
  "$SCRCPY" -s "$WIRELESS" --no-window --no-control --no-audio \
    --record="$REC_B" --record-format=mp4 >>"$REPORT" 2>&1 &
  PID_B=$!
  sleep 4
  if kill -0 "$PID_B" 2>/dev/null; then
    note "PASS  场景 B：无线镜像会话已启动"; pass=$((pass + 1))
  else
    note "FAIL  场景 B：无线镜像会话未启动"; fail=$((fail + 1))
  fi
  "$ADB" disconnect "$WIRELESS" >>"$REPORT" 2>&1
  if wait_proc_exit "$PID_B" 30; then
    note "PASS  场景 B：无线断开后会话退出（不挂死）"; pass=$((pass + 1))
  else
    kill -9 "$PID_B" 2>/dev/null
    note "FAIL  场景 B：无线断开 30s 后会话仍存活（挂死风险）"; fail=$((fail + 1))
  fi
  sleep 2
  # 重新建立无线连接以恢复双通道
  ip_port="${WIRELESS#adb-}"
  ip_port="${ip_port%._adb-tls-connect._tcp}"
  if "$ADB" connect "$ip_port" >>"$REPORT" 2>&1; then
    note "PASS  场景 B：无线通道重连（${ip_port}）"; pass=$((pass + 1))
  else
    note "INFO  场景 B：无线重连失败（不影响 USB 通道，可在设备端重新触发）"
  fi
  rm -f "$REC_B"
else
  note "--- 场景 B：跳过（当前无已连接的无线通道）---"
fi

# ---------------------------------------------------------------- 场景 C：锁屏与恢复
note "--- 场景 C：锁屏期间镜像存活与唤醒恢复 ---"
REC_C="$TMPD/fi-C.mp4"
"$SCRCPY" -s "$SERIAL" --no-window --no-control --no-audio \
  --record="$REC_C" --record-format=mp4 --time-limit=25 >>"$REPORT" 2>&1 &
PID_C=$!
sleep 4
"$ADB" -s "$SERIAL" shell input keyevent KEYCODE_POWER >>"$REPORT" 2>&1
sleep 3
if kill -0 "$PID_C" 2>/dev/null; then
  note "PASS  场景 C：锁屏后镜像会话未终止"; pass=$((pass + 1))
else
  note "FAIL  场景 C：锁屏导致镜像会话终止"; fail=$((fail + 1))
fi
# 唤醒并尝试解锁（无 PIN 设备可滑屏解锁；有 PIN 需人工配合）
"$ADB" -s "$SERIAL" shell input keyevent KEYCODE_WAKEUP >>"$REPORT" 2>&1
sleep 1
"$ADB" -s "$SERIAL" shell input keyevent 82 >>"$REPORT" 2>&1
sleep 1
"$ADB" -s "$SERIAL" shell wm dismiss-keyguard >>"$REPORT" 2>&1
wait "$PID_C" 2>/dev/null
if grep -aq moov "$REC_C"; then
  note "PASS  场景 C：包含锁屏区段的录制 moov 完整（time-limit 自然收尾，$(wc -c < "$REC_C" | tr -d ' ') 字节）"; pass=$((pass + 1))
else
  note "FAIL  场景 C：录制 moov 缺失"; fail=$((fail + 1))
fi
rm -f "$REC_C"
wakefulness="$("$ADB" -s "$SERIAL" shell dumpsys power 2>/dev/null | grep -m1 'mWakefulness=' | tr -d '\r')"
note "场景 C：唤醒后电源状态 ${wakefulness}（若仍 Sleeping 请手动点亮并解锁设备）"

note "=== 结果: $pass 通过 / $fail 失败，报告: $REPORT ==="
[ "$fail" -eq 0 ]
