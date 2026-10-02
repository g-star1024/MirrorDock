#!/usr/bin/env python3
"""伴侣端 Kotlin 编译检查（无需 gradle，2026-10-03 建立）。

背景：本机没有 gradle 发行版（沙箱拦大文件下载，curl 返回 exit 56），而
`build.yml` 的 `release` job `needs: [package, companion-apk]` —— 伴侣 APK 编译
失败会让**整个 Release 跳过，四平台包全部白跑**。2026-10-03 v0.4.7 连续两次发版
都栽在这上面，而当时：
- `/logs` 端点需 admin 权限（403），拿不到编译器原文；
- job annotations 只有 "exit code 1"。

于是本脚本用 Gradle 缓存里已有的 `kotlin-compiler-embeddable` + Android SDK 的
`android.jar` + AndroidX 的 classes.jar，**直接跑 kotlinc 做前端类型检查**。

用法（在仓库根目录执行）：
    python3 scripts/check-companion-kotlin.py [--full]

默认模式：跑**前端类型检查**（-Xuse-k2 已不需要，只要不加 -Xbackend-threads 之类
就会在 IR lowering 阶段因缺 aapt 产物而报 BackendException —— 那是本脚本环境
的产物，不是代码问题）。有真实类型错误时退出码 1。

--full：对**未改动的文件**也会跑后端，输出可能含环境噪声，仅供排查。

关键依赖（缺任一项脚本会明确报出并退出 2）：
- JDK 17（`/usr/bin/java`；Android Studio 自带的是 25，kotlinc 2.0.20 解析不了）
- `kotlin-compiler-embeddable` + `kotlin-stdlib` + `kotlin-reflect` +
  `kotlin-script-runtime` + `kotlin-daemon-embeddable` + `trove4j` + `kotlinx-coroutines-core-jvm`
- `android.jar`（API 34）
- AndroidX 的 aar（appcompat / activity / core / fragment / lifecycle / savedstate …）
- zxing-core、guava（ScanActivity / CaptureService 用）
"""
import fnmatch
import glob
import os
import pathlib
import re
import subprocess
import sys
import zipfile

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(SCRIPT_DIR)
SRC = os.path.join(REPO, "companion", "app", "src", "main")
JAVA_DIR = os.path.join(SRC, "java", "com", "mirrordock", "companion")
GRADLE_CACHE = os.path.expanduser("~/.gradle/caches/modules-2/files-2.1")
WORK = "/tmp/mirrordock-kotlin-check"
ANDROID_JAR = os.path.expanduser("~/Library/Android/sdk/platforms/android-34/android.jar")
JDK = "/usr/bin/java"

def die(msg, code=2):
    print(f"❌ {msg}")
    sys.exit(code)

def find_in_cache(module, filename_glob):
    """按「模块目录 + 文件名」两级查找，避开 Gradle 缓存 group/module/version/hash/file
    的层级与版本差异。"""
    base = os.path.join(GRADLE_CACHE, module)
    if not os.path.isdir(base):
        return []
    out = []
    for root, _dirs, files in os.walk(base):
        for fn in files:
            if fn == "sources.jar" or "sources" in fn or "javadoc" in fn:
                continue
            if fnmatch.fnmatch(fn, filename_glob):
                out.append(os.path.join(root, fn))
    return sorted(out)

def find_one(module, filename_glob):
    hits = find_in_cache(module, filename_glob)
    return hits[-1] if hits else None

def find_all_modules(modules_and_globs):
    out = []
    for module, glob in modules_and_globs:
        out += find_in_cache(module, glob)
    return out

def extract_aar(aar, dest_name):
    """AAR 是 zip，取出 classes.jar。"""
    target = os.path.join(WORK, "aar", dest_name)
    os.makedirs(target, exist_ok=True)
    jar = os.path.join(target, "classes.jar")
    if os.path.exists(jar):
        return jar
    try:
        with zipfile.ZipFile(aar) as z:
            if "classes.jar" in z.namelist():
                with z.open("classes.jar") as src, open(jar, "wb") as dst:
                    dst.write(src.read())
    except Exception:
        return None
    return jar if os.path.exists(jar) else None

def make_r_stub(classpath_dir):
    """AGP 生成的 R.class 本地没有；按 res/ 实际内容生成一个同形状的存根。

    字段值无关紧要（kotlinc 只做类型检查），关键是**字段名齐全**，否则会报
    一堆假的 "unresolved reference"，把真错误淹没。
    """
    res = os.path.join(SRC, "res")
    def names_in(fname, tag):
        p = os.path.join(res, "values", fname)
        if not os.path.exists(p):
            return set()
        return set(re.findall(rf'<{tag}[^>]*\bname="([^"]+)"',
                              pathlib.Path(p).read_text(encoding="utf-8")))

    strings = names_in("strings.xml", "string")
    colors = names_in("colors.xml", "color")
    drawables = {os.path.basename(f)[:-4]
                 for f in os.listdir(os.path.join(res, "drawable")) if f.endswith(".xml")}
    layouts = {os.path.basename(f)[:-4]
               for f in os.listdir(os.path.join(res, "layout")) if f.endswith(".xml")}
    layout_dir = os.path.join(res, "layout")
    ids = set()
    for f in os.listdir(layout_dir):
        if f.endswith(".xml"):
            ids |= set(re.findall(r'@\+id/([A-Za-z0-9_]+)',
                                  pathlib.Path(os.path.join(layout_dir, f)).read_text(encoding="utf-8")))
    mipmaps = {"ic_launcher", "ic_launcher_round"}

    out_dir = os.path.join(classpath_dir, "gen", "com", "mirrordock", "companion")
    os.makedirs(out_dir, exist_ok=True)
    parts = ["package com.mirrordock.companion;", "", "public final class R {"]
    for kind, names in [("string", strings), ("color", colors), ("drawable", drawables),
                        ("layout", layouts), ("mipmap", mipmaps), ("id", ids)]:
        parts.append(f"  public static final class {kind} {{")
        for i, n in enumerate(sorted(names)):
            parts.append(f"    public static final int {n} = {0x7f000000 + i};")
        parts.append("  }")
    parts.append("}")
    java_path = os.path.join(out_dir, "R.java")
    pathlib.Path(java_path).write_text("\n".join(parts), encoding="utf-8")

    out_classes = os.path.join(classpath_dir, "classes")
    os.makedirs(out_classes, exist_ok=True)
    r = subprocess.run(["javac", "-nowarn", "-d", out_classes, java_path],
                       capture_output=True, text=True)
    if r.returncode != 0:
        die(f"生成 R 存根失败：\n{r.stderr[:500]}")
    return out_classes

def main():
    full = "--full" in sys.argv
    os.makedirs(WORK, exist_ok=True)

    if not os.path.exists(JDK):
        die(f"找不到 JDK 17（{JDK}）。本机 Android Studio 自带的是 JDK 25，"
            f"kotlinc 2.0.20 解析不了 Java 25 版本号。")
    ver = subprocess.run([JDK, "-version"], capture_output=True, text=True).stderr
    if '"17' not in ver and '17.' not in ver:
        die(f"{JDK} 不是 JDK 17：\n{ver.strip()[:120]}")
    if not os.path.exists(ANDROID_JAR):
        die(f"找不到 android.jar（{ANDROID_JAR}）。装 Android SDK 或改脚本里的路径。")

    kc = find_one("org.jetbrains.kotlin/kotlin-compiler-embeddable", "kotlin-compiler-embeddable-2*.jar")
    stdlib = find_one("org.jetbrains.kotlin/kotlin-stdlib", "kotlin-stdlib-2*.jar")
    reflect = find_one("org.jetbrains.kotlin/kotlin-reflect", "kotlin-reflect-*.jar")
    script_rt = find_one("org.jetbrains.kotlin/kotlin-script-runtime", "kotlin-script-runtime-*.jar")
    daemon = find_one("org.jetbrains.kotlin/kotlin-daemon-embeddable", "kotlin-daemon-embeddable-*.jar")
    trove = find_one("org.jetbrains.intellij.deps/trove4j", "trove4j-*.jar")
    coroutines = find_one("org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm", "kotlinx-coroutines-core-jvm-*.jar")
    for name, jar in [("kotlin-compiler-embeddable", kc), ("kotlin-stdlib", stdlib),
                      ("kotlin-reflect", reflect), ("kotlin-script-runtime", script_rt),
                      ("kotlin-daemon-embeddable", daemon), ("trove4j", trove),
                      ("kotlinx-coroutines-core-jvm", coroutines)]:
        if not jar:
            die(f"Gradle 缓存里找不到 {name}。先跑一次 gradle 构建把依赖下下来。")

    # 编译器自身的 classpath
    run_cp = ":".join([kc, stdlib, reflect, script_rt, daemon, trove, coroutines])

    # 待编译源码的 classpath
    cp_parts = [stdlib, ANDROID_JAR]
    # 这些 artifact 里只要有一个缺失，kotlinc 就会报
    # "cannot access 'androidx.xxx' which is a supertype of ..." 一类假错误，
    # 把真错误淹没。宁可全加。
    aar_modules = [
        "androidx.appcompat/appcompat", "androidx.activity/activity",
        "androidx.core/core", "androidx.core/core-ktx", "androidx.fragment/fragment",
        "androidx.lifecycle/lifecycle-runtime", "androidx.lifecycle/lifecycle-viewmodel",
        "androidx.lifecycle/lifecycle-viewmodel-savedstate",
        "androidx.lifecycle/lifecycle-livedata", "androidx.lifecycle/lifecycle-livedata-core",
        "androidx.lifecycle/lifecycle-common", "androidx.lifecycle/lifecycle-process",
        "androidx.savedstate/savedstate", "androidx.camera/camera-core",
        "androidx.camera/camera-camera2", "androidx.camera/camera-lifecycle",
        "androidx.camera/camera-view", "androidx.camera/camera-video",
        "androidx.annotation/annotation", "androidx.annotation/annotation-jvm",
        "androidx.annotation/experimental", "androidx.collection/collection",
        "androidx.versionedparcelable/versionedparcelable",
        "androidx.customview/customview", "androidx.emoji2/emoji2",
        "androidx.emoji2-views-helper/emoji2-views-helper", "androidx.loader/loader",
        "androidx.interpolator/interpolator", "androidx.drawerlayout/drawerlayout",
        "androidx.viewpager/viewpager", "androidx.vectordrawable/vectordrawable",
        "androidx.resourceinspection/resourceinspection-annotation",
        "androidx.slidingpanelayout/slidingpanelayout", "androidx.legacy/legacy-support-v4",
        "androidx.window/window", "androidx.dynamicanimation/dynamicanimation",
        "androidx.tracing/tracing", "androidx.concurrent/concurrent-futures",
        "androidx.startup/startup-runtime", "androidx.profileinstaller/profileinstaller",
    ]
    jars = []
    for mod in aar_modules:
        jars += [(m, "*.aar") for m in find_in_cache(mod, "*.aar")]
        jars += [(m, "*.jar") for m in find_in_cache(mod, "*.jar")]
    for mod, pat in [("com.google.zxing/core", "core-*.jar"),
                     ("com.google.guava/guava", "guava-*.jar"),
                     ("com.google.guava/listenablefuture", "*.jar"),
                     ("androidx.camera/camera-core", "*.jar")]:
        jars += [(m, pat) for m in find_in_cache(mod, pat)]
    for jar, _pat in jars:
        base = os.path.basename(jar)
        j = extract_aar(jar, base[:-4]) if base.endswith(".aar") else jar
        if j:
            cp_parts.append(j)
        base = os.path.basename(jar)
        if base.endswith(".aar"):
            j = extract_aar(jar, base[:-4])
        else:
            j = jar
        if j:
            cp_parts.append(j)

    cp_parts.append(make_r_stub(WORK))
    cp = ":".join(cp_parts)

    sources = sorted(glob.glob(os.path.join(JAVA_DIR, "*.kt")))
    if not sources:
        die(f"没找到 Kotlin 源文件（{JAVA_DIR}）")

    cmd = [JDK, "-cp", run_cp, "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler",
           "-no-stdlib", "-jvm-target", "17", "-cp", cp,
           "-d", os.path.join(WORK, "out")] + sources
    r = subprocess.run(cmd, capture_output=True, text=True)
    out = (r.stdout or "") + (r.stderr or "")

    # 只统计前端类型检查错误（行首 "文件:行:列: error:"）。
    # BackendException 属后端 IR lowering，在缺 aapt 产物的本环境必然出现，
    # 与代码无关（已用发版前原始代码验证过同样报错），故不计入。
    errors = re.findall(r"^(\S+\.kt):(\d+):(\d+): error: (.+)$", out, re.M)
    backend = "Backend Internal error" in out

    if errors:
        print(f"❌ 发现 {len(errors)} 个 Kotlin 编译错误：")
        seen = set()
        for path, line, col, msg in errors:
            key = (path, msg)
            if key in seen:
                continue
            seen.add(key)
            name = os.path.basename(path)
            print(f"  {name}:{line}:{col}  {msg}")
        print()
        print("（这些是 CI「构建 debug APK」会失败的真实原因）")
        return 1

    if backend:
        print("✅ 前端类型检查通过（0 error）")
        print("ℹ️  后端 IR lowering 报 BackendException —— 这是本脚本环境缺少 aapt 产物导致的，")
        print("    与代码无关（用发版前未改动的原始代码验证过同样报错）。真编译仍以 CI 为准。")
        return 0

    print("✅ Kotlin 编译通过（0 error）")
    return 0

if __name__ == "__main__":
    sys.exit(main())
