import { useEffect, useState } from "react";
import { Save, X } from "lucide-react";
import { api } from "../lib/api";
import type { Tag } from "../types";

const colors = ["#3b82f6", "#ef4444", "#f97316", "#10b981", "#8b5cf6", "#0891b2", "#db2777", "#ca8a04"];

interface TagEditorModalProps {
  tag: Tag;
  onClose: () => void;
  onSaved: () => void;
}

export function TagEditorModal({ tag, onClose, onSaved }: TagEditorModalProps) {
  const [name, setName] = useState(tag.name);
  const [color, setColor] = useState(tag.color || colors[0]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  const save = async () => {
    if (!name.trim()) return;
    await api.upsertTag({
      id: tag.id,
      namespace: tag.namespace,
      name: name.trim(),
      color,
      categoryId: tag.categoryId
    });
    onSaved();
    onClose();
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div
        className="tag-editor-modal"
        role="dialog"
        aria-modal="true"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="modal-head">
          <strong>编辑标签</strong>
          <button className="icon-button" type="button" onClick={onClose}>
            <X size={16} />
          </button>
        </div>
        <label className="field-label">名称</label>
        <input value={name} onChange={(event) => setName(event.target.value)} />
        <label className="field-label">颜色</label>
        <div className="color-picker">
          {colors.map((item) => (
            <button
              type="button"
              key={item}
              className={color === item ? "is-active" : ""}
              style={{ background: item }}
              onClick={() => setColor(item)}
              aria-label={`选择颜色 ${item}`}
            />
          ))}
        </div>
        <button className="primary-button wide" type="button" onClick={save}>
          <Save size={16} />
          保存
        </button>
      </div>
    </div>
  );
}
