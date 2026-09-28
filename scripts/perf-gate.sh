#!/usr/bin/env bash
# MirrorDock B2-04 性能/稳定性门禁。
#
# 子命令:
#   perf-gate.sh <serial> first-frame [轮数]   多轮测量「启动→视频隧道建立」耗时，输出 P50/P95
#   perf-gate.sh <serial> soak [分钟]          无窗录制长时会话（默认 60 分钟），校验进程存活与 moov 完整
#
# 首帧口径（如实记录）：测量从 scrcpy 进程启动到视频隧道 TCP 连接建立的时间差，
# 是可见首帧的乐观下界代理（不含解码渲染，macOS 无头模式无法捕获窗口首帧）。
# 已实测：scrcpy 的 mp4/mkv 封装在缓冲满或收尾前不落盘，文件增长不可作首帧信号。
# 报告写入 test-runs/perf-<子命令>-<时间戳>.txt（Git 忽略）。
set -u

SERIAL="${1:-}"
SUBCMD="${2:-}"
if [ -z "$SERIAL" ] || [ -z "$SUBCMD" ]; then
  echo "用法: $0 <serial> first-frame [轮数] | soak [分钟]" >&2
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
mkdir -p "$OUT_DIR"
TMPD="$(mktemp -d)" || exit 70
trap 'rm -rf "$TMPD"' EXIT

state="$("$ADB" -s "$SERIAL" get-state 2>/dev/null)"
if [ "$state" != "device" ]; then
  echo "设备状态 '$state'，需要 device（已授权在线）。" >&2
  exit 69
fi

case "$SUBCMD" in
  first-frame)
    ROUNDS="${3:-10}"
    REPORT="$OUT_DIR/perf-first-frame-$STAMP.txt"
    note() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$REPORT"; }
    note "=== 首帧测量 ($STAMP) 设备 ${SERIAL}，$ROUNDS 轮 ==="
    note "口径: 进程启动 -> 视频隧道 TCP 连接建立（lsof ESTABLISHED，排除 adb server:5037）。"
    note "这是首帧的乐观下界代理：不含解码渲染时间（macOS 无头模式无法捕获窗口首帧）；"
    note "scrcpy 的 mp4/mkv 封装在缓冲满或收尾前不落盘，文件增长不可作首帧信号（已实测）。"
    times=()
    for i in $(seq 1 "$ROUNDS"); do
      REC="$TMPD/ff-$i.mp4"
      T0=$(python3 -c 'import time; print(time.time())')
      "$SCRCPY" -s "$SERIAL" --no-window --no-control --no-audio \
        --record="$REC" --record-format=mp4 >>"$REPORT" 2>&1 &
      PID=$!
      elapsed=""
      dead=0
      # 轮询隧道连接，~0.2s 粒度（lsof 开销），上限 15s
      for _ in $(seq 1 75); do
        if ! kill -0 "$PID" 2>/dev/null; then dead=1; break; fi
        out=$(lsof -nP -p "$PID" 2>/dev/null | grep ESTABLISHED | grep -v ':5037')
        if [ -n "$out" ]; then
          T1=$(python3 -c 'import time; print(time.time())')
          elapsed=$(python3 -c "print(f'{$T1 - $T0:.2f}')")
          break
        fi
        sleep 0.05
      done
      kill -TERM "$PID" 2>/dev/null
      wait "$PID" 2>/dev/null
      if [ -n "$elapsed" ]; then
        note "第 $i 轮: ${elapsed}s"
        times+=("$elapsed")
      else
        if [ "$dead" -eq 1 ]; then
          note "第 $i 轮: FAIL（scrcpy 进程在隧道建立前退出）"
        else
          note "第 $i 轮: FAIL（15s 内视频隧道未建立）"
        fi
        times+=("999")
      fi
      sleep 1
    done
    python3 - "$REPORT" "${times[@]}" <<'PYEOF'
import sys
vals = sorted(float(v) for v in sys.argv[2:])
ok = [v for v in vals if v < 999]
def pct(p):
    if not ok: return float('nan')
    k = max(0, min(len(ok) - 1, int(round(p / 100 * len(ok) + 0.5)) - 1))
    return ok[k]
with open(sys.argv[1], 'a') as f:
    f.write(f"\n样本 {len(ok)}/{len(vals)} 有效\n")
    f.write(f"P50={pct(50):.2f}s  P95={pct(95):.2f}s  max={max(ok) if ok else float('nan'):.2f}s\n")
    f.write(f"门禁: P95 < 5s -> {'PASS' if ok and pct(95) < 5 else 'FAIL'}\n")
print(open(sys.argv[1]).read().splitlines()[-3:])
PYEOF
    note "报告: $REPORT"
    ;;

  soak)
    MINUTES="${3:-60}"
    case "$MINUTES" in *[!0-9]|'') echo "分钟数须为正整数。" >&2; exit 2 ;; esac
    REPORT="$OUT_DIR/perf-soak-$STAMP.txt"
    REC="$TMPD/soak.mp4"
    note() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$REPORT"; }
    note "=== 稳定性 soak ($STAMP) 设备 ${SERIAL}，${MINUTES} 分钟 ==="
    "$SCRCPY" -s "$SERIAL" --no-window --no-control --no-audio \
      --record="$REC" --record-format=mp4 --time-limit=$((MINUTES * 60)) >>"$REPORT" 2>&1 &
    PID=$!
    # 每 60s 记录一次存活与文件大小
    for m in $(seq 1 "$MINUTES"); do
      sleep 60
      if kill -0 "$PID" 2>/dev/null; then
        sz=$(wc -c < "$REC" 2>/dev/null | tr -d ' ')
        note "第 ${m} 分钟: 进程存活，录制 ${sz:-0} 字节"
      else
        note "第 ${m} 分钟: FAIL 进程提前退出"
        break
      fi
    done
    wait "$PID" 2>/dev/null
    # 注意：macOS BSD grep 2.6.0-FreeBSD 的 -a 在二进制文件上反而不匹配（实测），
    # 故用不带 -a 的 grep -q（GNU/BSD 均在命中时返回 0，-q 下无输出）。
    if grep -q moov "$REC"; then
      note "PASS 会话全程完成，moov 索引完整（$(wc -c < "$REC" | tr -d ' ') 字节）"
      rc=0
    else
      note "FAIL moov 索引缺失或会话未完成"
      rc=1
    fi
    note "报告: $REPORT"
    exit $rc
    ;;

  *)
    echo "未知子命令: $SUBCMD" >&2
    exit 2
    ;;
esac
