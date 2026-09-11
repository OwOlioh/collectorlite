"""
网易云「歌单导入」全链路验证（P2 的最后一块拼图）。

2026-09-11 用真实 cookie 实测通过，结论全部固化在这里，app 实现时直接照抄。

前置：把 cookie 写进 cookie.txt（只需含 MUSIC_U）。
  ⚠️ cookie.txt 等同密码，已在 .gitignore 里。

用法：
  python probe_playlist.py                      # 列歌单
  python probe_playlist.py --playlist 5124170445 --limit 20   # 拉某歌单的曲目
  python probe_playlist.py --playlist 5124170445 --count-only # 只算量，不拉详情

实测结论（重要）：
  1. 登录态校验 + 取 uid：
       POST /weapi/w/nuser/account/get  {csrf_token:""}  -> account.id / profile.userId
     ⚠️ /weapi/user/playlist 传 uid="" 会 400「请求参数错误」，**必须先拿 uid**。
  2. 歌单列表：
       POST /weapi/user/playlist  {uid, limit, offset, csrf_token}
     -> 含「我喜欢的音乐」（specialType=5）与所有自建/订阅歌单。
  3. 歌单曲目：
       POST /weapi/v6/playlist/detail  {id, n, offset, total, csrf_token}
     -> playlist.trackIds[]，**一次就返回全部**（2934 首实测 1.7s，无需分页）。
     ❌ /weapi/playlist/track/all 实测 404，不存在。
     ❌ v6/playlist/detail 的 songs 字段是空的，只能拿到 trackIds。
  4. 曲目详情（必须单独拉）：
       POST /weapi/song/detail  {ids: "[...]", csrf_token}
     🔴 **单次最多 201 条**，250 条会被静默截断成 201（不报错！）。
        => 分块大小取 **200**。2934 首 = 15 次请求。
  5. 🔑 **增量同步的关键**：
       trackIds[i] = {id, v, t, at, uid, ...}
       **at = 加入歌单时间（ms），且整表严格倒序（新→旧）**，2934 条实测 0 违例。
     => 同步策略：拉取后从头部扫，遇到 at <= 上次水位就停，只对新 id 拉详情。
        取消收藏 = 全量 id 比对时消失的那些（软删除进回收站）。
"""

import argparse
import datetime
import json
import sys
import time

import probe_api as netease

COOKIE_FILE = "cookie.txt"
UID_FILE = "uid.txt"
CHUNK = 200  # /weapi/song/detail 单次上限 201，取 200 留余量


def load_cookie() -> str:
    with open(COOKIE_FILE, encoding="utf-8") as f:
        return f.read().strip()


def get_uid(cookie: str) -> str:
    """从登录态反查 uid。带缓存，避免重复请求。"""
    try:
        with open(UID_FILE, encoding="utf-8") as f:
            uid = f.read().strip()
        if uid:
            return uid
    except FileNotFoundError:
        pass
    res = netease.post("/weapi/w/nuser/account/get", {"csrf_token": ""}, cookie=cookie)
    prof = (res or {}).get("profile") or {}
    acct = (res or {}).get("account") or {}
    uid = str(prof.get("userId") or acct.get("id") or "")
    if not uid:
        print(f"  拿 uid 失败：{str(res)[:200]}")
        return ""
    print(f"  uid={uid}  昵称={prof.get('nickname')}")
    with open(UID_FILE, "w", encoding="utf-8") as f:
        f.write(uid)
    return uid


def fmt_ms(ms: int) -> str:
    if not ms:
        return "-"
    return datetime.datetime.fromtimestamp(ms / 1000).strftime("%Y-%m-%d %H:%M")


def list_playlists(cookie: str) -> None:
    uid = get_uid(cookie)
    if not uid:
        return
    res = netease.post(
        "/weapi/user/playlist",
        {"uid": uid, "limit": 100, "offset": 0, "csrf_token": ""},
        cookie=cookie,
    )
    pl = (res or {}).get("playlist") or []
    if not pl:
        print(f"  未通过：{str(res)[:250]}")
        return
    mine = [p for p in pl if str((p.get("creator") or {}).get("userId")) == uid]
    print(f"\n共 {len(pl)} 个歌单，其中自己创建的/收藏的 {len(mine)} 个：\n")
    for p in pl:
        own = "*" if str((p.get("creator") or {}).get("userId")) == uid else " "
        special = "  [我喜欢的音乐]" if p.get("specialType") == 5 else ""
        print(
            f"  {own} {str(p.get('name'))[:26]:<28} id={p.get('id'):<14}"
            f"{p.get('trackCount'):>5} 首  更新于 {fmt_ms(p.get('trackUpdateTime'))}{special}"
        )
    print("\n  （* = 自己的；不带 * 的是订阅别人的）")


def fetch_playlist(cookie: str, pid: str, limit: int, count_only: bool) -> None:
    res = netease.post(
        "/weapi/v6/playlist/detail",
        {"id": pid, "n": 1000, "offset": 0, "total": True, "csrf_token": ""},
        cookie=cookie,
    )
    pl = (res or {}).get("playlist") or {}
    tracks = pl.get("trackIds") or []
    if not tracks:
        print(f"  未通过：{str(res)[:250]}")
        return
    print(f"\n歌单：{pl.get('name')}  trackCount={pl.get('trackCount')}")
    print(f"单次返回 trackIds：{len(tracks)} 条")
    print(f"封面：{(pl.get('coverImgUrl') or '')[:90]}")

    ats = [t.get("at") or 0 for t in tracks]
    violations = sum(1 for i in range(1, len(ats)) if ats[i] > ats[i - 1])
    print(f"时间倒序违例：{violations}（0 = 严格新→旧，可安全做增量）")
    print(f"最新：{fmt_ms(ats[0])}   最旧：{fmt_ms(ats[-1])}")

    if count_only:
        n = len(tracks)
        print(f"\n按每批 {CHUNK} 条计算：{n} 首需要 {-(-n // CHUNK)} 次 song/detail 请求")
        return

    ids = [t.get("id") for t in tracks[:limit]]
    t0 = time.time()
    detail = netease.post(
        "/weapi/song/detail", {"ids": json.dumps(ids), "csrf_token": ""}, cookie=cookie
    )
    songs = detail.get("songs") or []
    print(f"\n拉取前 {len(ids)} 首，返回 {len(songs)} 首（耗时 {time.time() - t0:.1f}s）\n")
    for s in songs:
        al = s.get("album") or {}
        artists = "/".join(a.get("name", "") for a in s.get("artists") or [])
        print(
            f"  {s.get('id'):<12} {str(s.get('name'))[:24]:<26}"
            f"{artists[:26]:<28}{al.get('name') or '':<20}"
            f"{(al.get('picUrl') or '')[:46]}"
        )
    print(f"\n  => external_id 形如 netease:{songs[0].get('id') if songs else '?'}")


def main() -> None:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser()
    ap.add_argument("--playlist", help="歌单 id，不给则只列歌单")
    ap.add_argument("--limit", type=int, default=20, help="拉多少首详情（默认 20）")
    ap.add_argument("--count-only", action="store_true", help="只算请求次数，不拉详情")
    args = ap.parse_args()

    cookie = load_cookie()
    print(f"cookie 长度 {len(cookie)}，含 MUSIC_U：{'MUSIC_U' in cookie}")

    if args.playlist:
        fetch_playlist(cookie, args.playlist, args.limit, args.count_only)
    else:
        list_playlists(cookie)


if __name__ == "__main__":
    main()
