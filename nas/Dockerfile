# MirrorDock NAS 版容器镜像
#
# 立项场景：NAS 常开机 + USB 直连 + 24h 录制归档 + 远程 web 访问。
#
# 设计约束（与桌面端一致的 fail-closed 原则）：
#   • scrcpy / SDL3 / adb 三件的来源与 CI（build.yml 的 Linux job）**逐字一致**，
#     哈希写死、校验失败即终止构建——不在容器里引入第二套来源，避免桌面端与
#     NAS 端跑不同版本的引擎。
#   • 桌面端依赖 SDL 开窗，容器无 DISPLAY 故不可用；NAS 端只用 scrcpy 的
#     headless 能力（录制/截图走 adb），这是本版不提供实时画面的根本原因。
#
# 阶段划分：scrcpy-builder（最慢，先单独构建可复用缓存）→ app-builder → runtime

# ---------------------------------------------------------------------------
# 阶段 1：编译 scrcpy 运行时（SDL3 源码编译 + scrcpy v4.1 + adb）
# 独立成层，便于 `docker build --target scrcpy-builder` 预热缓存，
# 后续改 Rust 代码不必重来一遍十几分钟的 C 侧编译。
# ---------------------------------------------------------------------------
FROM debian:12-slim AS scrcpy-builder

# 版本常量与 build.yml 的 Linux job 完全对齐，改版本必须两处一起改。
ARG SCRCPY_VERSION=4.1
ARG SDL3_VERSION=release-3.4.16
ARG PLATFORM_TOOLS_VERSION=r34.0.5

RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl unzip \
      gcc make pkg-config meson ninja-build cmake pkg-config \
      # libc6-dev 必须显式写：Debian 里 gcc-12 对 libc6-dev 只是 Recommends 而非
      # Depends，而本层用了 --no-install-recommends → 不写就没有 Scrt1.o / crti.o。
      # 后果是「能编译不能链接」，报错点在 SDL3 的 cmake 里，离根因十万八千里
      # （2026-10-11 实踩：CI 3 分钟失败，靠完整日志才定位到 ld 找不到启动文件）。
      libc6-dev \
      libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libavdevice-dev \
      libusb-1.0-0-dev \
    && rm -rf /var/lib/apt/lists/* \
    # 工具链自检：必须是「真的编译+链接+跑一遍」，不能只查 --version。
    # 只查 --version 会让「gcc 在但 libc6-dev 不在」这类问题一路潜伏到 SDL3 的
    # cmake 才炸——那一刻的报错（CMAKE_C_COMPILER not set / C compiler broken）
    # 完全指不向真正的缺失项。
    && printf 'int main(void){return 0;}\n' > /tmp/md-toolchain-check.c \
    && cc /tmp/md-toolchain-check.c -o /tmp/md-toolchain-check \
    && /tmp/md-toolchain-check \
    && rm -f /tmp/md-toolchain-check /tmp/md-toolchain-check.c \
    && gcc --version >/dev/null \
    && cmake --version >/dev/null \
    && meson --version >/dev/null \
    && ninja --version >/dev/null \
    && echo "工具链自检通过（编译+链接+运行均成功）"

WORKDIR /build

# ⓪ SDL3：scrcpy 4.x 依赖 SDL3（Debian 12 仓库只有 SDL2，社区经验已实锤）。
#    哈希与 CI 一致，校验失败立即终止。
RUN curl -fsSL -o sdl3.tar.gz "https://codeload.github.com/libsdl-org/SDL/tar.gz/refs/tags/${SDL3_VERSION}" \
    && echo "c2ee715e42ec520c4d11fd8d249ef1d2b2baf4ad31148b72b3b000276c0b3633  sdl3.tar.gz" | sha256sum -c - \
    && tar -xzf sdl3.tar.gz \
    # SDL_UNIX_CONSOLE_BUILD=ON：容器里没有 X11/Wayland，SDL3 默认会因此直接
    # 终止配置（"SDL could not find X11 or Wayland development libraries"）。
    # 官方 docs/README-cmake.md 对「真的不需要窗口」给的就是这个开关——NAS 端
    # 只用 scrcpy 的 --no-window --no-playback 录制能力，确实不需要窗口。
    # 打开后保留的后端是 dummy / offscreen，正好是 headless 需要的那两个。
    && cmake -S "SDL-${SDL3_VERSION}" -B sdl3-build \
        -DCMAKE_BUILD_TYPE=Release -DSDL_SHARED=ON -DSDL_STATIC=OFF -DSDL_TEST_LIBRARY=OFF \
        -DSDL_UNIX_CONSOLE_BUILD=ON \
    && cmake --build sdl3-build -j4 \
    && cmake --install sdl3-build \
    && ldconfig

# ① scrcpy 源码（固定 tag + 固定源码哈希）
RUN curl -fsSL -o scrcpy-src.tar.gz "https://codeload.github.com/Genymobile/scrcpy/tar.gz/refs/tags/v${SCRCPY_VERSION}" \
    && echo "537b2ade623cb94b6edddfa5c61bf0b0af21484aa8365ea2531b686ea573249a  scrcpy-src.tar.gz" | sha256sum -c - \
    && tar -xzf scrcpy-src.tar.gz

# ② scrcpy-server：容器里没有 gh CLI，改用 GitHub API 取官方 release 的 digest，
#    与 CI 的 `--jq .digest` 语义等价（同样 fail-closed，取不到即失败）。
RUN SCRCPY_SERVER="scrcpy-server-v${SCRCPY_VERSION}" \
    && DEB_ARCH="$(dpkg --print-architecture)" \
    && case "$DEB_ARCH" in \
         amd64)   GH_ARCH=x86_64 ;; \
         arm64)   GH_ARCH=aarch64 ;; \
         *)       echo "unsupported arch: $DEB_ARCH"; exit 1 ;; \
       esac \
    && curl -fsSL -o "$SCRCPY_SERVER" \
        "https://github.com/Genymobile/scrcpy/releases/download/v${SCRCPY_VERSION}/${SCRCPY_SERVER}" \
    && echo "架构 ${DEB_ARCH} -> GitHub release arch ${GH_ARCH}" \
    && EXPECTED="$(curl -fsSL "https://api.github.com/repos/Genymobile/scrcpy/releases/tags/v${SCRCPY_VERSION}" \
        | sed -n 's/.*"digest" *: *"sha256:\([0-9a-f]*\)".*/\1/p' | head -1)" \
    && ACTUAL="$(sha256sum "$SCRCPY_SERVER" | cut -d' ' -f1)" \
    && echo "server digest: expected=${EXPECTED} actual=${ACTUAL}" \
    && [ -n "$EXPECTED" ] \
    && [ "$EXPECTED" = "$ACTUAL" ] \
    && echo "scrcpy-server 校验通过"

# ③ adb：Google 官方 platform-tools（固定版本 + 固定哈希）
RUN curl -fsSL -o platform-tools.zip \
      "https://dl.google.com/android/repository/platform-tools_${PLATFORM_TOOLS_VERSION}-linux.zip" \
    && echo "362f8f6218af0f4c61e5aaafb8e255a426c7a0ee00127dfab7371775081d3124  platform-tools.zip" | sha256sum -c - \
    && unzip -q platform-tools.zip

# ④ 编译 scrcpy（prebuilt_server 指向已校验过的 server，不重新编译 Java 侧）
RUN cd "scrcpy-${SCRCPY_VERSION}" \
    && meson setup build --buildtype=release \
        -Dprebuilt_server=../scrcpy-server-v${SCRCPY_VERSION} -Dcompile_server=false \
    && ninja -C build

# ⑤ 组装到统一目录，供 runtime 阶段整段 COPY
RUN mkdir -p /opt/scrcpy \
    && cp "scrcpy-${SCRCPY_VERSION}/build/app/scrcpy" /opt/scrcpy/scrcpy \
    && cp "scrcpy-server-v${SCRCPY_VERSION}" /opt/scrcpy/scrcpy-server \
    && cp platform-tools/adb /opt/scrcpy/adb \
    && chmod +x /opt/scrcpy/scrcpy /opt/scrcpy/adb \
    # scrcpy 启动时会在自身目录找图标，缺了会刷错误（X10-79 的经验）
    && touch /opt/scrcpy/scrcpy.png \
    && ls -la /opt/scrcpy/

# ---------------------------------------------------------------------------
# 阶段 2：编译 NAS 服务端（Rust）
# 放在 scrcpy 编译之后：C 侧产物可长期复用缓存，日常改 Rust 代码只重跑这一层。
# ---------------------------------------------------------------------------
FROM rust:1-bookworm AS app-builder

WORKDIR /src
# 先只拷清单，让依赖编译独立成层——改业务代码时不必重编全部依赖。
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY nas/server ./nas/server
# 用真实的空壳源码骗过 cargo 的编译顺序，先单独构建依赖层。
RUN mkdir -p nas/server/src crates/md-core/src \
    && echo 'fn main() {}' > nas/server/src/main.rs \
    && echo '' > crates/md-core/src/lib.rs \
    && cargo build --release -p mirrordock-nas || true
# 再拷真实源码构建（依赖层已缓存）。
COPY crates ./crates
COPY nas/server ./nas/server
RUN touch nas/server/src/main.rs crates/md-core/src/lib.rs \
    && cargo build --release -p mirrordock-nas \
    && cp target/release/mirrordock-nas /usr/local/bin/mirrordock-nas

# ---------------------------------------------------------------------------
# 阶段 3：运行时
# 只装运行所需动态库，不残留编译工具链，控制镜像体积。
# ---------------------------------------------------------------------------
FROM debian:12-slim AS runtime

# scrcpy 运行依赖：ffmpeg 解码/封装库、libusb（adb 需要）、SDL3（scrcpy client 链接）
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates \
      libavcodec59 libavformat59 libavutil57 libswscale6 libavdevice59 \
      libusb-1.0-0 \
    && rm -rf /var/lib/apt/lists/*

# SDL3 运行时（源码编译产物，Debian 仓库没有）
COPY --from=scrcpy-builder /usr/local/lib/libSDL3.so* /usr/local/lib/
COPY --from=scrcpy-builder /opt/scrcpy /opt/scrcpy
RUN ldconfig

# NAS 服务端二进制与前端静态站
COPY --from=app-builder /usr/local/bin/mirrordock-nas /opt/mirrordock/mirrordock-nas
COPY nas/web /opt/mirrordock/web

ENV PATH="/opt/scrcpy:${PATH}"
ENV MIRRORDOCK_DATA=/var/lib/mirrordock
ENV MIRRORDOCK_WEB=/opt/mirrordock/web

# adb server 的数据目录（adbkeys 持久化在此，避免容器重建后手机重新弹授权）
VOLUME ["/var/lib/mirrordock"]

EXPOSE 8080

WORKDIR /opt/mirrordock

CMD ["/opt/mirrordock/mirrordock-nas"]
