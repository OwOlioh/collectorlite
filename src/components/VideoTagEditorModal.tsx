import { useState } from "react";
import { Save, X } from "lucide-react";
import { api } from "../lib/api";
import type { Tag, TagInput, VideoItem } from "../types";
import { TagPoolInput } from "./TagPoolInput";

interface VideoTagEditorModalProps {
  item: VideoItem;
  tagPool: Tag[];
  onClose: () => void;
  onSaved: () => void;
  onTagsChanged: () => void;
}

export function VideoTagEditorModal({
  item,
  tagPool,
  onClose,
  onSaved,
  onTagsChanged
}: VideoTagEditorModalProps) {
  const [selected, setSelected] = useState<Tag[]>(item.tags);
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
      const tagSpecs: TagInput[] = selected.map((tag) => ({
        id: tag.id,
        namespace: tag.namespace,
        name: tag.name,
        color: tag.color,
        categoryId: tag.categoryId
      }));
      await api.updateItemTags(item.id, tagSpecs);
      onTagsChanged();
      onSaved();
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
            <strong>编辑视频标签</strong>
            <p>{item.title}</p>
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
          {saving ? "保存中..." : "保存标签"}
        </button>
      </div>
    </div>
  );
}
