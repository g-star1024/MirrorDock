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
