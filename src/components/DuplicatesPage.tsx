import { useCallback, useEffect, useState } from "react";
import { Check, Copy, HelpCircle } from "lucide-react";
import { api, resolveCoverUrl } from "../lib/api";
import { formatDate } from "../lib/format";
import type { DuplicateGroup, DuplicateItemPreview } from "../types";
import { useToast } from "./Toast";

const SOURCE_LABELS: Record<string, string> = {
  bilibili: "B站",
  browser: "浏览器",
  zhihu: "知乎",
  csdn: "CSDN",
  github: "GitHub",
  netease: "网易云"
};

function sourceLabel(source: string): string {
  return SOURCE_LABELS[source] ?? source;
}

export function DuplicatesPage({
  refreshToken,
  isActive,
  onChanged
}: {
  refreshToken: number;
  isActive: boolean;
  onChanged: () => void;
}) {
  const { toast } = useToast();
  const [groups, setGroups] = useState<DuplicateGroup[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [keepMap, setKeepMap] = useState<Record<string, number>>({});
  const [merging, setMerging] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setGroups(await api.getDuplicateGroups());
    } catch {
      setGroups(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isActive) void load();
  }, [isActive, refreshToken, load]);

  const keepOf = (group: DuplicateGroup): number =>
    keepMap[group.key] ?? group.items[0]?.id;

  const mergeGroup = async (group: DuplicateGroup) => {
    const keepId = keepOf(group);
    const removeIds = group.items.filter((i) => i.id !== keepId).map((i) => i.id);
    if (removeIds.length === 0) return;
    setMerging(true);
    try {
      await api.mergeDuplicates(keepId, removeIds);
      toast("success", `已合并 ${removeIds.length} 条重复到保留项`);
      onChanged();
      await load();
    } catch (e) {
      toast("error", "合并失败：" + (e instanceof Error ? e.message : String(e)));
    } finally {
      setMerging(false);
    }
  };

  const totalDuplicates = groups?.reduce((sum, g) => sum + g.items.length, 0) ?? 0;
  const urlGroupCount = groups?.filter((g) => g.matchType === "url").length ?? 0;
  const fuzzyGroupCount = (groups?.length ?? 0) - urlGroupCount;

  if (loading) {
    return (
      <div className="dup-page">
        <div className="dup-empty">加载中…</div>
      </div>
    );
  }

  if (!groups) {
    return (
      <div className="dup-page">
        <div className="dup-empty">加载失败</div>
      </div>
    );
  }

  if (groups.length === 0) {
    return (
      <div className="dup-page">
        <div className="dup-empty">没有发现重复项 🎉</div>
      </div>
    );
  }

  return (
    <div className="dup-page">
      <header className="dup-header">
        <h2>重复项</h2>
        <p>
          共 {groups.length} 组、{totalDuplicates} 条疑似重复。
          {urlGroupCount > 0 && ` 其中 ${urlGroupCount} 组按归一化链接判定（精确），`}
          {fuzzyGroupCount > 0 && `${fuzzyGroupCount} 组按标题相似判定（疑似，请手动确认）。`}
          勾选每组要保留的一条，其余合并进回收站（可恢复）。
        </p>
      </header>

      <div className="dup-groups">
        {groups.map((group) => {
          const keepId = keepOf(group);
          const removeCount = group.items.length - 1;
          const isFuzzy = group.matchType === "fuzzy";
          return (
            <section className={`dup-group ${isFuzzy ? "is-fuzzy" : ""}`} key={group.key}>
              <div className="dup-group-head">
                {isFuzzy ? (
                  <span className="dup-fuzzy-tag" title={group.key}>
                    <HelpCircle size={13} /> 疑似（标题相似）
                  </span>
                ) : (
                  <code className="dup-key" title={group.key}>
                    {group.key}
                  </code>
                )}
                <span className="dup-count">{group.items.length} 条</span>
                <button
                  type="button"
                  className="dup-merge-btn"
                  disabled={merging || removeCount === 0}
                  onClick={() => mergeGroup(group)}
                >
                  <Copy size={14} /> 合并其余 {removeCount} 条
                </button>
              </div>
              <div className="dup-items">
                {group.items.map((item) => (
                  <DuplicateCard
                    key={item.id}
                    item={item}
                    isKeep={item.id === keepId}
                    onKeep={() =>
                      setKeepMap((m) => ({ ...m, [group.key]: item.id }))
                    }
                    name={`keep-${group.key}`}
                  />
                ))}
              </div>
            </section>
          );
        })}
      </div>
    </div>
  );
}

function DuplicateCard({
  item,
  isKeep,
  onKeep,
  name
}: {
  item: DuplicateItemPreview;
  isKeep: boolean;
  onKeep: () => void;
  name: string;
}) {
  const cover = resolveCoverUrl(item.coverUrl, undefined);
  return (
    <label className={`dup-card ${isKeep ? "is-keep" : ""}`}>
      <input type="radio" name={name} checked={isKeep} onChange={onKeep} />
      <div className="dup-cover">
        {cover ? <img src={cover} alt="" loading="lazy" /> : null}
      </div>
      <div className="dup-meta">
        <div className="dup-title" title={item.title}>
          {item.title}
        </div>
        <div className="dup-sub">
          {sourceLabel(item.source)}
          {item.favoriteTime ? ` · ${formatDate(item.favoriteTime)}` : ""}
        </div>
      </div>
      {isKeep && (
        <span className="dup-keep-tag">
          <Check size={12} /> 保留
        </span>
      )}
    </label>
  );
}
