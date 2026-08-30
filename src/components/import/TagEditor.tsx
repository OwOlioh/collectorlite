import { useState } from "react";
import { Check, ChevronLeft, ChevronRight, LoaderCircle, Undo2 } from "lucide-react";
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
  const { toast } = useToast();

  const totalPages = Math.max(1, Math.ceil(preview.items.length / PAGE_SIZE));
  const visibleItems = preview.items.slice(
    (currentPage - 1) * PAGE_SIZE,
    currentPage * PAGE_SIZE
  );

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
    const assignments: ItemTagAssignment[] = preview.items.map((item) => ({
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
          <p>共 {preview.items.length} 条，当前第 {currentPage} / {totalPages} 页。</p>
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

      <div className="per-video-tag-list">
        {visibleItems.map((item) => {
          const state = perVideoTags[item.externalId];
          if (!state) return null;
          return (
            <article className="per-video-tag-card" key={item.externalId}>
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
          disabled={currentPage <= 1}
          onClick={() => setCurrentPage((page) => Math.max(1, page - 1))}
        >
          <ChevronLeft size={16} />
          上一页
        </button>
        <span>{currentPage} / {totalPages}</span>
        <button
          className="ghost-button"
          type="button"
          disabled={currentPage >= totalPages}
          onClick={() => setCurrentPage((page) => Math.min(totalPages, page + 1))}
        >
          下一页
          <ChevronRight size={16} />
        </button>
      </div>

      <div className="import-confirm-row">
        <button className="primary-button" type="button" onClick={execute} disabled={busy}>
          {busy ? <LoaderCircle className="spin" size={17} /> : <Check size={17} />}
          确认导入
        </button>
      </div>
    </div>
  );
}