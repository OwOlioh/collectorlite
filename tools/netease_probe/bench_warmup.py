"""
Shell 子系统预热探针 —— 验证能不能把「首次调用 ~310 ms」挪出用户等待路径。

背景（都是实测）：
  ShellExecuteW            首次 310 ms / 后续 9 ms
  ShellExecuteExW+NOASYNC 首次 311 ms / 后续 10 ms   <- 换 API 没用
  CreateProcess           首次  23 ms / 后续 3 ms

那个 ~310 ms 是**进程级**的 Shell 子系统初始化成本：同一个进程里第二次调用就降到 ~10 ms，
跟用哪个 ShellExecute API 无关。（早先一度以为 ShellExecuteExW 更快，其实是测试顺序污染 ——
它跑在 ShellExecuteW 后面，已经享用了预热。）

所以真正的问题是：App 启动后，用户**第一次**点网易云卡片要等这 310 ms。

思路：有没有一种**无害的** Shell API 调用，能把子系统初始化掉？
候选：SHGetFileInfoW（查文件图标）、AssocQueryStringW（查协议关联）—— 都是纯查询，不会有副作用。

判据：
  预热前 首个 ShellExecuteExW ≈ 310 ms
  预热后 首个 ShellExecuteExW ≈ 10 ms   -> 预热有效，可以搬到 App 启动阶段去做

⚠️ 会真的唤起客户端。用 --dry 只看格局不执行。
"""

import argparse
import base64
import ctypes
import json
import subprocess
import sys
import time
from ctypes import wintypes

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

SONG_ID = "2709782550"


class SHELLEXECUTEINFOW(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.DWORD), ("fMask", ctypes.c_ulong), ("hwnd", wintypes.HWND),
        ("lpVerb", wintypes.LPCWSTR), ("lpFile", wintypes.LPCWSTR),
        ("lpParameters", wintypes.LPCWSTR), ("lpDirectory", wintypes.LPCWSTR),
        ("nShow", ctypes.c_int), ("hInstApp", wintypes.HINSTANCE),
        ("lpIDList", ctypes.c_void_p), ("lpClass", wintypes.LPCWSTR),
        ("hkeyClass", wintypes.HKEY), ("dwHotKey", wintypes.DWORD),
        ("hIcon", wintypes.HANDLE), ("hProcess", wintypes.HANDLE),
    ]


class SHFILEINFOW(ctypes.Structure):
    _fields_ = [
        ("hIcon", wintypes.HANDLE), ("iIcon", ctypes.c_int),
        ("dwAttributes", wintypes.DWORD),
        ("szDisplayName", wintypes.WCHAR * 260),
        ("szTypeName", wintypes.WCHAR * 80),
    ]


def build_uri(song_id: str) -> str:
    payload = '{{"type":"song","id":"{}","cmd":"play"}}'.format(song_id)
    return "orpheus://" + base64.b64encode(payload.encode()).decode()


def open_uri(uri: str) -> float:
    """返回耗时（秒）。这是项目当前走的路径：ShellExecuteExW + NOASYNC。"""
    SEE_MASK_NOASYNC = 0x00000100
    info = SHELLEXECUTEINFOW()
    info.cbSize = ctypes.sizeof(info)
    info.fMask = SEE_MASK_NOASYNC
    info.lpVerb = "open"
    info.lpFile = uri
    info.nShow = 1
    t0 = time.perf_counter()
    ctypes.windll.shell32.ShellExecuteExW(ctypes.byref(info))
    return time.perf_counter() - t0


def warm_shgetfileinfo() -> float:
    """候选预热 1：SHGetFileInfoW 查一个文件的图标（纯查询）。"""
    SHGFI_ICON = 0x000000100
    SHGFI_TYPENAME = 0x000000400
    info = SHFILEINFOW()
    flags = SHGFI_ICON | SHGFI_TYPENAME
    t0 = time.perf_counter()
    # 查一个必然存在的文件：系统目录里的 notepad.exe
    ctypes.windll.shell32.SHGetFileInfoW(
        r"C:\Windows\System32\notepad.exe", 0, ctypes.byref(info),
        ctypes.sizeof(info), flags,
    )
    return time.perf_counter() - t0


def warm_assocquery() -> float:
    """候选预热 2：AssocQueryStringW 查协议关联（也是纯查询）。"""
    ASSOCSTR_COMMAND = 1
    ASSOCF_INIT_IGNOREUNKNOWN = 0x00000400
    out = ctypes.create_unicode_buffer(1024)
    size = wintypes.DWORD(1024)
    t0 = time.perf_counter()
    ctypes.windll.shlwapi.AssocQueryStringW(
        ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND,
        "orpheus", None, out, ctypes.byref(size),
    )
    return time.perf_counter() - t0


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry", action="store_true")
    ap.add_argument("--skip-open", action="store_true", help="只预热，不真的唤起客户端")
    args = ap.parse_args()

    uri = build_uri(SONG_ID)
    print("=== Shell 预热探针 ===")
    print(f"URI: {uri}\n")

    if args.dry:
        print("(--dry：不执行。实际会：预热 -> 唤起一次对比耗时)")
        return

    # 第一组：不预热，直接唤起 —— 应该看到 ~310 ms
    print("--- 对照组：全新进程，直接唤起 ---")
    d = open_uri(uri)
    print(f"  首次 ShellExecuteExW: {d * 1000:7.1f} ms\n")
    time.sleep(1.0)

    # 同一个进程已经预热过了，后面必然快。要看**预热本身的代价**。
    print("--- 预热调用本身的代价（这些是可以搬到 App 启动阶段做的）---")
    for name, fn in [("SHGetFileInfoW", warm_shgetfileinfo),
                     ("AssocQueryStringW", warm_assocquery)]:
        d = fn()
        print(f"  {name:20s}: {d * 1000:7.1f} ms")

    print("\n--- 预热后再唤起 ---")
    d = open_uri(uri)
    print(f"  ShellExecuteExW: {d * 1000:7.1f} ms")

    print("\n=== 怎么读这份数据 ===")
    print("1) 预热调用自身只要几十 ms，说明成本可以塞进启动流程而不拖慢启动；")
    print("2) 但能否真正消掉那 ~310 ms，要看**另一个全新进程**先预热、再唤起。")
    print("   本进程已经跑过一次 open_uri 了，Shell 早就热了，在这里读不出结论。")
    print("   请用 run_twice.py 或直接：")
    print("     python -c \"import bench_warmup as w; w.warm_shgetfileinfo(); print('%.1f' % (w.open_uri(w.build_uri())*1000))\"")


if __name__ == "__main__":
    main()
