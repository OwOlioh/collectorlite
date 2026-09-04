import { useEffect, useState } from "react";
import { ExternalLink, Eye, FileText, Pencil, Save, X } from "lucide-react";
import { api } from "../lib/api";
import { LinkifiedText } from "../lib/linkify";
import { useToast } from "./Toast";
import type { ObsidianSettings, VideoItem } from "../types";

interface VideoNoteEditorModalProps {
  item: VideoItem;
  onClose: () => void;
  onSaved: () => void;
  /** 导出到 Obsidian 成功后调用：父组件刷新列表，让 item.obsidianPath 尽快反映新状态。 */
  onExported?: () => void;
}

export function VideoNoteEditorModal({
  item,
  onClose,
  onSaved,
  onExported
}: VideoNoteEditorModalProps) {
  const [notes, setNotes] = useState(item.notes || "");
  const [mode, setMode] = useState<"edit" | "preview">("edit");
  const [saving, setSaving] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [opening, setOpening] = useState(false);
  const [obsidian, setObsidian] = useState<ObsidianSettings | null>(null);
  const [error, setError] = useState("");
  // 该收藏是否已有对应的 Obsidian 笔记（导出成功或原本就有映射才算）。
  // 没有笔记时「在 Obsidian 中打开」置灰 —— 得先导出生成笔记才能打开。
  const [synced, setSynced] = useState(!!item.obsidianPath);
  const { toast } = useToast();

  // 每次打开弹窗都重新拉取联动设置，避免「设置页开启联动后，收藏库的
  // obsidianEnabled 状态不刷新」导致导出入口消失的问题。
  useEffect(() => {
    void api.getObsidianSettings().then(setObsidian).catch(() => setObsidian(null));
  }, []);

  // 打开弹窗时从数据库确认该收藏是否已有 Obsidian 笔记。
  // item 快照的 obsidianPath 可能过时（比如上次会话已导出、但列表还没刷新），
  // 这里以数据库为准，有就点亮「在 Obsidian 中打开」。
  useEffect(() => {
    void api
      .getItemObsidianPath(item.id)
      .then((path) => {
        if (path) setSynced(true);
      })
      .catch(() => {});
  }, [item.id]);

  const obsidianReady = !!obsidian?.enabled && obsidian.vaultPath.trim() !== "";

  const save = async () => {
    setSaving(true);
    setError("");
    try {
      const updated = await api.updateItemNotes(item.id, notes);
      if (updated.obsidianPath) {
        setSynced(true);
        toast("success", "批注已保存，并同步到 Obsidian");
      } else if (obsidianReady && notes.trim()) {
        toast("success", "批注已保存（联动已开启，保存时自动同步）");
      } else {
        toast("success", "批注已保存");
      }
      onSaved();
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  // 导出 = 先把当前编辑的批注落库，再显式导出到 Obsidian，保证导出的是最新内容
  const exportToObsidian = async () => {
    setExporting(true);
    try {
      await api.updateItemNotes(item.id, notes);
      const n = await api.exportItemsToObsidian([item.id]);
      if (n > 0) {
        setSynced(true);
        toast("success", "已导出到 Obsidian");
        onExported?.();
      } else {
        toast("info", "暂无批注内容，未导出（先写点批注吧）");
      }
    } catch (e) {
      toast("error", `导出失败: ${String(e)}`);
    } finally {
      setExporting(false);
    }
  };

  const openInObsidian = async () => {
    setOpening(true);
    try {
      await api.openNoteInObsidian(item.id);
    } catch (err) {
      toast("error", `打开失败: ${String(err)}`);
    } finally {
      setOpening(false);
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
            placeholder="输入批注、笔记，或粘贴相关链接..."
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

        {obsidianReady && (
          <div className="obsidian-note-zone">
            <p className="obsidian-note-hint">
              {synced
                ? "该收藏已同步到 Obsidian，可直接打开"
                : "先写批注并「导出到 Obsidian」生成笔记，才能打开"}
            </p>
            <div className="obsidian-note-buttons">
              <button
                className="ghost-button"
                type="button"
                onClick={exportToObsidian}
                disabled={exporting}
              >
                <FileText size={16} />
                {exporting ? "导出中..." : "导出到 Obsidian"}
              </button>
              <button
                className="ghost-button"
                type="button"
                onClick={openInObsidian}
                disabled={opening || !synced}
                title={synced ? "在 Obsidian 中打开该笔记" : "尚无对应笔记，请先导出"}
              >
                <ExternalLink size={16} />
                {opening ? "打开中..." : "在 Obsidian 中打开"}
              </button>
            </div>
          </div>
        )}

        <button className="primary-button wide" type="button" onClick={save} disabled={saving}>
          <Save size={16} />
          {saving ? "保存中..." : "保存批注"}
        </button>
      </div>
    </div>
  );
}
