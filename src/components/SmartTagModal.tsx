import { useEffect, useMemo, useState } from "react";
import { X, Sparkles, Check } from "lucide-react";
import { useToast } from "./Toast";
import type { SmartTagCandidate, SmartTagMatch, Tag } from "../types";
import { TagBadge } from "./TagBadge";

interface SmartTagModalProps {
  count: number;
  tagPool: Tag[];
  /** 智能匹配候选：从已有标签库检索匹配来源文本的标签，及逐条命中溯源。 */
  smartCandidates: SmartTagCandidate[];
  /** 被选中的内容列表（含无匹配标签的项），供「按内容查看」逐条确认。 */
  selectedItems: { id: number; title: string }[];
  onClose: () => void;
  onApplySmartItem: (itemId: number, tagIds: number[]) => void;
}

type ProcessedMode = "applied" | "none" | "ignored";

interface ProcessedItem {
  id: number;
  title: string;
  mode: ProcessedMode;
  tagIds: number[];
}

export function SmartTagModal({
  count,
  tagPool,
  smartCandidates,
  selectedItems,
  onClose,
  onApplySmartItem
}: SmartTagModalProps) {
  const { toast } = useToast();
  // 每条内容的显式勾选（itemId -> 已选标签 id 集合）。缺省 = 空（默认不勾选，由用户主动选择）。
  const [smartItemSel, setSmartItemSel] = useState<Map<number, Set<number>>>(new Map());
  const [applyingItemId, setApplyingItemId] = useState<number | null>(null);
  // 已处理（确认 / 无适用 / 忽略）的内容，最新的排在前面，渲染时置于底部「已处理」区。
  const [processed, setProcessed] = useState<ProcessedItem[]>([]);
  const [error, setError] = useState("");

  // 把「标签 → 命中项」的候选翻转为「内容 → 匹配到的标签 + 逐条命中溯源」。
  const byItem = useMemo(() => {
    const map = new Map<number, { tag: Tag; matches: SmartTagMatch[] }[]>();
    for (const c of smartCandidates) {
      for (const mt of c.matches) {
        let arr = map.get(mt.itemId);
        if (!arr) {
          arr = [];
          map.set(mt.itemId, arr);
        }
        const group = arr.find((g) => g.tag.id === c.tag.id);
        if (group) group.matches.push(mt);
        else arr.push({ tag: c.tag, matches: [mt] });
      }
    }
    return map;
  }, [smartCandidates]);

  // 某内容当前应选哪些标签：有显式勾选用勾选；否则默认空（需用户主动勾选）。
  const selectedTagIdsForItem = (itemId: number): number[] => {
    const exp = smartItemSel.get(itemId);
    return exp ? [...exp] : [];
  };

  const toggleItemTag = (itemId: number, tagId: number) => {
    setSmartItemSel((current) => {
      const set = new Set(current.get(itemId) ?? []);
      if (set.has(tagId)) set.delete(tagId);
      else set.add(tagId);
      const next = new Map(current);
      next.set(itemId, set);
      return next;
    });
  };

  const markProcessed = (item: ProcessedItem) => {
    setSmartItemSel((current) => {
      const next = new Map(current);
      next.delete(item.id);
      return next;
    });
    setProcessed((current) => [item, ...current.filter((p) => p.id !== item.id)]);
  };

  const confirmItem = async (item: { id: number; title: string }) => {
    const groups = byItem.get(item.id) ?? [];
    if (groups.length === 0) return; // 无匹配项走「忽略」，不走确认
    const tagIds = selectedTagIdsForItem(item.id);
    setApplyingItemId(item.id);
    setError("");
    try {
      if (tagIds.length > 0) {
        await onApplySmartItem(item.id, tagIds);
      }
      // 勾选为 0 表示「匹配标签都不合适」→ 标记为无适用（none）；否则正常应用。
      const mode: ProcessedMode = tagIds.length > 0 ? "applied" : "none";
      markProcessed({ id: item.id, title: item.title, mode, tagIds });
      toast(
        "success",
        tagIds.length > 0
          ? `已为《${item.title}》添加 ${tagIds.length} 个标签`
          : `已确认《${item.title}》无适用标签`
      );
    } catch (err) {
      setError(String(err));
    } finally {
      setApplyingItemId(null);
    }
  };

  const ignoreItem = (item: { id: number; title: string }) => {
    markProcessed({ id: item.id, title: item.title, mode: "ignored", tagIds: [] });
  };

  // Ctrl+Z：撤销上一次确认 / 忽略（最新处理的项，即 processed[0]）。
  useEffect(() => {
    if (processed.length === 0) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && (e.key === "z" || e.key === "Z")) {
        e.preventDefault();
        undoProcessed(processed[0].id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [processed]);

  const undoProcessed = (id: number) => {
    setProcessed((current) => current.filter((p) => p.id !== id));
  };

  const processedIds = new Set(processed.map((p) => p.id));
  const pendingItems = selectedItems.filter((item) => !processedIds.has(item.id));

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div
        className="video-tag-editor-modal"
        role="dialog"
        aria-modal="true"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="modal-head">
          <div>
            <strong>
              <Sparkles size={14} /> 智能匹配标签
            </strong>
            <p>从来源文本检索匹配（不新建标签），逐条确认</p>
          </div>
          <button className="icon-button" type="button" onClick={onClose}>
            <X size={16} />
          </button>
        </div>

        <div className="batch-remove-section smart-match-section">
          <p className="section-hint">
            以下按<strong>每条收藏</strong>列出它从来源文本（标题 / 描述 / 作者 / 分区）中匹配到的标签，
            <strong>默认不勾选</strong>，勾选要打的标签后点「确认」。若匹配标签都不合适，可直接
            「确认（0）」标记为无适用。无匹配的可用「忽略」跳过。已处理项移到底部「已处理」区，
            <strong>Ctrl+Z</strong> 可撤销上一次确认 / 忽略。
          </p>
          {tagPool.length === 0 ? (
            <div className="empty-hint">标签库为空，无法智能匹配</div>
          ) : (
            <>
              {pendingItems.length === 0 ? (
                <div className="empty-hint">全部处理完成</div>
              ) : (
                <div className="smart-by-item-list">
                  {pendingItems.map((item) => {
                    const groups = byItem.get(item.id) ?? [];
                    const selectedIds = selectedTagIdsForItem(item.id);
                    const applying = applyingItemId === item.id;
                    return (
                      <div key={item.id} className="smart-by-item">
                        <div className="smart-by-item-head">
                          <span className="smart-by-item-title" title={item.title}>
                            《{item.title}》
                          </span>
                          {groups.length === 0 ? (
                            <button
                              className="secondary-button small"
                              type="button"
                              onClick={() => ignoreItem(item)}
                            >
                              <X size={14} />
                              忽略
                            </button>
                          ) : (
                            <button
                              className="secondary-button small"
                              type="button"
                              onClick={() => confirmItem(item)}
                              disabled={applying}
                            >
                              <Check size={14} />
                              {applying ? "确认中..." : `确认（${selectedIds.length}）`}
                            </button>
                          )}
                        </div>
                        {groups.length > 0 && (
                          <div className="smart-by-item-tags">
                            {groups.map((g) => {
                              const checked = selectedIds.includes(g.tag.id);
                              return (
                                <div
                                  key={g.tag.id}
                                  className={`smart-item-tag${checked ? " checked" : ""}`}
                                >
                                  <TagBadge
                                    tag={g.tag}
                                    selected={checked}
                                    onClick={() => toggleItemTag(item.id, g.tag.id)}
                                  />
                                  <ul className="smart-match-list">
                                    {g.matches.map((mt, i) => (
                                      <li key={i} className="smart-match-row">
                                        <span className="smart-match-field">
                                          {mt.fieldLabel}
                                        </span>
                                        <span className="smart-match-snippet">
                                          {mt.before}
                                          <mark>{mt.hit}</mark>
                                          {mt.after}
                                        </span>
                                      </li>
                                    ))}
                                  </ul>
                                </div>
                              );
                            })}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}

              {processed.length > 0 && (
                <div className="smart-processed">
                  <div className="smart-processed-head">已处理 {processed.length}</div>
                  {processed.map((p, idx) => (
                    <div
                      key={p.id}
                      className={`smart-processed-item${idx === 0 ? " latest" : ""}`}
                    >
                      <span className="smart-by-item-title" title={p.title}>
                        《{p.title}》
                      </span>
                      {p.mode === "applied" ? (
                        <span className="smart-processed-tags">
                          {p.tagIds.map((id) => {
                            const t = tagPool.find((x) => x.id === id);
                            return t ? <TagBadge key={id} tag={t} /> : null;
                          })}
                        </span>
                      ) : p.mode === "none" ? (
                        <span className="smart-ignored-label">无适用标签</span>
                      ) : (
                        <span className="smart-ignored-label">已忽略</span>
                      )}
                      <button
                        className="link-button"
                        type="button"
                        onClick={() => undoProcessed(p.id)}
                      >
                        撤销
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </div>
        {error && <div className="alert">{error}</div>}
      </div>
    </div>
  );
}
