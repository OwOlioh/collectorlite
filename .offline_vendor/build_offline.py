#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""把 presentation-demo.html 的外部 CDN 依赖全部内联，生成离线单文件版本。

用法: python build_offline.py
输出: ../presentation-demo.offline.html
"""
import base64
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
SRC = ROOT / "presentation-demo.html"
OUT = ROOT / "presentation-demo.offline.html"

UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")


def curl(url: str, dest: pathlib.Path) -> None:
    if dest.exists() and dest.stat().st_size > 0:
        return
    subprocess.run(
        ["curl", "-sSk", "-L", "-m", "90", "-A", UA, "-o", str(dest), url],
        check=True,
    )
    if not dest.exists() or dest.stat().st_size == 0:
        sys.exit(f"下载失败: {url}")


def b64(path: pathlib.Path) -> str:
    return base64.b64encode(path.read_bytes()).decode("ascii")


def js_guard(text: str) -> str:
    return text.replace("</script>", "<\\/script>")


def css_guard(text: str) -> str:
    return text.replace("</style>", "<\\/style>")


# ---------------------------------------------------------------- 1. 资源就绪
print("[1/5] 检查本地资源 ...")
assets = {
    "swiper.min.css": "https://cdn.jsdelivr.net/npm/swiper@11/swiper-bundle.min.css",
    "swiper.min.js": "https://cdn.jsdelivr.net/npm/swiper@11/swiper-bundle.min.js",
    "tailwind.js": "https://cdn.tailwindcss.com/3.4.16",
    "fa.min.css": "https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.4.0/css/all.min.css",
    "fa-solid-900.woff2": "https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.4.0/webfonts/fa-solid-900.woff2",
    "gf.css": ("https://fonts.googleapis.com/css2?family=Inter:wght@400;600;800"
               "&family=JetBrains+Mono:wght@700&display=swap"),
}
for name, url in assets.items():
    curl(url, HERE / name)
    print(f"      {name:22s} {len((HERE / name).read_bytes()) / 1024:8.1f} KB")

# --------------------------------------------- 2. FontAwesome: 内联 solid 字体
print("[2/5] 处理 FontAwesome ...")
fa_css = (HERE / "fa.min.css").read_text(encoding="utf-8")

# 删掉未使用的 @font-face（页面只用 fa-solid），避免离线时产生无谓的失败请求
fa_css = re.sub(
    r"@font-face\s*\{[^}]*?fa-(?:brands-400|regular-400|v4compatibility)[^}]*?\}",
    "",
    fa_css,
    flags=re.S,
)
# solid 的 woff2 转 base64；ttf 回退源已无网络，直接移除其 src 条目
fa_css = fa_css.replace(
    "url(../webfonts/fa-solid-900.woff2)",
    f"url(data:font/woff2;base64,{b64(HERE / 'fa-solid-900.woff2')})",
)
fa_css = re.sub(r"\s*url\(../webfonts/fa-solid-900\.ttf\)", "", fa_css)
fa_css = f"/* FontAwesome 6.4.0 (solid only, 字体已 base64 内联) */\n{fa_css}"

# --------------------------------------------- 3. Google Fonts: 只留 latin 子集
print("[3/5] 处理 Google Fonts ...")
gf_css = (HERE / "gf.css").read_text(encoding="utf-8")
blocks = re.findall(r"/\*\s*([\w-]+)\s*\*/\s*(@font-face\s*\{.*?\})", gf_css, flags=re.S)
keep_subsets = {"latin", "latin-ext"}
font_cache: dict[str, str] = {}
out_blocks = []
for subset, block in blocks:
    if subset not in keep_subsets:
        continue
    m = re.search(r"url\((https://[^)]+\.woff2)\)", block)
    if not m:
        continue
    url = m.group(1)
    if url not in font_cache:
        name = re.sub(r"[^A-Za-z0-9._-]", "_", url.split("/")[-1])
        path = HERE / "gf" / name
        path.parent.mkdir(exist_ok=True)
        curl(url, path)
        font_cache[url] = f"url(data:font/woff2;base64,{b64(path)})"
    out_blocks.append(f"/* {subset} */\n" + block.replace(f"url({url})", font_cache[url]))
gf_inline = ("/* Inter / JetBrains Mono (latin 子集, 已 base64 内联; "
             "中文字体改用系统字体) */\n" + "\n".join(out_blocks))
print(f"      内联 {len(font_cache)} 个字体分片 / {len(out_blocks)} 个 @font-face")

# ------------------------------------------------------------- 4. 组装单文件
print("[4/5] 组装离线单文件 ...")
html = SRC.read_text(encoding="utf-8")

tailwind_js = (HERE / "tailwind.js").read_text(encoding="utf-8")
swiper_css = (HERE / "swiper.min.css").read_text(encoding="utf-8")
swiper_js = (HERE / "swiper.min.js").read_text(encoding="utf-8")

replacements = [
    # head: Tailwind 引擎（须在 tailwind.config 之前）
    ('    <script src="https://cdn.tailwindcss.com"></script>',
     "    <script>/*! Tailwind CSS v3.4.16 (Play CDN, 已内联) */\n"
     + js_guard(tailwind_js) + "\n    </script>"),
    # head: FontAwesome
    ('    <link href="https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.4.0/css/all.min.css" rel="stylesheet">',
     "    <style>\n" + css_guard(fa_css) + "\n    </style>"),
    # head: Swiper CSS
    ('    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swiper@11/swiper-bundle.min.css" />',
     "    <style>/*! Swiper 11 (已内联) */\n" + css_guard(swiper_css) + "\n    </style>"),
    # head: Google Fonts
    ('    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;800&family=JetBrains+Mono:wght@700&family=Noto+Sans+SC:wght@400;700;900&display=swap" rel="stylesheet">',
     "    <style>\n" + css_guard(gf_inline) + "\n    </style>"),
    # body: Swiper JS
    ('    <script src="https://cdn.jsdelivr.net/npm/swiper@11/swiper-bundle.min.js"></script>',
     "    <script>/*! Swiper 11 (已内联) */\n" + js_guard(swiper_js) + "\n    </script>"),
    # 字体栈补系统字体兜底（离线时 Noto Sans SC 可能不存在）
    ("sans: ['Inter', 'Noto Sans SC', 'sans-serif'],",
     "sans: ['Inter', 'Noto Sans SC', 'PingFang SC', 'Microsoft YaHei', 'sans-serif'],"),
    ("mono: ['JetBrains Mono', 'monospace']",
     "mono: ['JetBrains Mono', 'Consolas', 'monospace']"),
]

for old, new in replacements:
    if old not in html:
        sys.exit(f"未能在源文件中定位片段: {old[:70]}...")
    html = html.replace(old, new, 1)

# --------------------------------------------------------------- 5. 校验输出
print("[5/5] 校验 ...")
leftover = re.findall(r'(?:src|href)="(?:https?:)?//[^"]+"', html)
if leftover:
    sys.exit("仍残留外部引用: " + ", ".join(leftover))
html_head_body = re.sub(r"<script>.*?</script>", "", html, flags=re.S)
html_head_body = re.sub(r"<style>.*?</style>", "", html_head_body, flags=re.S)
if re.search(r"(?:src|href)\s*=\s*[\"']https?://", html_head_body):
    sys.exit("标签属性中仍存在外部 URL")

OUT.write_text(html, encoding="utf-8")
size_mb = OUT.stat().st_size / 1024 / 1024
print(f"完成 -> {OUT.name}  ({size_mb:.2f} MB)")
