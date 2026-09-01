import { useEffect, useMemo, useState } from "react";
import { Check, ChevronLeft, ChevronRight, LoaderCircle, RotateCcw, Trash2, Undo2 } from "lucide-react";
import type { ImportPreview, ImportResult, ItemTagAssignment, Tag as AppTag, TagInput, TagNamespace, VideoItem } from "../../types";
import { TagPoolInput } from "../TagPoolInput";
import { api } from "../../lib/api";
import { useToast } from "../Toast";

interface PerVideoTagState {
  partitionTags: AppTag[];
  partitionManuallyEdited: boolean;
  otherTags: AppTag[];
}

export interface TagEditorProps {
  preview: ImportPreview;
  tagPool: AppTag[];
  onTagsChanged: () => void;
  onBack: () => void;
  onExecute: (result: ImportResult) => void;
  buildImportInput: (assignments: ItemTagAssignment[]) => {
    apiCall: () => Promise<ImportResult>;
  } | null;
}

const PAGE_SIZE = 8;

export function TagEditor({ preview, tagPool, onTagsChanged, onBack, onExecute, buildImportInput }: TagEditorProps) {
  const [folderPartitionEnabled, setFolderPartitionEnabled] = useState(false);
  const [folderPartitionTags, setFolderPartitionTags] = useState<AppTag[]>([]);
  const [perVideoTags, setPerVideoTags] = useState<Record<string, PerVideoTagState>>(() => {
    const states: Record<string, PerVideoTagState> = {};
    const poolByName = new Map(tagPool.map((t) => [t.normalized, t]));
    preview.items.forEach((item) => {
      let partitionTags: AppTag[] = [];
      if (item.partitionName) {
        const existing = poolByName.get(item.partitionName.toLowerCase());
        partitionTags = [
          existing ?? {
            id: 0,
            namespace: "manual",
            name: item.partitionName,
            normalized: item.partitionName.toLowerCase(),
            color: undefined
          }
        ];
      }
      states[item.externalId] = {
        partitionTags,
        partitionManuallyEdited: false,
        otherTags: []
      };
    });
    return states;
  });
  const [currentPage, setCurrentPage] = useState(1);
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [excluded, setExcluded] = useState<Record<string, boolean>>({});
  const { toast } = useToast();

  const activeItems = useMemo(
    () => preview.items.filter((item) => !excluded[item.externalId]),
    [preview.items, excluded]
  );
  const excludedCount = useMemo(
    () => Object.values(excluded).filter(Boolean).length,
    [excluded]
  );
  const totalPages = Math.max(1, Math.ceil(activeItems.length / PAGE_SIZE));
  const safePage = Math.min(currentPage, totalPages);
  const visibleItems = activeItems.slice(
    (safePage - 1) * PAGE_SIZE,
    safePage * PAGE_SIZE
  );
  const selectedCount = useMemo(
    () => Object.values(selected).filter(Boolean).length,
    [selected]
  );
  const allActiveSelected =
    activeItems.length > 0 && activeItems.every((item) => selected[item.externalId]);

  useEffect(() => {
    if (currentPage > totalPages) setCurrentPage(totalPages);
  }, [currentPage, totalPages]);

  const toggleSelect = (externalId: string) =>
    setSelected((current) => {
      const next = { ...current };
      if (next[externalId]) delete next[externalId];
      else next[externalId] = true;
      return next;
    });

  const toggleSelectAll = () => {
    const next = !allActiveSelected;
    setSelected((current) => {
      const copy = { ...current };
      activeItems.forEach((item) => {
        if (next) copy[item.externalId] = true;
        else delete copy[item.externalId];
      });
      return copy;
    });
  };

  const removeSelected = () => {
    setExcluded((current) => {
      const next = { ...current };
      Object.keys(selected).forEach((id) => {
        if (selected[id]) next[id] = true;
      });
      return next;
    });
    setSelected({});
  };

  const removeItem = (externalId: string) => {
    setExcluded((current) => ({ ...current, [externalId]: true }));
    setSelected((current) => {
      const next = { ...current };
      delete next[externalId];
      return next;
    });
  };

  const restoreAll = () => setExcluded({});

  const updateVideoTag = (externalId: string, patch: Partial<PerVideoTagState>) => {
    setPerVideoTags((current) => ({
      ...current,
      [externalId]: { ...current[externalId], ...patch }
    }));
  };

  const updateFolderPartitionTags = (tags: AppTag[]) => {
    setFolderPartitionTags(tags);
    if (!folderPartitionEnabled) return;
    setPerVideoTags((current) => {
      const next = { ...current };
      Object.entries(next).forEach(([externalId, state]) => {
        if (!state.partitionManuallyEdited) {
          next[externalId] = { ...state, partitionTags: tags };
        }
      });
      return next;
    });
  };

  const createTag = async (name: string, namespace: TagNamespace) => {
    const tag = await api.upsertTag({ namespace, name });
    onTagsChanged();
    return tag;
  };

  const buildTagSpecs = (item: VideoItem, state: PerVideoTagState): TagInput[] => {
    const specs: TagInput[] = [];
    state.partitionTags.forEach((tag) => {
      if (tag.id === 0) {
        specs.push({ namespace: "manual", name: tag.name });
      } else {
        specs.push({ id: tag.id, namespace: tag.namespace, name: tag.name, color: tag.color });
      }
    });
    state.otherTags.forEach((tag) => {
      specs.push({ id: tag.id, namespace: tag.namespace, name: tag.name, color: tag.color });
    });
    return specs;
  };

  const execute = async () => {
    const assignments: ItemTagAssignment[] = activeItems.map((item) => ({
      externalId: item.externalId,
      tagSpecs: buildTagSpecs(item, perVideoTags[item.externalId])
    }));
    const input = buildImportInput(assignments);
    if (!input) return;
    setBusy(true);
    try {
      onExecute(await input.apiCall());
    } catch (err) {
      toast("error", String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="import-tag-editor">
      <div className="import-editor-toolbar">
        <button className="ghost-button" type="button" onClick={onBack}>
          <Undo2 size={16} />
          返回
        </button>
        <div>
          <h2>{preview.collection.title}</h2>
          <p>
            共 {activeItems.length} 条
            {excludedCount > 0 ? `（已排除 ${excludedCount} 条）` : ""}
            ，当前第 {safePage} / {totalPages} 页。
          </p>
        </div>
      </div>

      <div className="folder-tag-config">
        <label className="checkbox-line">
          <input
            type="checkbox"
            checked={folderPartitionEnabled}
            onChange={(event) => {
              setFolderPartitionEnabled(event.target.checked);
              if (!event.target.checked) return;
              setPerVideoTags((current) => {
                const next = { ...current };
                Object.entries(next).forEach(([externalId, state]) => {
                  if (!state.partitionManuallyEdited) {
                    next[externalId] = { ...state, partitionTags: folderPartitionTags };
                  }
                });
                return next;
              });
            }}
          />
          <span>
            <strong>为整个收藏夹设置分区标签</strong>
            <small>该标签会先应用到所有项目，后续可逐条修改。</small>
          </span>
        </label>
        <div className="folder-partition-input">
          <TagPoolInput
            pool={tagPool}
            selected={folderPartitionTags}
            namespace="manual"
            disabled={!folderPartitionEnabled}
            onAdd={(tag) => updateFolderPartitionTags([...folderPartitionTags, tag])}
            onRemove={(tag) =>
              updateFolderPartitionTags(folderPartitionTags.filter((t) => t.id !== tag.id))
            }
            onCreate={createTag}
            placeholder="例如：知识、科技"
          />
        </div>
      </div>

      <div className="selection-toolbar">
        <label className="select-all-line">
          <input
            type="checkbox"
            checked={allActiveSelected}
            onChange={toggleSelectAll}
          />
          <span>全选</span>
        </label>
        <div className="bulk-actions">
          <button
            className="ghost-button"
            type="button"
            onClick={removeSelected}
            disabled={selectedCount === 0}
          >
            <Trash2 size={15} />
            移除选中（{selectedCount}）
          </button>
          {excludedCount > 0 && (
            <button className="ghost-button" type="button" onClick={restoreAll}>
              <RotateCcw size={15} />
              恢复全部（{excludedCount}）
            </button>
          )}
        </div>
        {excludedCount > 0 && (
          <span className="bulk-count">已排除 {excludedCount} 条，将不会导入</span>
        )}
      </div>

      {activeItems.length === 0 ? (
        <div className="empty-state">
          <div className="empty-state-icon"><Trash2 size={22} /></div>
          <h2>已全部移除</h2>
          <p>当前没有可导入的项目。点击下方按钮可恢复全部。</p>
          <button className="secondary-button" type="button" onClick={restoreAll}>
            <RotateCcw size={15} /> 恢复全部
          </button>
        </div>
      ) : (
        <>
          <div className="per-video-tag-list">
        {visibleItems.map((item) => {
          const state = perVideoTags[item.externalId];
          if (!state) return null;
          return (
            <article className="per-video-tag-card" key={item.externalId}>
              <div className="per-video-tag-card-head">
                <label className="compact-checkbox">
                  <input
                    type="checkbox"
                    checked={!!selected[item.externalId]}
                    onChange={() => toggleSelect(item.externalId)}
                  />
                  <span>选择</span>
                </label>
                <button
                  className="per-video-remove"
                  type="button"
                  title="移除（不导入此项）"
                  onClick={() => removeItem(item.externalId)}
                >
                  <Trash2 size={15} />
                </button>
              </div>
              <div className="video-tag-summary">
                <div className="preview-cover">
                  {item.coverUrl ? <img src={item.coverUrl} alt="" /> : <span>无封面</span>}
                </div>
                <div>
                  <strong>{item.title}</strong>
                  <span>{item.authorName} · {item.partitionName || "未分区"}</span>
                </div>
              </div>
              <div className="video-tag-fields">
                <label className="field-label">分区标签</label>
                <TagPoolInput
                  pool={tagPool}
                  selected={state.partitionTags}
                  namespace="manual"
                  onAdd={(tag) =>
                    updateVideoTag(item.externalId, {
                      partitionTags: [...state.partitionTags, tag],
                      partitionManuallyEdited: true
                    })
                  }
                  onRemove={(tag) =>
                    updateVideoTag(item.externalId, {
                      partitionTags: state.partitionTags.filter((t) => t.id !== tag.id),
                      partitionManuallyEdited: true
                    })
                  }
                  onCreate={createTag}
                  placeholder="检索或新建分区标签"
                />
              </div>
              <div className="video-other-tags">
                <TagPoolInput
                  pool={tagPool}
                  selected={state.otherTags}
                  onAdd={(tag) =>
                    updateVideoTag(item.externalId, {
                      otherTags: [...state.otherTags, tag]
                    })
                  }
                  onRemove={(tag) =>
                    updateVideoTag(item.externalId, {
                      otherTags: state.otherTags.filter((t) => t.id !== tag.id)
                    })
                  }
                  onCreate={createTag}
                  placeholder="检索或新建其他标签"
                />
              </div>
            </article>
          );
        })}
          </div>

          <div className="pagination-bar">
            <button
              className="ghost-button"
              type="button"
              disabled={safePage <= 1}
              onClick={() => setCurrentPage((page) => Math.max(1, page - 1))}
            >
              <ChevronLeft size={16} />
              上一页
            </button>
            <span>{safePage} / {totalPages}</span>
            <button
              className="ghost-button"
              type="button"
              disabled={safePage >= totalPages}
              onClick={() => setCurrentPage((page) => Math.min(totalPages, page + 1))}
            >
              下一页
              <ChevronRight size={16} />
            </button>
          </div>
        </>
      )}

      <div className="import-confirm-row">
        <button className="primary-button" type="button" onClick={execute} disabled={busy}>
          {busy ? <LoaderCircle className="spin" size={17} /> : <Check size={17} />}
          确认导入
        </button>
      </div>
    </div>
  );
}