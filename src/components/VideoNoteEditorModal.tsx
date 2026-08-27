import { useState } from "react";
import { Save, X } from "lucide-react";
import { api } from "../lib/api";
import type { VideoItem } from "../types";

interface VideoNoteEditorModalProps {
  item: VideoItem;
  onClose: () => void;
  onSaved: () => void;
}

export function VideoNoteEditorModal({
  item,
  onClose,
  onSaved
}: VideoNoteEditorModalProps) {
  const [notes, setNotes] = useState(item.notes || "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  const save = async () => {
    setSaving(true);
    setError("");
    try {
      await api.updateItemNotes(item.id, notes);
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
        className="video-note-editor-modal"
        role="dialog"
        aria-modal="true"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="modal-head">
          <div>
            <strong>视频批注</strong>
            <p>{item.title}</p>
          </div>
          <button className="icon-button" type="button" onClick={onClose}>
            <X size={16} />
          </button>
        </div>
        <textarea
          value={notes}
          onChange={(event) => setNotes(event.target.value)}
          placeholder="输入批注、笔记，或粘贴相关链接..."
        />
        {error && <div className="alert">{error}</div>}
        <button className="primary-button wide" type="button" onClick={save} disabled={saving}>
          <Save size={16} />
          {saving ? "保存中..." : "保存批注"}
        </button>
      </div>
    </div>
  );
}
