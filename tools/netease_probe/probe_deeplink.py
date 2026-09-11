"""
orpheus:// 深链探针 —— 验证能否从外部唤起网易云桌面端，对应 P0「在客户端里打开收藏」。

分两层，可以只跑第一层：
  [1] 静态检查 --check ：只读注册表，**完全无副作用**，回答"协议注册了吗 + exe 在哪"
  [2] 实际唤起         ：调 ShellExecuteW 并返回原始返回码

**为什么不用 os.startfile**：它内部也是 ShellExecute，但成功与否**丢给我们一个异常就只能二选一**，
拿不到返回码。Rust 侧会直接看 `ShellExecuteW` 的返回值（>32 成功），所以 Python 探针必须用同一口径，
否则两边判据不一致。这里用 ctypes 直接调 Win32，做到与 Rust 实现等价。

前置条件：网易云客户端**建议开着**（开着能立刻看到它跳到前台；不开也会被拉起）。
⚠️ 实际唤起会把客户端切到前台并跳转页面，会打断正在听的歌。

用法：
  python probe_deeplink.py --check              # 只查注册，不唤起（安全）
  python probe_deeplink.py --song 447926067     # 路径式
  python probe_deeplink.py --b64-song 447926067 # base64 JSON（官方网页端格式）
  python probe_deeplink.py --playlist 123456789
  python probe_deeplink.py --url "https://music.163.com/#/song?id=447926067"
  python probe_deeplink.py --now                # 用当前正在播的那首歌做目标（先读窗口标题）
"""

import argparse
import base64
import ctypes
import json
import sys
import time
import urllib.parse
import winreg
from ctypes import wintypes

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

NETEASE_EXE = "cloudmusic.exe"

# ShellExecute 错误码：返回 <=32 都是失败。这是 P0 判定"该不该回退浏览器"的依据。
SE_ERRORS = {
    0: "内存不足 / 资源耗尽",
    2: "ERROR_FILE_NOT_FOUND —— 目标文件或协议处理器不存在",
    3: "ERROR_PATH_NOT_FOUND",
    5: "ERROR_ACCESS_DENIED",
    8: "内存不足",
    26: "SE_ERR_SHARE",
    27: "SE_ERR_ASSOCINCOMPLETE —— 关联不完整（多半是协议没接对）",
    28: "SE_ERR_DDETIMEOUT",
    29: "SE_ERR_DDEFAIL",
    30: "SE_ERR_DDEBUSY",
    31: "SE_ERR_NOASSOC —— **协议没有关联程序**（客户端没装 / 没接管协议）",
}


# ── [1] 注册表静态检查 ───────────────────────────────────────────────────────
def read_key(root, path):
    try:
        with winreg.OpenKey(root, path) as k:
            return winreg.QueryValueEx(k, "")[0]
    except FileNotFoundError:
        return None
    except OSError as e:
        return f"<读取失败: {e}>"


def check_registration() -> str:
    """返回 exe 路径；没注册返回空串。这一键同时解决'装没装'和'路径在哪'。"""
    print("\n=== [1] 注册表静态检查（无副作用）===")
    found = ""
    for label, root, path in [
        ("HKCR", winreg.HKEY_CLASSES_ROOT, r"orpheus\shell\open\command"),
        ("HKCU", winreg.HKEY_CURRENT_USER, r"Software\Classes\orpheus\shell\open\command"),
    ]:
        val = read_key(root, path)
        if val:
            print(f"  [{label}] {val}")
            if NETEASE_EXE in str(val).lower():
                found = val
        else:
            print(f"  [{label}] 未注册")

    if found:
        print("  => 协议已注册，深链可用。")
    else:
        print("  => 未检测到 orpheus 协议。确认网易云桌面端已安装且接管了协议。")
    return found


# ── [2] ShellExecuteW 实际唤起 ───────────────────────────────────────────────
def _proc_name(pid: int) -> str:
    PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    h = ctypes.windll.kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not h:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wintypes.DWORD(1024)
        ok = ctypes.windll.kernel32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size))
        return buf.value if ok else ""
    finally:
        ctypes.windll.kernel32.CloseHandle(h)


def foreground_info() -> str:
    hwnd = ctypes.windll.user32.GetForegroundWindow()
    if not hwnd:
        return "(无前台窗口)"
    n = ctypes.windll.user32.GetWindowTextLengthW(hwnd)
    buf = ctypes.create_unicode_buffer(n + 1)
    ctypes.windll.user32.GetWindowTextW(hwnd, buf, n + 1)
    pid = wintypes.DWORD()
    ctypes.windll.user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    name = _proc_name(pid.value).rsplit("\\", 1)[-1]
    return f"{name} | {buf.value!r}"


def shell_open(uri: str, label: str, wait: float = 3.0) -> bool:
    print(f"\n--- {label} ---")
    print(f"  uri: {uri}")
    before = foreground_info()

    try:
        ctypes.windll.ole32.CoInitializeEx(None, 0x2)
    except Exception:  # noqa: BLE001
        pass

    shell32 = ctypes.windll.shell32
    shell32.ShellExecuteW.argtypes = [
        wintypes.HWND,
        wintypes.LPCWSTR,
        wintypes.LPCWSTR,
        wintypes.LPCWSTR,
        wintypes.LPCWSTR,
        ctypes.c_int,
    ]
    shell32.ShellExecuteW.restype = wintypes.HANDLE
    code = int(shell32.ShellExecuteW(None, "open", uri, None, None, 1) or 0)

    if code > 32:
        print(f"  返回码 {code} => 成功（>32）")
    else:
        print(f"  返回码 {code} => 失败：{SE_ERRORS.get(code, '未知错误')}")
        if code <= 32:
            print("  => Rust 侧应据此回退浏览器打开 + toast 提示。")
            return False

    time.sleep(wait)
    after = foreground_info()
    print(f"  唤起前前台：{before}")
    print(f"  唤起后前台：{after}")
    if NETEASE_EXE in after.lower():
        print("  => 网易云已切到前台，唤起成功。")
    else:
        print("  => 前台不是网易云（可能被系统限制了 stealing focus，或被其他窗口盖住）。")
    return True


def b64_song(song_id: str) -> str:
    payload = {"type": "song", "id": str(song_id), "cmd": "play", "channel": "webset"}
    raw = json.dumps(payload, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    return "orpheus://" + base64.b64encode(raw).decode()


def current_song():
    """复用窗口标题探针，拿当前在播曲目的 id，作为深链目标。"""
    try:
        import probe_nowplaying as np
    except ImportError:
        return None
    titles = [t for t in np.netease_titles() if " - " in t]
    if not titles:
        return None
    parsed = np.parse_title(titles[0])
    if not parsed:
        return None
    song, _ = np.resolve(*parsed)
    return parsed, (song or {}).get("id")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="只查注册表，不唤起")
    ap.add_argument("--song", help="歌曲 id（路径式）")
    ap.add_argument("--b64-song", help="歌曲 id（base64 JSON 格式）")
    ap.add_argument("--playlist", help="歌单 id")
    ap.add_argument("--url", help="music.163.com 链接，走 openurl 通用格式")
    ap.add_argument("--now", action="store_true", help="用当前在播曲目做目标")
    args = ap.parse_args()

    print("=== orpheus:// 深链探针 ===")
    registered = bool(check_registration())

    if args.check:
        print("\n(--check 模式，不唤起客户端)")
        return

    if args.now:
        got = current_song()
        if not got:
            print("\n读不到当前在播曲目，请用 --song 手动指定。")
            return
        (title, artists), sid = got
        if not sid:
            print(f"\n当前曲目 {title} - {'/'.join(artists)} 没能反查到 id")
            return
        print(f"\n当前在播：{title} - {'/'.join(artists)}  (id={sid})")
        shell_open(f"orpheus://song/{sid}", "路径式 song（当前在播曲目）")
        shell_open(b64_song(sid), "base64 JSON（当前在播曲目）")
        return

    any_open = False
    if args.url:
        any_open = True
        shell_open("orpheus://openurl?url=" + urllib.parse.quote(args.url, safe=""), "openurl 通用格式")
    if args.song:
        any_open = True
        shell_open(f"orpheus://song/{args.song}", "路径式 song")
    if args.b64_song:
        any_open = True
        shell_open(b64_song(args.b64_song), "base64 JSON song")
    if args.playlist:
        any_open = True
        shell_open(f"orpheus://playlist/{args.playlist}", "路径式 playlist")

    if not any_open:
        print("\n没有指定目标。先跑 --check 看协议在不在，")
        print("再用 --now（当前在播曲目）或 --song <id> 试精确跳转。")

    if registered:
        print("\n注：以上返回码 >32 说明协议注册正常。")
        print("    至于「是否精确跳到目标页面」，需要你自己瞄一眼客户端窗口确认 —— ")
        print("    程序只能验证唤起与前台切换，读不到客户端内部的路由结果。")
    print()


if __name__ == "__main__":
    main()
