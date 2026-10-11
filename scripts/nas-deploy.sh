#!/usr/bin/env bash
# MirrorDock NAS 版 · 一键部署/更新脚本（X10-120）
#
# 在飞牛 fnOS（或任何跑了 docker 的 NAS）上执行：
#   ./scripts/nas-deploy.sh
# 它会：
#   1) docker compose pull 拉取最新镜像（强制比对远端 digest，拿到真新版）
#   2) 读取刚拉下的镜像 RepoDigest，注入 MIRRORDOCK_IMAGE_DIGEST
#      ——「检查更新」功能据此与远端 latest 比对，判断有没有更新的版本
#   3) 读取当前 git 短哈希注入 MIRRORDOCK_REVISION（人话版本号；非 git 部署可空）
#   4) docker compose up -d 重建容器
#
# 为什么需要这个脚本：
#   飞牛的「有新版本」徽标只适配 Docker Hub，对 ghcr.io 不比对，永远显示
#   「已是最新」（真机实踩）。手动 docker compose up 又因为 docker 对同名
#   latest 用本地缓存，不会主动重查远端——必须先显式 pull 才能拿到新版。
#   而「检查更新」又需要容器知道自己跑的是哪个 digest，这个值只能在
#   pull 之后、up 之前从宿主机读出来注入。三步合一，一次执行到位。
#
# 可选：WITH_WATCHTOWER=1 ./scripts/nas-deploy.sh 会额外启动 Watchtower
# 自动更新容器（叠加 docker-compose.watchtower.yml），实现无人值守在线更新。

set -euo pipefail

IMAGE="ghcr.io/g-star1024/mirrordock/mirrordock-nas:latest"

# 切到 compose 文件所在目录（脚本在 scripts/ 下，compose 在 nas/ 下）。
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_DIR="${SCRIPT_DIR}/../nas"
cd "${COMPOSE_DIR}"

# 是否叠加 Watchtower 自动更新（WITH_WATCHTOWER=1 时启用）。
COMPOSE_FILES=(-f docker-compose.yml)
if [ "${WITH_WATCHTOWER:-0}" = "1" ]; then
  COMPOSE_FILES+=(-f docker-compose.watchtower.yml)
  echo "==> 已启用 Watchtower 自动更新（每 6 小时检查一次）"
fi

echo "==> 拉取最新镜像 ${IMAGE}"
docker compose "${COMPOSE_FILES[@]}" pull

echo "==> 读取本地镜像 digest"
DIGEST="$(docker inspect --format='{{index .RepoDigests 0}}' "${IMAGE}" 2>/dev/null || true)"
if [ -n "${DIGEST}" ]; then
  # RepoDigests 形如 ghcr.io/...@sha256:xxx，服务端只比对 @ 后的 digest 部分。
  export MIRRORDOCK_IMAGE_DIGEST="${DIGEST#*@}"
  echo "    ${MIRRORDOCK_IMAGE_DIGEST}"
else
  echo "    （未取到 RepoDigest——检查更新将退化为按 commit 比对）"
  export MIRRORDOCK_IMAGE_DIGEST=""
fi

# git 短哈希（人话版本号）。非 git 部署（比如只拷了 compose）时留空。
if git rev-parse --short HEAD >/dev/null 2>&1; then
  export MIRRORDOCK_REVISION="$(git rev-parse --short HEAD)"
  echo "==> 本地版本 ${MIRRORDOCK_REVISION}"
else
  export MIRRORDOCK_REVISION=""
fi

echo "==> 重建容器"
docker compose "${COMPOSE_FILES[@]}" up -d

echo ""
echo "完成。打开 http://<NAS IP>:8080 ，状态栏会显示「检查更新」结果。"
