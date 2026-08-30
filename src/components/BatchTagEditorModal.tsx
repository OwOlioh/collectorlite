import { useState } from "react";
import { Save, X } from "lucide-react";
import { api } from "../lib/api";
import type { Tag } from "../types";
import { TagPoolInput } from "./TagPoolInput";

interface BatchTagEditorModalProps {
  count: number;
  tagPool: Tag[];
  onClose: () => void;
  onSave: (tags: Tag[]) => void;
  onTagsChanged: () => void;
}

export function BatchTagEditorModal({
  count,
  tagPool,
  onClose,
  onSave,
  onTagsChanged
}: BatchTagEditorModalProps) {
  const [selected, setSelected] = useState<Tag[]>([]);
  const [saving, setSaving] = useState(false);
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
            <strong>批量打标签</strong>
            <p>为选中的 {count} 条收藏添加标签（不影响已有标签）</p>
          </div>
          <button className="icon-button" type="button" onClick={onClose}>
            <X size={16} />
          </button>
        </div>
        <label className="field-label">标签</label>
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
      </div>
    </div>
  );
}
