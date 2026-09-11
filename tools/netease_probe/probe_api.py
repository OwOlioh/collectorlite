"""
网易云 weapi 连通性探针 —— 独立验证脚本，不依赖 collectorlite 任何代码。

目的：用最短时间判断 P2（歌单导入 / 自动同步）是否可行。分两阶段：

  阶段 1（无需任何配合）
    实现 weapi 加密（AES-128-CBC ×2 + RSA no-padding），打一次搜索接口。
    通过 = 加密实现正确 + 网络可达。这是 P2 的地基。

  阶段 2（需要浏览器登录态）
    带 MUSIC_U cookie 调「我的歌单」接口。
    通过 = 能拿到用户真实收藏数据，P2 完全可行。

用法：
  python probe_api.py                        # 只跑阶段 1
  python probe_api.py --cookie "MUSIC_U=xxx" # 两个阶段都跑
  python probe_api.py --cookie-file cookie.txt

说明：阶段 1 是纯网络验证，不需要打开网易云客户端。
"""

import argparse
import base64
import json
import random
import ssl
import string
import sys
import urllib.error
import urllib.parse
import urllib.request

try:
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
except ImportError:
    print("缺少依赖，请先执行： pip install cryptography")
    sys.exit(1)

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

# ── weapi 固定常量（公开算法） ───────────────────────────────────────────────
MODULUS = (
    "00e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b7"
    "25152b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e0"
    "312ecbda92557c93870114af6c9d05c4f7f0c3685b7a46bee255932575cce10"
    "b424d813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3ece0462db0"
    "a22b8e7"
)
NONCE = "0CoJUm6Qyw8W8jud"
PUBKEY = "010001"
IV = b"0102030405060708"

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
)


def aes_cbc_b64(plain: str, key: str) -> str:
    """AES-128-CBC 加密后 base64。填充用 PKCS7，按**字节**长度算（中文关键点）。"""
    raw = plain.encode("utf-8")
    pad = 16 - len(raw) % 16
    raw += bytes([pad]) * pad
    enc = Cipher(algorithms.AES(key.encode("utf-8")), modes.CBC(IV)).encryptor()
    return base64.b64encode(enc.update(raw) + enc.finalize()).decode()


def rsa_no_padding(seckey: str) -> str:
    """RSA 加密随机密钥：明文反转后做模幂，无填充，输出 256 位 hex。"""
    n = int(MODULUS, 16)
    e = int(PUBKEY, 16)
    m = int.from_bytes(seckey[::-1].encode("utf-8"), "big")
    return format(pow(m, e, n), "x").zfill(256)


def weapi(payload: dict) -> dict:
    """把请求体包成 weapi 的 params / encSecKey。"""
    text = json.dumps(payload, separators=(",", ":"))
    seckey = "".join(random.choice(string.ascii_letters + string.digits) for _ in range(16))
    return {
        "params": aes_cbc_b64(aes_cbc_b64(text, NONCE), seckey),
        "encSecKey": rsa_no_padding(seckey),
    }


def make_ctx() -> ssl.SSLContext:
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    return ctx


def fetch_anon_cookie() -> str:
    """访问首页拿匿名 cookie（NMTID 等）。很多 weapi 端点不接受完全空的 cookie。"""
    req = urllib.request.Request("https://music.163.com/", headers={"User-Agent": UA})
    try:
        with urllib.request.urlopen(req, timeout=20, context=make_ctx()) as resp:
            raw = resp.headers.get_all("Set-Cookie") or []
    except Exception:  # noqa: BLE001
        return ""
    parts = [c.split(";")[0] for c in raw if "=" in c.split(";")[0]]
    return "; ".join(parts)


def post(path: str, payload: dict, cookie: str = "") -> dict:
    url = "https://music.163.com" + path
    body = urllib.parse.urlencode(weapi(payload)).encode("utf-8")
    headers = {
        "User-Agent": UA,
        "Referer": "https://music.163.com/",
        "Content-Type": "application/x-www-form-urlencoded",
    }
    if cookie:
        headers["Cookie"] = cookie
    req = urllib.request.Request(url, data=body, headers=headers)
    ctx = make_ctx()
    try:
        with urllib.request.urlopen(req, timeout=20, context=ctx) as resp:
            return json.loads(resp.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        return {"__http_error__": e.code, "body": e.read().decode("utf-8", "replace")[:300]}
    except Exception as e:  # noqa: BLE001
        return {"__network_error__": f"{type(e).__name__}: {e}"}


def show(label: str, res: dict) -> None:
    if "__network_error__" in res:
        print(f"  [网络失败] {res['__network_error__']}")
        return
    if "__http_error__" in res:
        print(f"  [HTTP {res['__http_error__']}] {res['body']}")
        return
    code = res.get("code")
    print(f"  [code={code}] {json.dumps(res, ensure_ascii=False)[:400]}")
    return code


def phase1() -> bool:
    print("\n=== 阶段 1：weapi 加密 + 匿名可达性（无需登录态）===")
    anon = fetch_anon_cookie()
    print(f"  匿名 cookie：{anon[:80] or '(获取失败)'}")

    # 从最宽松的端点开始试，任何 200 都说明「加密正确 + 匿名可访问」
    candidates = [
        ("/weapi/song/detail", {"id": "447926067", "ids": "[447926067]", "csrf_token": ""}),
        ("/weapi/song/enhance/player/url", {"ids": "[447926067]", "br": "128000", "csrf_token": ""}),
        (
            "/weapi/cloudsearch/get/web",
            {"s": "周杰伦", "type": 1, "limit": 3, "offset": 0, "total": "true", "csrf_token": ""},
        ),
        ("/weapi/search/get", {"s": "周杰伦", "type": 1, "limit": 3, "offset": 0, "csrf_token": ""}),
    ]

    ok = False
    for path, payload in candidates:
        res = post(path, payload, cookie=anon)
        if "__network_error__" in res:
            print(f"\n  {path}\n    [网络失败] {res['__network_error__']}")
            continue
        if "__http_error__" in res:
            print(f"\n  {path}\n    [HTTP {res['__http_error__']}] {res['body'][:200]}")
            continue
        code = res.get("code")
        snippet = json.dumps(res, ensure_ascii=False)[:220]
        print(f"\n  {path}\n    [code={code}] {snippet}")
        if code == 200:
            print("    ^ 通过")
            ok = True
            if path.endswith("song/detail") and res.get("songs"):
                s = res["songs"][0]
                artists = "/".join(a.get("name", "") for a in s.get("artists") or [])
                print(f"    样例：{s.get('name')} — {artists} (id={s.get('id')})")

    print()
    if ok:
        print("  结论：weapi 加密实现正确，匿名可访问。P2 地基成立。")
    else:
        print("  结论：所有端点都没返回 200。")
        print("    —— 注意：能拿到结构化 JSON（而不是乱码/502）就说明**加密是对的**，")
        print("       被拒是业务规则（多半是要求登录态），不是算法问题。")
    return ok


def phase2(cookie: str) -> None:
    print("\n=== 阶段 2：带 cookie 拉「我的歌单」===")
    acc = post("/weapi/w/nuser/account/get", {"csrf_token": ""})
    uid = (acc.get("account") or {}).get("id") or (acc.get("profile") or {}).get("userId")
    if not uid:
        print("  拿不到 uid，可能 cookie 已失效：")
        show("account/get", acc)
        return
    print(f"  uid = {uid}")

    res = post(
        "/weapi/user/playlist",
        {"uid": str(uid), "limit": 10, "offset": 0, "csrf_token": ""},
    )
    code = show("user/playlist", res)
    if code == 200:
        playlists = res.get("playlist") or []
        print(f"  拿到 {len(playlists)} 个歌单：")
        for p in playlists[:10]:
            print(f"    - {p.get('name')}  (id={p.get('id')}, {p.get('trackCount')} 首)")
        print("  结论：能拿到用户真实收藏数据，P2 完全可行。")
    else:
        print("  结论：cookie 可能过期或接口口径有变。")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cookie", default="", help="浏览器里的完整 cookie 字符串")
    ap.add_argument("--cookie-file", default="", help="存放 cookie 的文件路径")
    ap.add_argument("--skip-phase1", action="store_true")
    args = ap.parse_args()

    cookie = args.cookie
    if not cookie and args.cookie_file:
        with open(args.cookie_file, encoding="utf-8") as f:
            cookie = f.read().strip()

    ok = True if args.skip_phase1 else phase1()
    if cookie:
        phase2(cookie)
    else:
        print("\n（未提供 cookie，跳过阶段 2。）")
        print("  想跑阶段 2：在浏览器登录 https://music.163.com ，")
        print('  F12 → Network → 任选一个请求 → 复制 Request Headers 里的 cookie，')
        print('  然后： python probe_api.py --cookie "MUSIC_U=..."')
        print('  关键是要有 MUSIC_U 这一项。')
    print()


if __name__ == "__main__":
    main()
