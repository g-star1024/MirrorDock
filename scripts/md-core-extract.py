#!/usr/bin/env python3
"""X10-113：把共享设备/错误逻辑从 src-tauri 搬进 crates/md-core。

【为什么改成自动求依赖闭包】
第一版按人工划定的行号区间搬运，结果 dedup_devices 依赖的辅助判据散落在前后
一千行里，只能靠编译器一个个报 E0425 再逐个补——逐个试错不可收敛，漏一个就是
一次完整的重新抽取。
本版建立「顶层函数 → 被调用的顶层函数」调用图，从种子函数做传递闭包，一次性
把整条调用链搬干净；搬之前再静态扫一遍残留的 crate:: 引用，凡是还得指回桌面端
的符号直接终止，避免把依赖桌面的代码偷偷带进跨平台 core。
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LIB = ROOT / "src-tauri" / "src" / "lib.rs"
CORE = ROOT / "crates" / "md-core" / "src"

STRUCT_BLOCK = (51, 87)
ERROR_BLOCK = (121, 157)
BASE32_BLOCK = (7711, 7754)
BASE32_FILL = [
    "    // ---- Base32（RFC 4648，无填充）。实现已迁入共享核心库 md-core，",
    "    //      桌面端与 NAS 端共用同一份，杜绝两处实现漂移（X10-113）。 ----",
    "    pub use md_core::base32::{base32_decode, base32_encode};",
]

SEEDS = ["dedup_devices", "is_wireless_endpoint", "endpoint_preference"]

RESERVED = {
    "if", "for", "while", "match", "return", "let", "mut", "move", "as", "in", "else",
    "loop", "break", "continue", "struct", "enum", "fn", "impl", "mod", "pub", "use",
    "unsafe", "const", "static", "ref", "box", "dyn", "Self", "true", "false", "some",
    "ok", "err", "and_then", "map", "unwrap", "expect", "clone", "iter", "collect",
    "and", "or", "not", "is", "to", "from", "into", "push", "get", "set", "len",
}


def top_level_fns(lines):
    out, i, n = {}, 0, len(lines)
    while i < n:
        line = lines[i]
        m = re.match(r"^(?:pub(?:\(crate\))? )?fn (\w+)", line)
        if line and not line.startswith(" ") and m:
            j = i
            while j < n and lines[j].rstrip() != "}":
                j += 1
            out[m.group(1)] = (i + 1, j + 1)
            i = j + 1
            continue
        i += 1
    return out


def callees(lines, rng):
    s, e = rng
    body = "\n".join(lines[s - 1:e])
    names = {m.group(1) for m in re.finditer(r"(?<![\.\w:])([a-z_][a-z0-9_]*)\s*\(", body)}
    return names - RESERVED


def closure(fnmap, seeds, lines):
    seen, queue = set(), list(seeds)
    missing = [s for s in seeds if s not in fnmap]
    if missing:
        sys.exit("✗ 顶层找不到种子函数: %s" % missing)
    while queue:
        cur = queue.pop()
        if cur in seen:
            continue
        seen.add(cur)
        for cal in callees(lines, fnmap[cur]):
            if cal in fnmap and cal not in seen:
                queue.append(cal)
    return seen


def expand_docs(lines, rng):
    start = rng[0]
    j = start - 2
    while j >= 0 and lines[j].strip().startswith(("///", "//", "#[")):
        start = j + 1
        j -= 1
    return (start, rng[1])


def make_pub(text):
    field_re = re.compile(r"^    (\w+): .*,$")
    in_body, in_impl, out = False, False, []
    for line in text.split("\n"):
        # impl 块：其中的关联函数同样要对外可见（AppError::new 就是这么用的）
        if line.startswith("impl "):
            in_impl = True
            out.append(line)
            continue
        if in_impl and re.match(r"^    fn \w+", line):
            out.append("    pub " + line[4:])
            continue
        if in_impl and re.match(r"^\}", line):
            in_impl = False
            out.append(line)
            continue
        if not in_body and re.match(r"^(?:pub )?(?:struct|enum) \w+", line) \
                and line.rstrip().endswith("{"):
            out.append(line if line.startswith("pub ") else "pub " + line)
            in_body = True
            continue
        if in_body and (m := field_re.match(line)):
            out.append("    pub %s: %s" % (m.group(1), line.strip().split(":", 1)[1].strip()))
            continue
        if in_body and re.match(r"^\}", line):
            in_body = False
            out.append(line)
            continue
        if re.match(r"^(?:pub )?fn \w+", line) and not line.startswith("pub "):
            line = "pub " + line
        out.append(line)
    return "\n".join(out)


def main():
    lines = LIB.read_text(encoding="utf-8").split("\n")
    original_len = len(lines)
    fnmap = top_level_fns(lines)

    for rng, token in [
        (STRUCT_BLOCK, "enum DeviceState"),
        (ERROR_BLOCK, "struct AppError"),
        (BASE32_BLOCK, "B32_ALPHABET"),
    ]:
        seg = "\n".join(lines[rng[0] - 1:rng[1]])
        if token not in seg:
            sys.exit("✗ 区块 %s 不含 %s，行号已漂移，先重新定位" % (rng, token))

    # 固定区块同样要向上吃掉紧邻的 #[derive] 与文档注释：第一版把 AppError 的
    # derive 留在了 lib.rs 里，孤儿属性挂到下一个 item 上，造成 SessionPhase
    # Debug 重复派生的诡异报错（编译器实锤后修正）。
    STRUCT_RNG = expand_docs(lines, STRUCT_BLOCK)
    ERROR_RNG = expand_docs(lines, ERROR_BLOCK)
    BASE32_RNG = expand_docs(lines, BASE32_BLOCK)

    need = closure(fnmap, SEEDS, lines)
    print("依赖闭包：%d 个函数" % len(need))
    for name in sorted(need):
        print("    · %s  (%d-%d)" % (name, fnmap[name][0], fnmap[name][1]))

    fn_blocks = sorted((expand_docs(lines, fnmap[n]), n) for n in need)

    struct_text = "\n".join(lines[STRUCT_RNG[0] - 1:STRUCT_RNG[1]])
    error_text = "\n".join(lines[ERROR_RNG[0] - 1:ERROR_RNG[1]])
    base32_text = "\n".join(lines[BASE32_RNG[0] - 1:BASE32_RNG[1]])
    print("  区块扩展为 struct=%s error=%s base32=%s" % (STRUCT_RNG, ERROR_RNG, BASE32_RNG))
    fn_texts = ["\n".join(lines[s - 1:e]) for (s, e), _ in fn_blocks]

    all_text = "\n".join([struct_text, error_text, base32_text] + fn_texts)
    leaked = sorted(set(re.findall(r"crate::([A-Za-z_][A-Za-z0-9_]*)", all_text)))
    if leaked:
        sys.exit("✗ 待搬代码仍引用桌面端 crate::%s——不适合放进跨平台 crate" % leaked)
    print("静态自检：无 crate:: 残留引用 ✓")

    points = [(s, e, None) for (s, e), _ in fn_blocks]
    points.append((BASE32_RNG[0], BASE32_RNG[1], BASE32_FILL))
    points.append((ERROR_RNG[0], ERROR_RNG[1], None))
    points.append((STRUCT_RNG[0], STRUCT_RNG[1], None))
    points.sort(reverse=True)

    seen = set()
    for s, e, fill in points:
        if (s, e) in seen:
            continue
        seen.add((s, e))
        lines[s - 1:e] = fill if fill is not None else []

    (CORE / "devices.rs").write_text(
        "//! 设备发现与通道语义（自桌面端 lib.rs 逐字搬入，X10-113）。\n"
        "//!\n//! 这里决定「同一台手机同时插着 USB 且开着无线时走哪条通道」——\n"
        "//! NAS 端必须给出与桌面端完全一致的答案。用户在两端看到不同的优先级\n"
        "//! 是最难排查的一类问题，故把判据收成唯一一份实现。\n\n"
        "use serde::Serialize;\n\n"
        + make_pub(struct_text) + "\n\n"
        + "\n\n".join(make_pub(t) for t in fn_texts) + "\n",
        encoding="utf-8")
    print("  写入 devices.rs")

    (CORE / "error.rs").write_text(
        "//! 结构化错误（自桌面端 lib.rs 逐字搬入，X10-113）。\n//!\n"
        "//! NAS 端的 HTTP 响应直接序列化本类型，保证错误码与桌面端同源，\n"
        "//! 前端不必为两种形态各写一套错误处理分支。\n\n"
        "use serde::Serialize;\n\n" + make_pub(error_text) + "\n",
        encoding="utf-8")
    print("  写入 error.rs")

    (CORE / "base32.rs").write_text(
        "//! Base32（RFC 4648，无填充）——自桌面端 licensing 模块抽离，X10-113。\n"
        "//!\n//! 先前深埋在 mod licensing 内，NAS 端要用就得整体依赖授权模块，\n"
        "//! 故抽为独立模块供两端共用。\n\n" + base32_text + "\n",
        encoding="utf-8")
    print("  写入 base32.rs")

    # 整个闭包都要 re-export：测试里也直接调用这些辅助判据（device_has_usb 就是
    # 这么被 lib.rs 的用例引用的），只导出入口函数会让测试编译失败。
    fns = ", ".join(sorted(need))
    types = "AdbDevice, ConnectionEndpoint, ConnectionKind, DeviceState"
    anchor = next(i for i, l in enumerate(lines) if "设备与结构化错误" in l)
    lines[anchor:anchor] = [
        "// X10-113：共享核心库。设备发现与结构化错误已迁入 crates/md-core，桌面端与",
        "// NAS 端共用同一份判据。此处 re-export 让 lib.rs 内所有既有引用路径保持",
        "// 不变——这是不破坏桌面端的关键：调用方（含测试）一行都不用改。",
        "pub use md_core::devices::{%s, %s};" % (fns, types),
        "pub use md_core::error::{adb_command_error, adb_missing_error, AppError};",
        "pub use md_core::{base32_decode, base32_encode};",
        "",
    ]

    LIB.write_text("\n".join(lines), encoding="utf-8")
    print("\nlib.rs: %d → %d 行" % (original_len, len(lines)))


if __name__ == "__main__":
    main()
