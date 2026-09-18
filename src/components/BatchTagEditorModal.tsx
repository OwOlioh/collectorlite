import { useState } from "react";
import { Save, X, Trash2 } from "lucide-react";
import { api } from "../lib/api";
import type { Tag } from "../types";
import { TagPoolInput } from "./TagPoolInput";
import { TagBadge } from "./TagBadge";

interface BatchTagEditorModalProps {
  count: number;
  tagPool: Tag[];
  /** 选中项共有的标签（交集）。为空表示没有共同标签。 */
  commonTags: Tag[];
  onClose: () => void;
  onSave: (tags: Tag[]) => void;
  onRemove: (tagIds: number[]) => void;
  onTagsChanged: () => void;
}

export function BatchTagEditorModal({
  count,
  tagPool,
  commonTags,
  onClose,
  onSave,
  onRemove,
  onTagsChanged
}: BatchTagEditorModalProps) {
  const [selected, setSelected] = useState<Tag[]>([]);
  const [removeSelected, setRemoveSelected] = useState<Set<number>>(new Set());
  const [saving, setSaving] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState("");

  const createTag = async (name: string) => {
    const tag = await api.upsertTag({ namespace: "manual", name });
    onTagsChanged();
    return tag;
  };

  const save = async () => {
    setSaving(true);
    setError("");
    try {
      await onSave(selected);
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (removeSelected.size === 0) return;
    setRemoving(true);
    setError("");
    try {
      await onRemove([...removeSelected]);
      // 不关闭弹窗：LibraryPage 刷新列表后共同标签会自动重算，便于继续操作
      setRemoveSelected(new Set());
    } catch (err) {
      setError(String(err));
    } finally {
      setRemoving(false);
    }
  };

  const toggleRemove = (tag: Tag) => {
    setRemoveSelected((current) => {
      const next = new Set(current);
      if (next.has(tag.id)) {
        next.delete(tag.id);
      } else {
        next.add(tag.id);
      }
      return next;
    });
  };

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
            <strong>批量更改标签</strong>
            <p>为选中的 {count} 条收藏添加或移除标签</p>
          </div>
          <button className="icon-button" type="button" onClick={onClose}>
            <X size={16} />
          </button>
        </div>

        {/* 添加标签 */}
        <label className="field-label">添加标签</label>
        <TagPoolInput
          pool={tagPool}
          selected={selected}
          onAdd={(tag) =>
            setSelected((current) =>
              current.some((item) => item.id === tag.id) ? current : [...current, tag]
            )
          }
          onRemove={(tag) =>
            setSelected((current) => current.filter((item) => item.id !== tag.id))
          }
          onCreate={createTag}
          placeholder="检索已有标签，或输入后按空格新建"
        />
        {error && <div className="alert">{error}</div>}
        <button className="primary-button wide" type="button" onClick={save} disabled={saving}>
          <Save size={16} />
          {saving ? "保存中..." : `保存到 ${count} 条`}
        </button>

        <div className="modal-divider" />

        {/* 删除共有标签 */}
        <div className="batch-remove-section">
          <label className="field-label">批量删除标签（共同标签）</label>
          <p className="section-hint">
            以下为选中 {count} 条收藏<strong>共有</strong>的标签，勾选后从它们身上移除。
          </p>
          {commonTags.length === 0 ? (
            <div className="empty-hint">无共同标签</div>
          ) : (
            <>
              <div className="tag-picker-chips">
                {commonTags.map((tag) => (
                  <TagBadge
                    key={tag.id}
                    tag={tag}
                    selected={removeSelected.has(tag.id)}
                    onClick={() => toggleRemove(tag)}
                  />
                ))}
              </div>
              <button
                className="secondary-button danger-action wide"
                type="button"
                onClick={remove}
                disabled={removing || removeSelected.size === 0}
              >
                <Trash2 size={16} />
                {removing ? "删除中..." : `删除选中标签（${removeSelected.size}）`}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
