"""
网易云客户端**本地服务**探针 —— 在 127.0.0.1 上找到客户端自己开的端口，试着跟它说话。

背景：2026-09-11 实测发现网易云主进程在 127.0.0.1 上监听了一个端口（当次是 20017）。
这条通道很可能就是「网页端打开客户端」功能的背后机制 —— 网页要知道客户端在不在、
要能给它传命令。如果它提供本地 HTTP/WebSocket API，价值很大：

  - 读当前播放曲目**和播放进度**（SMTC 给不到的那块就补回来了）
  - 可能能拿到登录态用户信息
  - 可能能下发「打开某首歌/某个歌单」的指令

前置：网易云客户端**必须开着**。

用法：
  python probe_local.py              # 自动发现回环端口并探测常见路径
  python probe_local.py --port 20017 # 指定端口
"""

import argparse
import ctypes
import http.client
import socket
import subprocess
import sys
from ctypes import wintypes

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

CANDIDATE_PATHS = [
    "/",
    "/json/version",
    "/json",
    "/json/list",
    "/api/status",
    "/api/player",
    "/api/user",
    "/status",
    "/ping",
]

PROCESS_QUERY_LIMITED_INFORMATION = 0x1000


def _run_netstat() -> str:
    r = subprocess.run(
        ["netstat", "-ano", "-p", "tcp"],
        capture_output=True,
        text=True,
        encoding="gbk",
        errors="replace",
        timeout=30,
    )
    return r.stdout or ""


def pids_by_image(exe: str) -> set:
    """枚举进程并比对映像名。不依赖 tasklist/Get-Process 的输出格式。"""
    found = set()
    psapi = ctypes.windll.psapi
    kernel32 = ctypes.windll.kernel32
    buf = (wintypes.DWORD * 8192)()
    needed = wintypes.DWORD()
    if not psapi.EnumProcesses(ctypes.byref(buf), ctypes.sizeof(buf), ctypes.byref(needed)):
        return found
    n = needed.value // ctypes.sizeof(wintypes.DWORD)
    for i in range(n):
        pid = buf[i]
        h = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
        if not h:
            continue
        try:
            name = ctypes.create_unicode_buffer(1024)
            size = wintypes.DWORD(1024)
            if kernel32.QueryFullProcessImageNameW(h, 0, name, ctypes.byref(size)):
                if name.value.rsplit("\\", 1)[-1].lower() == exe.lower():
                    found.add(pid)
        finally:
            kernel32.CloseHandle(h)
    return found


def loopback_listen_ports(pids: set) -> list:
    ports = []
    for line in _run_netstat().splitlines():
        if "LISTENING" not in line.upper():
            continue
        parts = line.split()
        if len(parts) < 5 or not parts[-1].isdigit():
            continue
        addr, pid = parts[1], int(parts[-1])
        if pid not in pids:
            continue
        host, _, port = addr.rpartition(":")
        if host in ("127.0.0.1", "::1", "[::1]") and port.isdigit():
            ports.append(int(port))
    return sorted(set(ports))


def get(port: int, path: str, timeout: float = 2.0):
    try:
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
        conn.request("GET", path, headers={"Host": f"127.0.0.1:{port}"})
        resp = conn.getresponse()
        body = resp.read(4000).decode("utf-8", "replace")
        head = dict(resp.getheaders())
        conn.close()
        return resp.status, head, body
    except (ConnectionRefusedError, socket.timeout, TimeoutError):
        return None, None, "timeout/refused"
    except Exception as e:  # noqa: BLE001
        return None, None, f"{type(e).__name__}: {e}"


def probe_port(port: int) -> None:
    print(f"\n=== 探测 127.0.0.1:{port} ===")
    for path in CANDIDATE_PATHS:
        status, head, body = get(port, path)
        if status is None:
            print(f"  {path:<26} {body}")
            continue
        ct = (head or {}).get("Content-Type", "?")
        print(f"  {path:<26} HTTP {status}  {ct}")
        if body:
            print(f"       {body[:300]!r}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=0)
    args = ap.parse_args()

    print("=== 网易云本地服务探针 ===")
    pids = pids_by_image("cloudmusic.exe")
    print(f"cloudmusic PIDs: {sorted(pids)}")
    if not pids:
        print("网易云没在运行 —— 这个探针必须开着客户端才有意义。")
        return

    if args.port:
        ports = [args.port]
    else:
        ports = loopback_listen_ports(pids)
        print(f"回环监听端口：{ports}")

    if not ports:
        print("没发现回环监听 —— 该功能可能已下线，或端口非动态但被防火墙挡住。")
        return

    for p in ports:
        probe_port(p)

    import json

    print("\n提示：端口大概率是**每次启动动态分配**的。")
    print("      真要用，得每次都「枚举进程→查其 LISTEN 端口」动态发现，不能写死。")


if __name__ == "__main__":
    main()
