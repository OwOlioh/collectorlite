"""
打开网易云歌曲的**耗时探针** —— 定位「点了卡片感觉卡」到底卡在哪一步。

只拆三段，每段独立计时：
  [A] Python/JS 侧生成深链 URI             —— 纯 CPU，应该是微秒级
  [B] ShellExecuteW(uri)                  —— 最大嫌疑：Windows Shell 会尝试 DDE 握手，
                                             目标不响应时要等超时，经典卡顿源
  [C] 直接 CreateProcess(exe --webcmd=uri) —— 绕过 Shell 的协议解析与 DDE，通常是快的那个

对照理论：
  注册表里 orpheus 的命令是（注意网易云自己就少写了一个空格）
      "C:\\...\\cloudmusic.exe"--webcmd="%1"
  所以 [C] 是有根有据的做法，不是 hack。

⚠️ 会真的唤起 / 启动客户端，会打断正在听的歌。
   先用 `--dry` 看它打算做什么。

用法：
  python bench_open.py --dry                 # 只打印计划，不动客户端
  python bench_open.py                       # 跑一轮三种方式
  python bench_open.py --rounds 3            # 每种方式跑 3 轮（观察首次 vs 后续差异）
  python bench_open.py --only c              # 只跑某一种（a/b/c 可组合）
"""

import argparse
import base64
import ctypes
import json
import subprocess
import sys
import time
import os
from ctypes import wintypes

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

SONG_ID = "2709782550"
EXE = r"C:\Program Files\Netease\CloudMusic\cloudmusic.exe"

# ShellExecuteEx 用的结构（比 ShellExecuteW 能多传 flag）
class SHELLEXECUTEINFOW(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.DWORD),
        ("fMask", ctypes.c_ulong),
        ("hwnd", wintypes.HWND),
        ("lpVerb", wintypes.LPCWSTR),
        ("lpFile", wintypes.LPCWSTR),
        ("lpParameters", wintypes.LPCWSTR),
        ("lpDirectory", wintypes.LPCWSTR),
        ("nShow", ctypes.c_int),
        ("hInstApp", wintypes.HINSTANCE),
        ("lpIDList", ctypes.c_void_p),
        ("lpClass", wintypes.LPCWSTR),
        ("hkeyClass", wintypes.HKEY),
        ("dwHotKey", wintypes.DWORD),
        ("hIcon", wintypes.HANDLE),
        ("hProcess", wintypes.HANDLE),
    ]


SEE_MASK_NOASYNC = 0x00000100  # 不等 Shell 内部的异步操作
SEE_MASK_NOCLOSEPROCESS = 0x00000040


def build_payload(song_id: str) -> str:
    """与 Rust `netease::song_play_uri` 逐字节一致的格式（单测已锁死）。"""
    return '{{"type":"song","id":"{}","cmd":"play"}}'.format(song_id)


def build_uri(song_id: str) -> str:
    return "orpheus://" + base64.b64encode(build_payload(song_id).encode()).decode()


def step_a(song_id: str) -> tuple:
    t0 = time.perf_counter()
    uri = build_uri(song_id)
    return time.perf_counter() - t0, uri


def shell_execute_w(uri: str) -> tuple:
    """项目现在用的方式：ShellExecuteW，同步阻塞。"""
    t0 = time.perf_counter()
    try:
        ctypes.windll.ole32.CoInitializeEx(None, 0x2)
    except Exception:
        pass
    shell32 = ctypes.windll.shell32
    shell32.ShellExecuteW.argtypes = [
        wintypes.HWND, wintypes.LPCWSTR, wintypes.LPCWSTR,
        wintypes.LPCWSTR, wintypes.LPCWSTR, ctypes.c_int,
    ]
    shell32.ShellExecuteW.restype = wintypes.HANDLE
    try:
        code = int(shell32.ShellExecuteW(None, "open", uri, None, None, 1) or 0)
    except Exception as e:
        return time.perf_counter() - t0, f"异常 {e}"
    return time.perf_counter() - t0, f"返回码 {code}"


def shell_execute_ex(uri: str) -> tuple:
    """加 SEE_MASK_NOASYNC：告诉 Shell 别为了自己的异步回调阻塞调用方。"""
    t0 = time.perf_counter()
    info = SHELLEXECUTEINFOW()
    info.cbSize = ctypes.sizeof(info)
    info.fMask = SEE_MASK_NOASYNC
    info.lpVerb = "open"
    info.lpFile = uri
    info.nShow = 1
    try:
        ctypes.windll.ole32.CoInitializeEx(None, 0x2)
    except Exception:
        pass
    ok = ctypes.windll.shell32.ShellExecuteExW(ctypes.byref(info))
    elapsed = time.perf_counter() - t0
    return elapsed, ("成功" if ok else f"失败 {ctypes.windll.kernel32.GetLastError()}")


def create_process(uri: str) -> tuple:
    """绕过 Shell：直接按注册表的写法拉起 exe。"""
    t0 = time.perf_counter()
    try:
        # registry 里是 exe--webcmd="%1"（缺空格），这里补齐成正常命令行
        subprocess.Popen([EXE, f'--webcmd={uri}'], close_fds=True)
    except Exception as e:
        return time.perf_counter() - t0, f"异常 {e}"
    return time.perf_counter() - t0, "已发起"


def proc_running() -> bool:
    # tasklist 中文系统输出 GBK，且 stdout 在非 TTY 下有时为 None —— 两种都得兜住
    out = subprocess.run(
        ["tasklist", "/FI", "IMAGENAME eq cloudmusic.exe"],
        capture_output=True,
    ).stdout or b""
    text = out.decode("gbk", errors="ignore")
    return "cloudmusic.exe" in text


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--song", default=SONG_ID)
    ap.add_argument("--rounds", type=int, default=1)
    ap.add_argument("--only", default="bcd", help="要跑哪几组：b=ShellExecuteW c=ShellExecuteExW d=CreateProcess")
    ap.add_argument("--dry", action="store_true", help="只打印计划")
    ap.add_argument("--negative", action="store_true",
                    help="负面用例：用不存在的协议，检验三种方式能否如实报错")
    args = ap.parse_args()

    if args.negative:
        # ⚠️ 关键验证：换更快 API 的前提是**错误处理不能退化**。
        # 现有实现靠「返回码 <=32」决定要不要 fallback 浏览器。如果 NOASYNC 下
        # 无效协议也返回成功，用户点下去就是「毫无反应」，比慢更糟。
        bad = "orpheus-nonexistent-scheme-test://abc123"
        print("=== 负面用例：不存在的协议（应当全部判失败）===")
        d, note = shell_execute_w(bad)
        print(f"  ShellExecuteW          : {d * 1000:8.1f} ms  {note}")
        d, note = shell_execute_ex(bad)
        print(f"  ShellExecuteExW+NOASYNC: {d * 1000:8.1f} ms  {note}")
        d, note = create_process(bad)
        print(f"  CreateProcess          : {d * 1000:8.1f} ms  {note}")
        print("\n判据：三者都应判失败，能 fallback 到浏览器。若某方式返回成功，")
        print("      说明用它会导致「点了没反应」，不能采用。")
        return

    dur_a, uri = step_a(args.song)
    print("=== 打开网易云歌曲 耗时探针 ===")
    print(f"歌曲 id       : {args.song}")
    print(f"URI           : {uri}")
    print(f"客户端当前运行 : {'是' if proc_running() else '否'}")
    print(f"[A] 构造 URI  : {dur_a * 1000:.3f} ms   （纯 CPU，应是微秒级）")

    if args.dry:
        print("\n(--dry：未唤起客户端。去掉 --dry 会真的打开，会打断正在听的歌。)")
        print("计划执行：")
        print("  [B] ShellExecuteW          —— 当前实现")
        print("  [C] CreateProcess + --webcmd —— 候选优化")
        return

    print(f"\n每种方式跑 {args.rounds} 轮，观察「首次」与「后续」的差异：\n")
    steps = [
        ("B", "ShellExecuteW（当前实现）", shell_execute_w, "b"),
        ("C", "ShellExecuteExW + NOASYNC", shell_execute_ex, "c"),
        ("D", "CreateProcess + --webcmd", create_process, "d"),
    ]
    results = {}
    for label, name, fn, key in steps:
        if key not in args.only:
            continue
        print(f"--- [{label}] {name} ---")
        times = []
        for i in range(args.rounds):
            dur, note = fn(uri)
            times.append(dur)
            mark = "首次" if i == 0 else f"第{i + 1}轮"
            print(f"  {mark}: {dur * 1000:8.1f} ms   ({note})")
            time.sleep(1.0)
        if times:
            fastest = min(times) * 1000
            results[label] = times
            print(f"  最快: {fastest:.1f} ms\n")

    print("=== 结论提示 ===")
    if not results:
        print("没跑任何方式。")
        return
    base_labels = [l for l in results if l == "B"]
    if base_labels and "D" in results:
        b = min(results["B"]) * 1000
        d = min(results["D"]) * 1000
        print(f"ShellExecuteW  最快 {b:.1f} ms")
        print(f"CreateProcess  最快 {d:.1f} ms")
        if d < b * 0.7:
            print(f"=> 直接拉 exe 快 {b - d:.0f} ms（{(1 - d / b) * 100:.0f}%），值得换。")
        else:
            print("=> 两者差不多，瓶颈不在 Shell 调用本身，要往别处找。")
    print()
    print("参考刻度：<50 ms 人基本无感；~200 ms 开始有「顿一下」的感觉；>500 ms 明显卡。")


if __name__ == "__main__":
    main()
