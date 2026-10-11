#!/usr/bin/env bash
# NAS 端「停止录制 → SIGINT 送达 → MP4 收尾」回归门禁。
#
# 【为什么需要这个脚本】
# 2026-10-11 飞牛真机实锤：代码用 Command::new("kill") 发 SIGINT，而 debian:12-slim
# 里 /usr/bin/kill 不存在（kill 只是 shell 内建命令，exec 不经过 shell）→ spawn 静默
# 失败 → 信号从未送达 → 15 秒后 SIGKILL → MP4 永远缺 moov、浏览器拒播。
#
# 这个 bug 在本地 macOS 上**测不出来**——macOS 有 /bin/kill。所以门禁必须跑在
# 与生产一致的容器里，而不是开发机上。
#
# 【做法】
# 用 shell 脚本冒充 scrcpy（解析 --record= 参数，收到 INT/TERM 后写 "MOOVATOM" 再退出），
# 通过 SCRCPY 环境变量注入，走完整的 /api/record/start → /api/record/stop 链路。
# 三组断言：
#   绿队：替身正常响应信号 → stop 秒回成功、产物含 MOOVATOM
#   红队：替身忽略 INT/TERM   → stop 必须报失败（证明门禁真的在测信号送达，
#                                而不是「进程恰好退出了」这类假阳性）
#
# 用法：
#   ./scripts/nas-record-stop-gate.sh [镜像 tag，默认 latest]
#
# 退出码 0 = 全通过；非 0 = 有断言失败。

set -uo pipefail

IMAGE="${1:-ghcr.io/g-star1024/mirrordock/mirrordock-nas:latest}"
WORK="$(mktemp -d)"
PORT_BASE="${PORT_BASE:-38700}"
PASS=0
FAIL=0

cleanup() {
  docker rm -f md-gate-green md-gate-red >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

if ! command -v docker >/dev/null 2>&1; then
  echo "❌ 需要 docker"; exit 2
fi
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "拉取镜像：$IMAGE"
  docker pull -q "$IMAGE" || { echo "❌ 镜像拉取失败：$IMAGE"; exit 2; }
fi

# ---- 替身 1：正常响应 INT/TERM，退出前写出 moov ----
cat > "$WORK/fake-scrcpy" <<'EOF'
#!/bin/sh
REC=""
for a in "$@"; do
  case "$a" in --record=*) REC="${a#--record=}";; esac
done
printf 'MP4HEADER' > "$REC"
trap 'printf "MOOVATOM" >> "$REC"; exit 0' INT TERM
while true; do sleep 1; done
EOF

# ---- 替身 2：刻意忽略 INT/TERM，只能被 SIGKILL 杀 ----
cat > "$WORK/fake-scrcpy-ignore" <<'EOF'
#!/bin/sh
REC=""
for a in "$@"; do
  case "$a" in --record=*) REC="${a#--record=}";; esac
done
printf 'MP4HEADER' > "$REC"
trap '' INT TERM
while true; do sleep 1; done
EOF

chmod +x "$WORK"/fake-scrcpy "$WORK"/fake-scrcpy-ignore

# 单次「录制→停止」往返。$1=替身文件名 $2=数据目录 $3=端口
run_case() {
  local fake="$1" datadir="$2" port="$3"
  docker rm -f "md-gate-$4" >/dev/null 2>&1 || true
  docker run -d --name "md-gate-$4" -p "${port}:8080" \
    -v "$WORK:$WORK" \
    -e "SCRCPY=$WORK/$fake" \
    -e "MIRRORDOCK_DATA=$datadir" \
    "$IMAGE" >/dev/null || return 1
  sleep 3
  curl -s --max-time 15 -X POST -H 'Content-Type: application/json' \
    -d '{"serial":"GATEDEV","quality":"balanced"}' \
    "http://127.0.0.1:${port}/api/record/start" >/dev/null || return 1
  sleep 1
  curl -s --max-time 60 -X POST "http://127.0.0.1:${port}/api/record/stop"
  return 0
}

assert() { # $1=描述 $2=条件是否满足
  if [ "$2" = "0" ]; then echo "  ✅ $1"; PASS=$((PASS + 1))
  else echo "  ❌ $1"; FAIL=$((FAIL + 1)); fi
}

# ================= 绿队 =================
echo "【绿队】替身正常响应 SIGINT → stop 应秒回成功，产物应含 MOOVATOM"
mkdir -p "$WORK/green/recordings"
RESP="$(run_case fake-scrcpy "$WORK/green" "$PORT_BASE" green)"
echo "  响应：$RESP"
echo "$RESP" | grep -q '"file"' ; assert "stop 返回成功（含 file 字段）" $?
echo "$RESP" | grep -q 'record_stop_failed' ; [ $? -ne 0 ] ; assert "未报 record_stop_failed" $?
CONTENT="$(cat "$WORK"/green/recordings/*.mp4 2>/dev/null)"
# 注意必须写 ${CONTENT} 而非 $CONTENT：macOS 自带 bash 3.2 在多字节字符前有
# 解析 bug，紧邻中文全角括号时它会把后续字节并进变量名，报「unbound variable」。
[ "$CONTENT" = "MP4HEADERMOOVATOM" ] ; assert "产物完成收尾（MP4HEADER+MOOVATOM，实际：${CONTENT}）" $?

# ================= 红队（反向验证） =================
echo
echo "【红队】替身忽略 INT/TERM → stop 必须报失败（证明门禁真在测信号送达）"
mkdir -p "$WORK/red/recordings"
RESP="$(run_case fake-scrcpy-ignore "$WORK/red" "$((PORT_BASE + 1))" red)"
echo "  响应：$RESP"
echo "$RESP" | grep -q 'record_stop_failed' ; assert "stop 如实报错（未伪装成功）" $?
CONTENT="$(cat "$WORK"/red/recordings/*.mp4 2>/dev/null)"
[ "$CONTENT" = "MP4HEADER" ] ; assert "产物确实缺 moov（实际：${CONTENT}）" $?

echo
echo "=========================================="
echo "通过 $PASS 项，失败 $FAIL 项"
[ "$FAIL" -eq 0 ] || exit 1
echo "✅ NAS 录制停止信号链路正常"
