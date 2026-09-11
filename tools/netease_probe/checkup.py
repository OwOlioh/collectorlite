"""
网易云接入 · 一键体检

把三个探针合成一次运行，输出一份结论报告，对应三个阶段的可行性：

  [1] weapi 加密 + 匿名接口  ->  P2（歌单导入 / 自动同步）的地基。无需配合。
  [2] SMTC 媒体会话          -> 已于 2026-09-11 判负：网易云原生不注册 SMTC。此项仅作参考。
  [3] 我的歌单（需 cookie）  ->  P2 最后一块拼图。需要浏览器登录态。
  [4] orpheus:// 深链        ->  P0（在客户端里打开）。会真的唤起网易云，默认跳过。

**P1 浮窗速记改由「读网易云窗口标题」实现**（不再依赖 SMTC），见 probe_nowplaying.py。

用法：
  python checkup.py                              # 跑 1、2
  python checkup.py --cookie "MUSIC_U=..."       # 再跑 3
  python checkup.py --deeplink --song 447926067  # 再跑 4（会唤起客户端，**会打断正在听的歌**）

依赖：
  pip install cryptography
  pip install winrt-Windows.Media.Control winrt-Windows.Foundation \
              winrt-Windows.Foundation.Collections winrt-Windows.Storage.Streams
  （别装 winsdk：它打包了全部 Windows API 绑定，装八分钟还没完）
"""

import argparse
import asyncio
import base64
import sys

import probe_deeplink
import probe_nowplaying

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

TICKS_PER_SECOND = 10_000_000
BAR = "=" * 56

results = {}


def mark(key: str, state: str, detail: str = "") -> None:
    results[key] = (state, detail)
    tag = {"ok": "[OK]", "no": "[!!]", "skip": "[--]"}.get(state, "[??]")
    print(f"  {tag} {detail}")


# ── [1] weapi ────────────────────────────────────────────────────────────────
def part_nowplaying() -> None:
    """P1 的新地基：读网易云窗口标题 -> 反查 song_id。替代已判负的 SMTC。"""
    print("\n[2] 当前在播曲目（读客户端窗口标题）")
    try:
        titles = [t for t in probe_nowplaying.netease_titles() if " - " in t]
    except Exception as e:  # noqa: BLE001
        mark("nowplaying", "no", f"读取失败：{type(e).__name__}: {e}")
        return
    if not titles:
        mark("nowplaying", "no", "没读到曲目标题 —— 打开网易云客户端放一首歌后重跑")
        return
    parsed = probe_nowplaying.parse_title(titles[0])
    if not parsed:
        mark("nowplaying", "no", f"标题格式无法解析：{titles[0]!r}")
        return
    title, artists = parsed
    song, kind = probe_nowplaying.resolve(title, artists)
    who = f"{title} - {'/'.join(artists)}"
    if song:
        mark("nowplaying", "ok", f"读到「{who}」→ id={song.get('id')}（{kind}），可造深链")
    else:
        mark("nowplaying", "ok", f"读到「{who}」但反查不到 id（{kind}），降级为复制链接")


def part_api() -> None:
    print("\n[1] weapi 加密 + 匿名接口  （对应 P2 地基，无需配合）")
    try:
        import probe_api
    except Exception as e:  # noqa: BLE001
        mark("api", "no", f"无法导入 probe_api: {e}")
        return
    try:
        res = probe_api.post(
            "/weapi/song/detail",
            {"id": "447926067", "ids": "[447926067]", "csrf_token": ""},
        )
    except Exception as e:  # noqa: BLE001
        mark("api", "no", f"请求异常: {type(e).__name__}: {e}")
        return

    if "__network_error__" in res:
        mark("api", "no", f"网络失败：{res['__network_error__']}")
        return
    if res.get("code") == 200 and res.get("songs"):
        s = res["songs"][0]
        artists = "/".join(a.get("name", "") for a in s.get("artists") or [])
        mark("api", "ok", f"加密正确、匿名可达（样例：{s.get('name')} — {artists}）")
    else:
        mark("api", "no", f"未返回 200：{str(res)[:160]}")


# ── [2] SMTC ─────────────────────────────────────────────────────────────────
async def part_smtc() -> None:
    print("\n[2b] SMTC 媒体会话  （2026-09-11 已判负，仅作参考）")
    print("     网易云PC端原生不注册 SMTC，社区需 BetterNCM + InfLink-rs 插件才能补上。")
    print("     P1 浮窗已改由 [2] 读窗口标题实现，本节结果不影响后续方案。")
    # winsdk 与 winrt 两个包 API 一致，但 winrt 需按分包安装且小得多
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
            mark("smtc", "skip", "未安装 SMTC 依赖，执行：")
            print("       pip install winrt-Windows.Media.Control winrt-Windows.Foundation \\")
            print("                   winrt-Windows.Foundation.Collections winrt-Windows.Storage.Streams")
            return

    try:
        manager = await MediaManager.request_async()
        sessions = manager.get_sessions()
    except Exception as e:  # noqa: BLE001
        mark("smtc", "no", f"读取失败：{type(e).__name__}: {e}")
        return

    # winrt 的 IVectorView 用 size/get_at，winsdk 可直接迭代
    if hasattr(sessions, "size") and hasattr(sessions, "get_at"):
        all_sessions = [sessions.get_at(i) for i in range(sessions.size)]
    else:
        all_sessions = list(sessions)

    if not all_sessions:
        # 没有会话不等于失败，只是没开播放器，属于"待测"而非"不成立"
        mark("smtc", "skip", "当前没有任何媒体会话 —— 请打开网易云并播放一首歌后重跑")
        return

    hit = None
    for s in all_sessions:
        app_id = s.source_app_user_model_id or ""
        if "netease" in app_id.lower() or "cloudmusic" in app_id.lower():
            hit = s
            break

    if hit is None:
        names = [s.source_app_user_model_id for s in all_sessions]
        mark("smtc", "no", f"没找到网易云的会话。当前有：{names}")
        return

    props = await hit.try_get_media_properties_async()
    tl = hit.get_timeline_properties()

    def secs(span) -> float:
        try:
            return span.duration / TICKS_PER_SECOND
        except Exception:  # noqa: BLE001
            return -1.0

    pos, end = secs(tl.position), secs(tl.end_time)
    detail = f"读到「{props.title} — {props.artist}」，进度 {pos:.1f}s / {end:.1f}s"
    mark("smtc", "ok", detail)
    print(f"       进度可读 = {'是' if end > 0 else '否'}（决定能不能做时间戳）")


# ── [3] 我的歌单 ─────────────────────────────────────────────────────────────
def part_playlist(cookie: str) -> None:
    print("\n[3] 我的歌单  （对应 P2 最后一块拼图，需要 cookie）")
    if not cookie:
        mark("playlist", "skip", "未提供 cookie。加 --cookie \"MUSIC_U=...\" 开启")
        return
    import probe_api

    acc = probe_api.post("/weapi/w/nuser/account/get", {"csrf_token": ""}, cookie=cookie)
    uid = (acc.get("account") or {}).get("id") or (acc.get("profile") or {}).get("userId")
    if not uid:
        mark("playlist", "no", f"cookie 可能失效：{str(acc)[:160]}")
        return

    res = probe_api.post(
        "/weapi/user/playlist",
        {"uid": str(uid), "limit": 10, "offset": 0, "csrf_token": ""},
        cookie=cookie,
    )
    playlists = res.get("playlist") if isinstance(res, dict) else None
    if res.get("code") == 200 and playlists:
        names = [f"{p.get('name')}({p.get('trackCount')})" for p in playlists[:5]]
        mark("playlist", "ok", f"uid={uid}，拿到 {len(playlists)} 个歌单：{names}")
    else:
        mark("playlist", "no", f"未返回 200：{str(res)[:200]}")


# ── [4] 深链 ─────────────────────────────────────────────────────────────────
def part_deeplink(song: str, playlist: str, url: str) -> None:
    """
    实测结论（2026-09-11）：三种格式里**只有 base64 JSON 被证明可用**。

      base64 JSON  orpheus://<b64({"type":"song","id":"...","cmd":"play"})>   ✅ 已验证：跳转并开始播放
      路径式       orpheus://song/{id}                                        ⚠️ 未证实能用
      openurl     orpheus://openurl?url=<encoded>                            ❌ 两次实测都没跳转

    判定依据是**网易云窗口标题会不会变成目标曲目**：只有 cmd=play 会改变播放、从而被观测到。
    路径式即使"跳了但没播"也观测不到，所以它没被证伪，只是没法证明 —— 不要当作可用。
    """
    print("\n[4] orpheus:// 深链  （对应 P0，会真的唤起网易云客户端）")

    registered = bool(probe_deeplink.check_registration())
    if not registered:
        mark("deeplink", "no", "orpheus 协议未注册，客户端没装或没接管协议")
        return

    if not (song or playlist or url):
        print("      未提供目标 id，只验证到协议已注册。")
        mark("deeplink", "ok", "协议已注册（未做跳转测试，需带 --song/-id 才能验证精确跳转）")
        return

    # 只用被验证过的格式：base64 JSON + cmd=play
    b64 = f'{{"type":"song","id":"{song}","cmd":"play","channel":"webset"}}'
    uri = "orpheus://" + base64.b64encode(b64.encode("utf-8")).decode()
    print(f"      尝试 [已验证格式] base64 JSON: {uri}")
    ok = probe_deeplink.shell_open(uri, "base64 JSON", wait=6)

    titles = probe_nowplaying.netease_titles()
    print(f"      当前窗口标题：{titles}")
    if ok:
        mark("deeplink", "ok", f"已唤起；请比对窗口标题是否变成目标曲目。当前标题：{titles}")
    else:
        mark("deeplink", "no", "ShellExecuteW 返回码 <=32，应回退浏览器打开")


# ── 汇总 ─────────────────────────────────────────────────────────────────────
def summary() -> None:
    print("\n" + BAR)
    print("结论")
    print(BAR)

    def line(name: str, key: str, when_ok: str, when_no: str) -> None:
        state, detail = results.get(key, ("skip", "未运行"))
        if state == "ok":
            print(f"  {name}: {when_ok}")
        elif state == "no":
            print(f"  {name}: {when_no}  —— {detail}")
        else:
            print(f"  {name}: 待测（{detail}）")

    line("P2 歌单导入 / 自动同步", "api", "地基成立，可以开工", "地基不成立，需换路")
    line("P2 拿到真实收藏数据  ", "playlist", "完全可行", "受阻")
    # SMTC 已判负：网易云原生不注册。P1 改走「读窗口标题」，见 probe_nowplaying.py
    line("P1 浮窗速记（窗口标题）", "nowplaying", "可行", "不成立，退回复制链接方案")
    line("P0 客户端内打开      ", "deeplink", "可行", "受阻")
    print(BAR)

    missing = [k for k in ("api", "nowplaying", "deeplink") if results.get(k, ("skip", ""))[0] == "skip"]
    if missing:
        print(f"\n下一步：补齐未跑的项目 {missing}。"
              "其中 api 无需配合、deeplink 需网易云开着、nowplaying 需网易云正在放歌。")
    if results.get("api", ("", ""))[0] == "ok" and results.get("playlist", ("", ""))[0] != "ok":
        print("P2 只剩登录态一块：python checkup.py --cookie \"MUSIC_U=...\"")
    print()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cookie", default="")
    ap.add_argument("--cookie-file", default="")
    ap.add_argument("--deeplink", action="store_true", help="开启深链测试（会唤起网易云）")
    ap.add_argument("--song", default="")
    ap.add_argument("--playlist", default="")
    ap.add_argument("--url", default="")
    args = ap.parse_args()

    cookie = args.cookie
    if not cookie and args.cookie_file:
        with open(args.cookie_file, encoding="utf-8") as f:
            cookie = f.read().strip()

    print(BAR)
    print("网易云接入 · 一键体检")
    print(BAR)

    part_api()
    part_nowplaying()
    asyncio.run(part_smtc())
    part_playlist(cookie)
    if args.deeplink or args.song or args.playlist or args.url:
        part_deeplink(args.song, args.playlist, args.url)
    else:
        print("\n[4] orpheus:// 深链  （会唤起网易云，默认跳过）")
        mark("deeplink", "skip", "加 --deeplink 开启；带 id 更准：--deeplink --song 447926067")

    summary()


if __name__ == "__main__":
    main()
