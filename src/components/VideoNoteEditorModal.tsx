import { useEffect, useState } from "react";
import { Eye, Pencil, Save, X } from "lucide-react";
import { api } from "../lib/api";
import { LinkifiedText } from "../lib/linkify";
import { useToast } from "./Toast";
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
  const [notes, setNotes] = useState("");
  const [mode, setMode] = useState<"edit" | "preview">("edit");
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const { toast } = useToast();

  // 打开弹窗时从数据库读「批注」：轻量批注，独立于 Obsidian 笔记（items.notes），
  // 与浏览器侧边栏「批注模式」共用同一份数据，三者天然同步。批注绝不进 Obsidian。
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api
      .getItemAnnotation(item.id)
      .then((text) => {
        if (!cancelled) setNotes(text);
      })
      .catch(() => {
        // 读不到就当空，不阻断编辑
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [item.id]);

  const save = async () => {
    setSaving(true);
    setError("");
    try {
      await api.updateItemAnnotation(item.id, notes);
      toast("success", "批注已保存");
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

        <div className="note-mode-tabs">
          <button
            type="button"
            className={`note-tab ${mode === "edit" ? "is-active" : ""}`}
            onClick={() => setMode("edit")}
          >
            <Pencil size={14} />
            编辑
          </button>
          <button
            type="button"
            className={`note-tab ${mode === "preview" ? "is-active" : ""}`}
            onClick={() => setMode("preview")}
          >
            <Eye size={14} />
            预览
          </button>
        </div>

        {mode === "edit" ? (
          <textarea
            value={notes}
            onChange={(event) => setNotes(event.target.value)}
            placeholder="写点批注、笔记，或粘贴相关链接..."
          />
        ) : (
          <div className="note-preview">
            {notes.trim() ? (
              <LinkifiedText text={notes} />
            ) : (
              <span className="muted">暂无批注内容</span>
            )}
          </div>
        )}

        {error && <div className="alert">{error}</div>}

        <button
          className="primary-button wide"
          type="button"
          onClick={save}
          disabled={saving || loading}
        >
          <Save size={16} />
          {saving ? "保存中..." : "保存批注"}
        </button>
      </div>
    </div>
  );
}
