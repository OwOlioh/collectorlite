"""
网易云「当前在播曲目」探针 —— 验证 SMTC 之外的替代数据源。

背景：网易云 PC 客户端不注册 Windows SMTC（需 BetterNCM + InfLink-rs 插件才支持），
所以 System Media Transport Controls 读不到任何会话。但客户端**主窗口标题**自带
正在播放的曲名与歌手，形如：

    我的悲伤是水做的 - ChiliChill乐团/洛天依Official

本探针验证三件事：
  1. 能否用纯 Win32 API 拿到客户端窗口标题（无需注入、无需 hook）
  2. 标题解析是否稳定（多歌手用 "/" 分隔）
  3. 用标题里的「歌名 + 歌手」去 weapi 搜索反查 song_id，能否拼出可点击的深链

用法：
  python probe_nowplaying.py        # 读一次当前播的歌
  python probe_nowplaying.py --watch 10   # 连续观察 10 次（每 3 秒），用于确认切歌时会不会自动变

前置：打开网易云 PC 客户端并播放任意歌曲。Web 版无效（标题不会变）。
"""

import ctypes
import json
import sys
import time
from ctypes import wintypes

import probe_api as netease

NETEASE_EXE = "cloudmusic.exe"


# ── Win32：枚举可见窗口标题 ──────────────────────────────────────────────────
EnumWindowsProc = ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)


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


def netease_titles() -> list:
    """返回 cloudmusic.exe 的所有可见窗口标题。"""
    found = []

    def _cb(hwnd, _lparam):
        if not ctypes.windll.user32.IsWindowVisible(hwnd):
            return True
        n = ctypes.windll.user32.GetWindowTextLengthW(hwnd)
        if not n:
            return True
        buf = ctypes.create_unicode_buffer(n + 1)
        ctypes.windll.user32.GetWindowTextW(hwnd, buf, n + 1)
        pid = wintypes.DWORD()
        ctypes.windll.user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if NETEASE_EXE in _proc_name(pid.value).lower():
            found.append(buf.value)
        return True

    ctypes.windll.user32.EnumWindows(EnumWindowsProc(_cb), 0)
    return found


# ── 解析： 曲名 - 歌手A/歌手B ────────────────────────────────────────────────
def parse_title(raw: str):
    """把窗口标题拆成 (曲名, [歌手])。拆不出返回 None。"""
    sep = " - "
    if sep not in raw:
        return None
    # 用最后一个分隔符切：曲名里通常不会有 " - "，副标题形式的歌手串在尾部
    name, rest = raw.rsplit(sep, 1)
    artists = [a.strip() for a in rest.split("/") if a.strip()]
    if not name.strip() or not artists:
        return None
    return name.strip(), artists


# ── 反查：歌名 + 歌手 → song_id ─────────────────────────────────────────────
def resolve(title: str, artists: list):
    """匿名搜索反查。**宁可查不到也不要猜**：只有歌手名能对上才认。"""
    res = netease.post(
        "/weapi/search/get",
        {"s": title, "type": 1, "limit": 20, "offset": 0, "csrf_token": ""},
    )
    songs = (res.get("result") or {}).get("songs") or []
    for s in songs:
        names = [a.get("name", "") for a in s.get("artists") or []]
        hit = set(artists) & set(names)
        if s.get("name") == title and hit:
            return s, "exact"
    return None, "miss"


def run_once() -> None:
    titles = netease_titles()
    playing = [t for t in titles if " - " in t]

    if not titles:
        print("  没找到网易云窗口 —— 请打开网易云 PC 客户端并播放歌曲")
        return
    if not playing:
        print(f"  找到网易云窗口但没有在播曲目。标题：{titles}")
        return

    raw = playing[0]
    print(f"  窗口标题：{raw}")
    parsed = parse_title(raw)
    if not parsed:
        print("  标题格式无法解析")
        return
    title, artists = parsed
    print(f"  解析结果：曲名={title!r}  歌手={artists}")

    song, kind = resolve(title, artists)
    if song:
        names = "/".join(a.get("name", "") for a in song.get("artists") or [])
        print(f"  反查 [{kind}]：id={song.get('id')}  {song.get('name')} — {names}")
        print(f"  可造深链：orpheus://song/{song.get('id')}")
        print(f"  可建条目：external_id = netease:{song.get('id')}")
    else:
        print("  反查：没匹配上（可能是纯音乐/DJ/付费专属），降级为『粘贴分享链接』")


def main() -> None:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")

    watch = 0
    if "--watch" in sys.argv:
        i = sys.argv.index("--watch")
        watch = int(sys.argv[i + 1]) if len(sys.argv) > i + 1 else 5

    print("=== 网易云「当前在播」获取探针（SMTC 替代方案）===")
    if watch:
        for i in range(watch):
            print(f"\n--- 第 {i + 1}/{watch} 次 ---")
            run_once()
            if i < watch - 1:
                time.sleep(3)
        print("\n提示：观察期间试着切歌，看曲名会不会跟着变。")
    else:
        run_once()


if __name__ == "__main__":
    main()
