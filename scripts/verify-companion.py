#!/usr/bin/env python3
"""伴侣端静态交叉校验（X10-73 起随重构引入）。

为什么需要它：本地没有 gradle，且沙箱会拦大文件下载（curl 拉 gradle 发行版
返回 exit 56），所以 Kotlin 编译只能靠 CI。但 XML 合法性、资源引用完整性、
findViewById 的 id 是否声明、Manifest 类是否存在这类错误**本地就能查**，
不该等到 CI 才发现。

用法（在仓库根目录执行）：
    python3 scripts/verify-companion.py
退出码：0 = 通过；1 = 有错误。
"""
import os, re, sys, xml.dom.minidom

# 无论从哪个目录调用，都以本文件位置定位仓库根。
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(SCRIPT_DIR)


ROOT = os.path.join(REPO, "companion", "app", "src", "main")
RES = os.path.join(ROOT, "res")
JAVA = os.path.join(ROOT, "java")
errors, warnings = [], []

# 1) 所有 XML 必须合法
for dirpath, _, files in os.walk(RES):
    for f in files:
        if f.endswith(".xml"):
            p = os.path.join(dirpath, f)
            try:
                xml.dom.minidom.parse(p)
            except Exception as e:
                errors.append(f"XML 非法 {p}: {e}")
for extra in ["AndroidManifest.xml"]:
    p = os.path.join(ROOT, extra)
    try:
        xml.dom.minidom.parse(p)
    except Exception as e:
        errors.append(f"XML 非法 {p}: {e}")

# 2) 资源定义集合
def names_in(path, tag):
    if not os.path.exists(path): return set()
    return set(re.findall(rf'<{tag}[^>]*\bname="([^"]+)"', open(path, encoding="utf-8").read()))

defined = {
    "string": names_in(os.path.join(RES, "values/strings.xml"), "string"),
    "color":  names_in(os.path.join(RES, "values/colors.xml"), "color"),
    "style":  names_in(os.path.join(RES, "values/styles.xml"), "style"),
    "dimen":  names_in(os.path.join(RES, "values/dimens.xml"), "dimen"),
}
drawables = {f[:-4] for f in os.listdir(os.path.join(RES, "drawable")) if f.endswith(".xml")}
mipmaps = set()
for d in os.listdir(RES):
    if d.startswith("mipmap"):
        mipmaps |= {f.split(".")[0] for f in os.listdir(os.path.join(RES, d))}

# 3) layout 里的 @string/@color/@style/@drawable 引用
layouts = {}
for f in os.listdir(os.path.join(RES, "layout")):
    if f.endswith(".xml"):
        p = os.path.join(RES, "layout", f)
        layouts[f] = open(p, encoding="utf-8").read()
        for kind, pool in [("string", defined["string"]), ("color", defined["color"]),
                           ("style", defined["style"]), ("drawable", drawables)]:
            for ref in set(re.findall(rf'@{kind}/([A-Za-z0-9_]+)', layouts[f])):
                if ref not in pool:
                    errors.append(f"layout/{f}: @{kind}/{ref} 未定义")

# 4) 代码里的 R.string/R.color/R.drawable/R.mipmap 引用
code = ""
for dp, _, fs in os.walk(JAVA):
    for f in fs:
        if f.endswith(".kt"):
            code += open(os.path.join(dp, f), encoding="utf-8").read() + "\n"
for kind, pool in [("string", defined["string"]), ("color", defined["color"]),
                   ("drawable", drawables), ("mipmap", mipmaps)]:
    # 用负向前瞻排除框架引用（android.R.string.ok / androidx...）——
    # 那些是系统资源，不在 app 的 R 里，检查会误报。
    for ref in set(re.findall(rf'(?<!android\.)(?<!androidx\.)R\.{kind}\.([A-Za-z0-9_]+)', code)):
        if ref not in pool:
            errors.append(f"Kotlin: R.{kind}.{ref} 未定义")

# 5) findViewById 的 id 必须在某个 layout 里存在
declared_ids = set()
for content in layouts.values():
    declared_ids |= set(re.findall(r'android:id="@\+id/([A-Za-z0-9_]+)"', content))
for rid in set(re.findall(r'findViewById(?:<[^>]*>)?\(R\.id\.([A-Za-z0-9_]+)\)', code)):
    if rid not in declared_ids:
        errors.append(f"Kotlin: R.id.{rid} 在任何 layout 中都未声明")

# 6) Manifest 里的 Activity/Service 类必须存在
manifest = open(os.path.join(ROOT, "AndroidManifest.xml"), encoding="utf-8").read()
for cls in re.findall(r'android:name="\.([A-Za-z0-9_.]+)"', manifest):
    path = os.path.join(JAVA, "com/mirrordock/companion", cls.replace(".", "/") + ".kt")
    if not os.path.exists(path):
        errors.append(f"Manifest 声明 .{cls} 但源文件不存在: {path}")

# 7) Kotlin 花括号/圆括号平衡（粗检，抓结构性错误）
for dp, _, fs in os.walk(JAVA):
    for f in fs:
        if not f.endswith(".kt"): continue
        p = os.path.join(dp, f)
        src = open(p, encoding="utf-8").read()
        # 粗略剔除字符串与注释，避免误报
        stripped = re.sub(r'"""(?:.|\n)*?"""', '""', src)
        stripped = re.sub(r'(?<!\\)"(?:[^"\\\n]|\\.)*"', '""', stripped)
        stripped = re.sub(r"(?<!\\)'(?:[^'\\\n]|\\.)*'", "''", stripped)
        stripped = re.sub(r'//[^\n]*', '', stripped)
        stripped = re.sub(r'(?s)/\*.*?\*/', '', stripped)
        for open_c, close_c in [("{", "}"), ("(", ")"), ("[", "]")]:
            if stripped.count(open_c) != stripped.count(close_c):
                errors.append(f"{f}: {open_c}{close_c} 不平衡 ({stripped.count(open_c)} vs {stripped.count(close_c)})")

# 7.5) Kotlin 常见编译期陷阱（X10-73 教训：这些坑静态校验最初没覆盖，
#      结果 CI 的「构建 debug APK」步骤失败、Release 被跳过）
#      a) View 没有可写的 minWidth/minHeight（只有 minimumWidth/Height，且
#         TextView 另有 setMinWidth）——对 View/Button 用 minWidth 会编译失败。
#      b) Long 与 IntRange 混用（`if (longValue in 0..100)`）——类型不匹配编译失败。
#      c) API 30+ 的属性/方法未做版本判断（minSdk 26）——运行期 NoSuchMethodError。
#      这里做文本级启发式检查，宁可误报也不漏报。
TRAPS = [
    (
        "min_width_on_view",
        re.compile(r"\bminWidth\s*=|\bminHeight\s*="),
        None,  # 只在 View/Button 上下文里报，见下方细化
    ),
]

for dp, _, fs in os.walk(JAVA):
    for f in sorted(fs):
        if not f.endswith(".kt"):
            continue
        path = os.path.join(dp, f)
        src = open(path, encoding="utf-8").read()
        # a) minWidth/minHeight 赋值：TextView 有 setMinWidth，View/Button 没有。
        #    粗判：所在 apply 块若构造的是 View/Button/ImageView 则可疑。
        for match in re.finditer(r"\b(minWidth|minHeight)\s*=", src):
            line_no = src[: match.start()].count("\n") + 1
            window = src[max(0, match.start() - 400): match.start()]
            # 向上找最近的构造器调用
            ctor = None
            for c in re.finditer(r"\b([A-Z][A-Za-z0-9_]*)\s*\(", window):
                ctor = c.group(1)
            if ctor in ("View", "Button", "ImageView", "EditText", "TextView"):
                # TextView/EditText 有 setMinWidth；View/Button/ImageView 没有。
                if ctor in ("View", "Button", "ImageView"):
                    errors.append(
                        f"{f}:{line_no} {ctor} 没有可写的 {match.group(1)} 属性"
                        f"（编译失败；View 只有 minimumWidth/Height）"
                    )
        # b) Long 值 in IntRange
        for match in re.finditer(
            r"\b(\w+)\s+in\s+([\d_]+)L?\.\.([\d_]+)L?\b", src
        ):
            var, lo, hi = match.group(1), match.group(2), match.group(3)
            line_no = src[: match.start()].count("\n") + 1
            # 两端都带 L 才是 LongRange；只在一端带或都不带 = IntRange。
            if re.search(r"(rtt|elapsed|duration|ms|nanos|diff|took|since|at)", var, re.I):
                text = match.group(0)
                lo_long = text.split("..")[0].rstrip().endswith("L")
                hi_long = text.rstrip().rstrip("L").split("..")[-1].isupper() or text.endswith("L")
                if not (lo_long and hi_long):
                    errors.append(
                        f"{f}:{line_no} 疑似 Long 与 IntRange 混用：{text.strip()}"
                        f"（两端都应带 L：{lo}L..{hi}L）"
                    )
        # c) API 30+ (isLongLived / setLongLived) 未见版本判断
        for match in re.finditer(r"\.(isLongLived|setLongLived)\b", src):
            line_no = src[: match.start()].count("\n") + 1
            window = src[max(0, match.start() - 600): match.start() + 200]
            if "SDK_INT" not in window:
                errors.append(
                    f"{f}:{line_no} {match.group(1)} 是 API 30+，"
                    f"minSdk 26 需先判 SDK_INT（否则运行期 NoSuchMethodError）"
                )

# 8) 未被引用的 string（提示级）
used_strings = set()
for c in layouts.values():
    used_strings |= set(re.findall(r'@string/([A-Za-z0-9_]+)', c))
used_strings |= set(re.findall(r'R\.string\.([A-Za-z0-9_]+)', code))
for s in sorted(defined["string"] - used_strings):
    warnings.append(f"strings.xml: '{s}' 定义但未被引用")

print("=" * 60)
if errors:
    print(f"❌ 发现 {len(errors)} 个错误：")
    for e in errors: print("  -", e)
else:
    print("✅ 静态交叉校验全部通过（XML 合法 / 资源引用完整 / id 与类均存在 / 括号平衡）")
if warnings:
    print(f"\n⚠️  {len(warnings)} 个提示：")
    for w in warnings: print("  -", w)
print("=" * 60)
sys.exit(1 if errors else 0)
