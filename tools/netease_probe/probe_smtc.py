"""
SMTC 探针 —— 验证网易云桌面端是否向 Windows 注册了媒体会话。

这是「浮窗速记」方案的成败前提：SMTC 必须能读到网易云在播的歌名/歌手/进度。

前置条件（需要你配合）：
  1. 打开网易云音乐桌面端
  2. **正在播放**一首歌（暂停状态也算，但最好是播放中）
  3. 然后再跑本脚本

安装依赖：
  pip install winsdk

运行：
  python probe_smtc.py
"""

import asyncio
import sys

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

TICKS_PER_SECOND = 10_000_000  # WinRT TimeSpan 单位是 100ns


def secs(span) -> float:
    try:
        return span.duration / TICKS_PER_SECOND
    except Exception:  # noqa: BLE001
        return -1.0


async def main() -> None:
    try:
        from winsdk.windows.media.control import (  # type: ignore
            GlobalSystemMediaTransportControlsSessionManager as MediaManager,
        )
    except ImportError:
        try:
            from winrt.windows.media.control import (  # type: ignore
                GlobalSystemMediaTransportControlsSessionManager as MediaManager,
            )
        except ImportError:
            print("缺少依赖，请先执行：")
            print("  pip install winrt-Windows.Media.Control winrt-Windows.Foundation \\")
            print("              winrt-Windows.Foundation.Collections winrt-Windows.Storage.Streams")
            return

    manager = await MediaManager.request_async()
    sessions = manager.get_sessions()

    if hasattr(sessions, "size") and hasattr(sessions, "get_at"):
        sessions = [sessions.get_at(i) for i in range(sessions.size)]

    print(f"\n当前媒体会话数：{len(sessions)}")
    if not sessions:
        print("  没有任何媒体会话。请确认网易云**正在播放**，再跑一次。")
        return

    current = manager.get_current_session()
    current_id = current.source_app_user_model_id if current else None

    netease_found = False
    for s in sessions:
        props = await s.try_get_media_properties_async()
        timeline = s.get_timeline_properties()
        playback = s.get_playback_info()
        app_id = s.source_app_user_model_id

        is_netease = "netease" in app_id.lower() or "cloudmusic" in app_id.lower()
        netease_found = netease_found or is_netease
        marker = "  <== 网易云" if is_netease else ""

        print(f"\n  app       : {app_id}{marker}")
        print(f"  title     : {props.title}")
        print(f"  artist    : {props.artist}")
        print(f"  album     : {props.album_title}")
        print(f"  position  : {secs(timeline.position):.1f}s / {secs(timeline.end_time):.1f}s")
        print(f"  status    : {playback.playback_status}")
        print(f"  是当前会话: {app_id == current_id}")

    print()
    if netease_found:
        print("结论：网易云**已注册** SMTC，浮窗方案可行。")
        print("  注意看 position 是否在变 —— 能做时间戳的前提是进度可读。")
    else:
        print("结论：没找到网易云的媒体会话。浮窗方案不成立，需退回其它路径。")
    print()


if __name__ == "__main__":
    asyncio.run(main())
