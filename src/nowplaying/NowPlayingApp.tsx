import { useEffect, useRef, useState } from "react";
import { api, inTauri } from "../lib/api";
import type { NowPlayingState, NowPlayingTrack, TrackResolveResult, Tag } from "../types";
import { TagPoolInput } from "../components/TagPoolInput";

/**
 * 速记面板。**按需打开、用完即销毁**：窗口活着的时候每秒问一次后端
 * 「网易云在不在出声 + 曲目标题变没变」，销毁后零开销 —— 这点轮询只在
 * 面板存续的几十秒里发生，不违背按需形态。
 *
 * 定位是「随手记」而不是第二个收藏入口：红心的歌由 P2 同步自动进库，
 * 这里只做同步做不到的两件事 —— 即时，以及批注 / 时间戳。
 */
export function NowPlayingApp() {
  const [track, setTrack] = useState<NowPlayingTrack | null>(null);
  /** 客户端在跑但读不到曲目时的具体原因（迷你模式等），直接显示给用户 */
  const [hint, setHint] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [note, setNote] = useState("");
  const [resolved, setResolved] = useState<TrackResolveResult | null>(null);
  const [resolving, setResolving] = useState(false);
  const [saving, setSaving] = useState(false);
  const [flash, setFlash] = useState<string | null>(null);
  /** 已选标签（沿用主窗口 TagPoolInput 的形态：池子里挑 / 输错就新建）。 */
  const [tagPool, setTagPool] = useState<Tag[]>([]);
  const [selectedTags, setSelectedTags] = useState<Tag[]>([]);
  /** 已累计的「播放中」秒数。暂停不走、切歌归零 —— 窗口标题里没有播放进度，
   *  只能这样估：面板打开 / 切歌那一刻开始，只数真正出声的时间。 */
  const [elapsedSec, setElapsedSec] = useState(0);
  /** 网易云是否正在出声（WASAPI 会话状态），控制计时与 UI 提示 */
  const [playing, setPlaying] = useState(false);
  /** 当前曲目标识，用来检测「面板开着的时候切歌了」 */
  const trackKeyRef = useRef("");

  useEffect(() => {
    if (!inTauri()) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    void api
      .nowPlayingCurrent()
      .then((state: NowPlayingState) => {
        if (cancelled) return;
        trackKeyRef.current = state.track
          ? `${state.track.title}|${state.track.artist}`
          : "";
        setTrack(state.track);
        setHint(state.hint);
      })
      .catch(() => undefined)
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // 每秒一次：① 网易云在出声吗 → 在出声计时 +1；② 切歌了吗 → 换曲目、计时归零。
  // 两个都是「读本机状态」的轻调用，面板存活期间可接受。
  useEffect(() => {
    if (!inTauri()) return;
    const id = window.setInterval(() => {
      void (async () => {
        try {
          const state: NowPlayingState = await api.nowPlayingCurrent();
          const key = state.track
            ? `${state.track.title}|${state.track.artist}`
            : "";
          if (key !== trackKeyRef.current) {
            trackKeyRef.current = key;
            setTrack(state.track);
            setHint(state.hint);
            setElapsedSec(0);
          }
          const isPlaying = await api.neteaseIsPlaying();
          setPlaying(isPlaying);
          if (isPlaying) setElapsedSec((s) => s + 1);
        } catch {
          // 单次轮询失败忽略，下一秒重试
        }
      })();
    }, 1000);
    return () => window.clearInterval(id);
  }, []);

  // 标签池：面板打开时拉一次就够。已选是局部状态，关掉面板就丢 —— 用户重新唤起从零开始。
  useEffect(() => {
    if (!inTauri()) return;
    let cancelled = false;
    void api
      .listTags()
      .then((tags) => {
        if (!cancelled) setTagPool(tags);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  // 拿到曲目就反查一次，好把真实的 song id / 封面 / 时长带回来
  useEffect(() => {
    if (!track) return;
    let cancelled = false;
    setResolving(true);
    void api
      .nowPlayingResolve(track.title, track.artist)
      .then((r) => {
        if (!cancelled) setResolved(r);
      })
      .catch(() => undefined)
      .finally(() => {
        if (!cancelled) setResolving(false);
      });
    return () => {
      cancelled = true;
    };
  }, [track?.title, track?.artist]);

  // Esc 关闭。面板是置顶的，给一个键盘出口比只留右上角 × 顺手。
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        void api.nowplayingClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const elapsedLabel = `[${String(Math.floor(elapsedSec / 60)).padStart(2, "0")}:${String(
    elapsedSec % 60
  ).padStart(2, "0")}]`;

  const insertElapsed = () => {
    setNote((prev) => (prev ? `${prev}\n${elapsedLabel} ` : `${elapsedLabel} `));
  };

  const handleSave = async () => {
    if (!track || saving) return;
    setSaving(true);
    setFlash(null);
    try {
      const result = await api.nowPlayingCapture({
        title: track.title,
        artist: track.artist,
        songId: resolved?.songId ?? null,
        coverUrl: resolved?.coverUrl ?? null,
        duration: resolved?.duration ?? null,
        note,
        tags: selectedTags.map((t) => t.name)
      });
      setFlash(
        result.unresolved
          ? "已保存（没匹配到正式条目，用了标题兜底 id）"
          : result.created
            ? "已入库"
            : "已更新批注"
      );
      setNote("");
      setSelectedTags([]);
    } catch (error) {
      setFlash(`保存失败：${String(error)}`);
    } finally {
      setSaving(false);
    }
  };

  if (!inTauri()) {
    return (
      <div className="np-root">
        <div className="np-idle">速记面板仅在桌面端可用</div>
      </div>
    );
  }

  return (
    <div className="np-root">
      <header className="np-header">
        <span className="np-header-title">速记</span>
        <button
          className="np-icon-button"
          type="button"
          title="关闭（Esc）"
          onClick={() => void api.nowplayingClose()}
        >
          ×
        </button>
      </header>

      <section className="np-track">
        {loading && <div className="np-idle">读取中…</div>}
        {!loading && track && (
          <>
            <div className="np-title">{track.title}</div>
            <div className="np-artist">{track.artist}</div>
          </>
        )}
        {!loading && !track && <div className="np-idle">{hint ?? "网易云未运行"}</div>}
      </section>

      <section className="np-panel">
        <div className="np-meta">
          {resolving && <span>识别中…</span>}
          {!resolving && resolved?.resolved && (
            <span className="np-ok">
              已匹配正式条目 · 库内{resolved.inLibrary ? "已有" : "还没有"}
            </span>
          )}
          {!resolving && resolved && !resolved.resolved && (
            <span className="np-warn">没匹配上，仍可记批注（会用标题作兜底 id）</span>
          )}
          {!loading && track && !playing && (
            <span className="np-warn">已暂停 · 计时停走</span>
          )}
        </div>

        <button
          className="np-elapsed"
          type="button"
          disabled={!track}
          onClick={insertElapsed}
        >
          插入时间 {elapsedLabel}（约）
        </button>

        <TagPoolInput
          pool={tagPool}
          selected={selectedTags}
          onAdd={(tag) =>
            setSelectedTags((prev) =>
              prev.some((t) => t.id === tag.id) ? prev : [...prev, tag]
            )
          }
          onRemove={(tag) =>
            setSelectedTags((prev) => prev.filter((t) => t.id !== tag.id))
          }
          onCreate={async (name, namespace) => {
            try {
              return await api.upsertTag({ name, namespace });
            } catch {
              return undefined;
            }
          }}
          placeholder="标签 / 匹配现有"
          namespace="manual"
        />
        <textarea
          className="np-textarea"
          placeholder="这一刻想记点什么…"
          value={note}
          onChange={(e) => setNote(e.target.value)}
        />

        <div className="np-actions">
          <button
            className="np-primary"
            type="button"
            disabled={!track || saving}
            onClick={() => void handleSave()}
          >
            {saving ? "保存中…" : "保存"}
          </button>
          {flash && <span className="np-flash">{flash}</span>}
        </div>
      </section>

      <footer className="np-footer">按下同一快捷键或 Esc 关闭</footer>
    </div>
  );
}
