import { useCallback, useEffect, useRef, useState } from "react";
import { History, RotateCcw, Trash2, Undo2 } from "lucide-react";
import { api, resolveCoverUrl } from "../lib/api";
import type { VideoItem } from "../types";
import { useToast } from "./Toast";
import { getRetentionDays } from "../lib/retention";

interface TrashPageProps {
  onTrashChanged: () => void;
  /** 当前是否显示本视图（App 按 active view 传入）。从其他页切回时自动静默刷新。 */
  isActive?: boolean;
}

function formatRemaining(deletedAt: number, retentionDays: number, now: number): string {
  const deadline = deletedAt + retentionDays * 86400;
  const remain = deadline - now;
  if (remain <= 0) return "已过期，将在下次启动时清除";
  const days = Math.floor(remain / 86400);
  const hours = Math.floor((remain % 86400) / 3600);
  const minutes = Math.floor((remain % 3600) / 60);
  if (days > 0) return `剩余约 ${days} 天 ${hours} 小时`;
  if (hours > 0) return `剩余约 ${hours} 小时 ${minutes} 分钟`;
  return `剩余约 ${minutes} 分钟`;
}

export function TrashPage({ onTrashChanged, isActive = true }: TrashPageProps) {
  const [items, setItems] = useState<VideoItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const { toast } = useToast();
  const retention = getRetentionDays();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setItems(await api.listTrash());
    } finally {
      setLoading(false);
    }
  }, []);

  // 静默刷新：不切 loading 态，避免从其他页面切回时列表闪一下。
  const reloadSilently = useCallback(async () => {
    try {
      setItems(await api.listTrash());
    } catch {
      /* 静默失败：保留现状，下次刷新兜底 */
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 从其他页面切回回收站视图时自动静默刷新（删除/恢复的新状态即时可见）
  const wasActiveRef = useRef(isActive);
  useEffect(() => {
    if (isActive && !wasActiveRef.current) {
      void reloadSilently();
      onTrashChanged();
    }
    wasActiveRef.current = isActive;
  }, [isActive, reloadSilently, onTrashChanged]);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 30000);
    return () => window.clearInterval(timer);
  }, []);

  const refresh = useCallback(() => {
    void load();
    onTrashChanged();
  }, [load, onTrashChanged]);

  const handleRestore = async (item: VideoItem) => {
    setBusyId(item.id);
    try {
      await api.restoreItem(item.id);
      toast("success", `已恢复「${item.title}」`);
      refresh();
    } catch (error) {
      toast("error", `恢复失败：${String(error)}`);
    } finally {
      setBusyId(null);
    }
  };

  const handlePurge = async (item: VideoItem) => {
    if (!window.confirm(`永久删除「${item.title}」？此操作不可撤销。`)) return;
    setBusyId(item.id);
    try {
      await api.purgeItem(item.id);
      toast("info", `已永久删除「${item.title}」`);
      refresh();
    } catch (error) {
      toast("error", `删除失败：${String(error)}`);
    } finally {
      setBusyId(null);
    }
  };

  const handleRestoreAll = async () => {
    if (items.length === 0) return;
    try {
      await api.restoreItems(items.map((item) => item.id));
      toast("success", `已恢复 ${items.length} 条收藏`);
      refresh();
    } catch (error) {
      toast("error", `恢复失败：${String(error)}`);
    }
  };

  const handleEmpty = async () => {
    if (items.length === 0) return;
    if (!window.confirm(`永久清空回收站中的 ${items.length} 条收藏？此操作不可撤销。`)) return;
    try {
      await api.emptyTrash();
      toast("info", "回收站已清空");
      refresh();
    } catch (error) {
      toast("error", `清空失败：${String(error)}`);
    }
  };

  return (
    <section className="page trash-page">
      <header className="page-header">
        <div>
          <h1>回收站</h1>
          <p>
            删除的收藏会先进入回收站，{retention} 天内可恢复；超过保留期将在应用启动时自动清除。
          </p>
        </div>
        <div className="trash-actions">
          <button
            className="ghost-button"
            type="button"
            disabled={items.length === 0}
            onClick={handleRestoreAll}
          >
            <RotateCcw size={16} />
            全部恢复
          </button>
          <button
            className="ghost-button danger"
            type="button"
            disabled={items.length === 0}
            onClick={handleEmpty}
          >
            <Trash2 size={16} />
            清空回收站
          </button>
        </div>
      </header>

      {loading ? (
        <div className="empty-state">加载中…</div>
      ) : items.length === 0 ? (
        <div className="empty-state">
          <History size={28} />
          <p>回收站是空的。删除的收藏会显示在这里，{retention} 天内可恢复。</p>
        </div>
      ) : (
        <ul className="trash-list">
          {items.map((item) => {
            const cover = resolveCoverUrl(item.coverUrl, item.coverLocalPath);
            return (
              <li key={item.id} className="trash-row">
                <div className="trash-cover">
                  {cover ? (
                    <img src={cover} alt="" loading="lazy" />
                  ) : (
                    <div className="trash-cover-fallback" />
                  )}
                </div>
                <div className="trash-info">
                  <div className="trash-title" title={item.title}>
                    {item.title}
                  </div>
                  <div className="trash-meta">
                    <span className="trash-source">{item.source}</span>
                    {item.deletedAt ? (
                      <span className="trash-countdown">
                        {formatRemaining(item.deletedAt, retention, now)}
                      </span>
                    ) : null}
                  </div>
                </div>
                <div className="trash-row-actions">
                  <button
                    className="ghost-button"
                    type="button"
                    disabled={busyId === item.id}
                    onClick={() => handleRestore(item)}
                  >
                    <Undo2 size={15} />
                    恢复
                  </button>
                  <button
                    className="ghost-button danger"
                    type="button"
                    disabled={busyId === item.id}
                    onClick={() => handlePurge(item)}
                  >
                    <Trash2 size={15} />
                    永久删除
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
