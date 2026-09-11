"""
网易云**扫码登录** —— 拿 MUSIC_U cookie 的推荐方式，不用手动开 F12 复制。

实测（2026-09-11）两个接口在 weapi 加密下都正常：

  POST /weapi/login/qrcode/unikey          -> {"code":200,"unikey":"..."}
  POST /weapi/login/qrcode/client/login    -> 801 等待扫码 / 802 待确认 / 803 成功（响应 Set-Cookie 带回 MUSIC_U）

二维码内容由我们本地拼 ''https://music.163.com/login?codekey=<unikey>'' 后自己渲染，
不需要走官方的 create 接口（那个 /api/login/qrcode/create 实测 404）。

⚠️ 2026-09-11 实测：**这条路目前走不通**。扫码能成功（802），但确认时返回

  code=8821  'message': '请切换其他登录方式或升级新版本再试'  （= 风控，要求行为验证码）

同 IP 连续轰炸后连申请 unikey 都会被拦（`code=-462 检测到您的网络环境存在风险`）。
社区（ncmctl / HyPlayer）也是同样结论。**当前推荐改用浏览器手动复制 cookie**。

🔴 教训：轮询必须复用会话 cookie + 控制频率。第一版每个请求都重新抓匿名 cookie
（内部会 GET 一次首页），等于每 2 秒刷一次 homepage，把自己打进了风控名单。

用法：
  python qr_login.py            # 生成 qr.png 并开始轮询，扫码后自动存 cookie.txt
  python qr_login.py --poll-only KEY

⚠️ cookie.txt 是登录凭证，等同于密码，**别提交进 git**。
"""

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

import qrcode

import probe_api as netease

QR_FILE = "qr.png"
COOKIE_FILE = "cookie.txt"
LOGIN_BASE = "https://music.163.com/login?codekey="

STATUS = {
    800: "二维码已过期，请重跑本脚本",
    801: "等待扫码",
    802: "已扫码，请在手机上确认",
    803: "登录成功",
    8821: "⚠️ 被风控拦截（需要行为验证码），扫码这条路走不通",
}

# 出现这些 code 就不用再轮询了，继续只会加重风控
FATAL = {8821}


class Session:
    """登录流程必须复用同一份 cookie。

    ⚠️ 原来的写法每个请求都调一次 `fetch_anon_cookie()`，等于每次都换一个新的 NMTID，
    在服务端看来是"会话不断跳号"，很可疑（实测会更容易撞上 8821 风控）。
    """

    def __init__(self) -> None:
        self.jar: dict[str, str] = {"os": "pc"}
        for seg in netease.fetch_anon_cookie().split(";"):
            if "=" in seg:
                k, v = seg.split("=", 1)
                self.jar[k.strip()] = v.strip()

    def header(self) -> str:
        return "; ".join(f"{k}={v}" for k, v in self.jar.items())

    def absorb(self, set_cookie: str) -> None:
        for seg in set_cookie.replace("\r", "").split("\n"):
            kv = seg.strip().split(";")[0]
            if "=" in kv:
                k, v = kv.split("=", 1)
                self.jar[k.strip()] = v.strip()


SESSION = Session()


def weapi_post(path: str, payload: dict):
    """返回 (json, response_headers)。登录成功时 cookie 在响应头里。"""
    body = urllib.parse.urlencode(netease.weapi(payload)).encode("utf-8")
    req = urllib.request.Request(
        "https://music.163.com" + path,
        data=body,
        headers={
            "User-Agent": netease.UA,
            "Referer": "https://music.163.com/",
            "Content-Type": "application/x-www-form-urlencoded",
            "Cookie": SESSION.header(),
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=20, context=netease.make_ctx()) as resp:
            raw = resp.read().decode("utf-8", "replace")
            heads = dict(resp.getheaders())
            SESSION.absorb(heads.get("Set-Cookie", ""))
            return json.loads(raw), heads
    except urllib.error.HTTPError as e:
        return {"__http_error__": e.code}, {}
    except Exception as e:  # noqa: BLE001
        return {"__network_error__": f"{type(e).__name__}: {e}"}, {}


def get_unikey() -> str:
    res, _ = weapi_post("/weapi/login/qrcode/unikey", {"type": 1, "csrf_token": ""})
    if res.get("code") != 200:
        print(f"  拿 key 失败：{res}")
        return ""
    return res.get("unikey", "")


def qr_matrix(key: str) -> list[list[bool]]:
    """返回二维码布尔矩阵（不含边框），供外部渲染 SVG / 终端图形。"""
    qr = qrcode.QRCode(border=0)
    qr.add_data(LOGIN_BASE + key)
    qr.make(fit=True)
    return qr.get_matrix()


def make_qr(key: str, path: str) -> None:
    url = LOGIN_BASE + key
    img = qrcode.make(url, box_size=8, border=2)
    img.save(path)
    print(f"  二维码图片已生成：{os.path.abspath(path)}")
    print(f"  扫码内容：{url}")


def issue_round(state_file: str = "qr_state.json") -> str:
    """申请一个新 unikey，把二维码矩阵写进 state_file 供外部渲染。返回 key。"""
    key = get_unikey()
    if not key:
        return ""
    data = {"key": key, "url": LOGIN_BASE + key, "matrix": qr_matrix(key)}
    with open(state_file, "w", encoding="utf-8") as f:
        json.dump(data, f)
    write_svg(data["matrix"], "qr.svg")
    print(f"  [{time.strftime('%H:%M:%S')}] 新二维码已就绪 key={key}")
    return key


def write_svg(matrix, path: str) -> None:
    """把二维码画成 SVG 文件，便于直接在预览面板里展示 / 扫码。"""
    n = len(matrix)
    quiet, unit = 4, 8
    size = (n + quiet * 2) * unit
    runs = []
    for y, row in enumerate(matrix):
        x = 0
        while x < n:
            if row[x]:
                run = 1
                while x + run < n and row[x + run]:
                    run += 1
                runs.append(
                    f'<rect x="{(x + quiet) * unit}" y="{(y + quiet) * unit}" '
                    f'width="{run * unit}" height="{unit}"/>'
                )
                x += run
            else:
                x += 1
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {size} {size}" '
        f'width="{size}" height="{size}" role="img">'
        f'<rect width="{size}" height="{size}" fill="#ffffff"/>'
        f'<g fill="#111418">{"".join(runs)}</g></svg>'
    )
    with open(path, "w", encoding="utf-8") as f:
        f.write(svg)


def daemon(max_rounds: int = 8, state_file: str = "qr_state.json") -> str:
    """二维码只有 ~3 分钟有效期：过期就自动换一张，用户随时扫都行。"""
    for rnd in range(1, max_rounds + 1):
        key = issue_round(state_file)
        if not key:
            print("  申请 unikey 失败，10s 后重试")
            time.sleep(10)
            continue
        deadline = time.time() + 150  # key 约 3 分钟过期，提前一点换
        last = None
        while time.time() < deadline:
            res, heads = weapi_post(
                "/weapi/login/qrcode/client/login",
                {"key": key, "type": 1, "csrf_token": ""},
            )
            code = res.get("code")
            if code != last:
                print(f"  [{time.strftime('%H:%M:%S')}] 第{rnd}轮 code={code}  {STATUS.get(code, str(res)[:100])}")
                last = code
            if code == 803:
                return merge_cookie(res.get("cookie") or "", heads.get("Set-Cookie", ""))
            if code == 800:
                break  # 过期，换新码
            if code in FATAL:
                print(f"  完整响应：{str(res)[:300]}")
                print("  停止轮询 —— 反复重试会加重风控。建议改用浏览器手动取 cookie。")
                sys.exit(2)
            time.sleep(2)
    return ""


def poll(key: str, timeout: int = 300) -> str:
    """轮询直到 803；返回 cookie 字符串（失败返回空）。"""
    print(f"\n开始轮询（最多 {timeout}s）。请用网易云手机 App 扫码：")
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        res, heads = weapi_post(
            "/weapi/login/qrcode/client/login", {"key": key, "type": 1, "csrf_token": ""}
        )
        code = res.get("code")
        msg = STATUS.get(code, str(res)[:120])
        if code != last:
            print(f"  [{time.strftime('%H:%M:%S')}] code={code}  {msg}")
            last = code
        if code == 803:
            cookie = res.get("cookie") or ""
            set_cookie = heads.get("Set-Cookie", "")
            merged = merge_cookie(cookie, set_cookie)
            return merged
        if code == 800:
            return ""
        time.sleep(2)
    print("  超时未扫码。")
    return ""


def merge_cookie(body_cookie: str, set_cookie: str) -> str:
    """登录响应里 cookie 会同时出现在 body 和 Set-Cookie 头，取并集。"""
    parts = {}

    def eat(text: str) -> None:
        for seg in text.replace("\r", "").split("\n"):
            kv = seg.strip().split(";")[0]
            if "=" in kv:
                k, v = kv.split("=", 1)
                parts[k.strip()] = v.strip()

    eat(set_cookie)
    for seg in body_cookie.split(";"):
        if "=" in seg:
            k, v = seg.split("=", 1)
            parts[k.strip()] = v.strip()
    # Set-Cookie 在前，body 的 MUSIC_U 通常更完整，这里让 body 覆盖
    eat(set_cookie)
    return "; ".join(f"{k}={v}" for k, v in parts.items())


def verify(cookie: str) -> None:
    """用拿到的 cookie 调「我的歌单」，这一步通过就代表 P2 完全打通。"""
    print("\n=== 用 cookie 验证「我的歌单」===")
    res = netease.post(
        "/weapi/user/playlist",
        {"uid": "", "limit": 20, "offset": 0, "csrf_token": ""},
        cookie=cookie,
    )
    if isinstance(res, dict) and isinstance(res.get("playlist"), list):
        for p in res["playlist"][:10]:
            print(f"   - {p.get('name')}  (id={p.get('id')}, {p.get('trackCount')} 首)")
        print(f"  共 {len(res['playlist'])} 个歌单 —— P2 完全可行。")
    else:
        print(f"  未通过：{str(res)[:250]}")


def main() -> None:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")

    ap = argparse.ArgumentParser()
    ap.add_argument("--poll-only", help="只对给定的 key 轮询（用于二维码已生成的情况）")
    ap.add_argument("--daemon", action="store_true", help="二维码过期自动换码，长时间等待")
    ap.add_argument("--rounds", type=int, default=8)
    args = ap.parse_args()

    print("=== 网易云扫码登录 ===")
    if args.daemon:
        cookie = daemon(args.rounds)
    elif args.poll_only:
        cookie = poll(args.poll_only)
    else:
        key = get_unikey()
        if not key:
            return
        print(f"  unikey = {key}")
        make_qr(key, QR_FILE)
        cookie = poll(key)

    if not cookie:
        print("\n没拿到 cookie。")
        return

    safe = cookie[:40] + "..." if len(cookie) > 40 else cookie
    print(f"\n拿到 cookie（共 {len(cookie)} 字符）：{safe}")
    with open(COOKIE_FILE, "w", encoding="utf-8") as f:
        f.write(cookie)
    print(f"已写入 {os.path.abspath(COOKIE_FILE)}")

    if "MUSIC_U" in cookie:
        print("包含 MUSIC_U —— 这就是我们需要的登录凭证。")
    else:
        print("⚠️ 没有 MUSIC_U，可能登录态不完整。")

    verify(cookie)


if __name__ == "__main__":
    main()
