#!/usr/bin/env bash
# Run a consented, authorized MirrorDock POC session and keep only local metrics/logs.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <authorized-adb-serial>" >&2
  exit 64
fi

serial="$1"
case "$serial" in
  *[!A-Za-z0-9._:-]*|'')
    echo "The device serial contains unsupported characters." >&2
    exit 64
    ;;
esac

workspace_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scrcpy_bin="$workspace_root/.tools/scrcpy/macos-x86_64/scrcpy"
result_dir="$workspace_root/test-runs/$(date -u +%Y%m%dT%H%M%SZ)-$serial"

if ! command -v adb >/dev/null 2>&1; then
  echo "adb is not installed." >&2
  exit 69
fi
if [ ! -x "$scrcpy_bin" ]; then
  echo "The development scrcpy runtime is unavailable at $scrcpy_bin." >&2
  exit 69
fi
if [ "$(adb -s "$serial" get-state 2>/dev/null || true)" != "device" ]; then
  echo "The requested device is not authorized and ready." >&2
  exit 69
fi

mkdir -p "$result_dir"
{
  echo "started_at_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "serial=$serial"
  adb version | head -n 2
  "$scrcpy_bin" --version | head -n 1
} > "$result_dir/session-metadata.txt"

echo "Session logs: $result_dir/scrcpy.log"
echo "Record the visible first-frame time and interaction checks in docs/POC_ACCEPTANCE.md."
"$scrcpy_bin" --serial "$serial" --print-fps 2>&1 | tee "$result_dir/scrcpy.log"
