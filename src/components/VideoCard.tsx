import { FileText, Globe, Pencil, Trash2 } from "lucide-react";
import { resolveCoverUrl } from "../lib/api";
import { formatDate, formatDuration } from "../lib/format";
import type { VideoItem } from "../types";
import { TagBadge } from "./TagBadge";
import { CoverImage } from "./CoverImage";

interface VideoCardProps {
  item: VideoItem;
  isSelected: boolean;
  onToggleSelect: (id: number) => void;
  onOpen: (url: string) => void;
  onEditTags: (item: VideoItem) => void;
  onEditNote: (item: VideoItem) => void;
  onDelete: (item: VideoItem) => void;
}

export function VideoCard({
  item,
  isSelected,
  onToggleSelect,
  onOpen,
  onEditTags,
  onEditNote,
  onDelete,
}: VideoCardProps) {
  const isBrowser = item.source === "browser";
  const cover = resolveCoverUrl(item.coverUrl, item.coverLocalPath);

  return (
    <article className="video-card">
      <label
        className={`video-select-checkbox ${isSelected ? "is-checked" : ""}`}
        title="选择视频"
      >
        <input
          type="checkbox"
          checked={isSelected}
          onChange={() => onToggleSelect(item.id)}
        />
      </label>
      <button
        className="video-cover-button"
        type="button"
        onClick={() => onOpen(item.sourceUrl)}
        title="在浏览器打开"
      >
        {isBrowser ? (
          item.coverUrl ? (
            <div className="browser-cover-placeholder">
              <img src={item.coverUrl} alt="" className="browser-favicon" />
            </div>
          ) : (
            <div className="browser-cover-placeholder">
              <Globe size={28} />
            </div>
          )
        ) : cover ? (
          <CoverImage src={cover} alt="" />
        ) : (
          <div className="cover-placeholder">无封面</div>
        )}
        {!isBrowser && item.duration != null && (
          <span className="duration">{formatDuration(item.duration)}</span>
        )}
      </button>
      <div className="video-card-body">
        <button
          type="button"
          className="video-title"
          onClick={() => onOpen(item.sourceUrl)}
        >
          {item.title}
        </button>
        <div className="video-meta">
          {isBrowser ? (
            <span>{formatDate(item.favoriteTime || item.publishedAt)}</span>
          ) : (
            <>
              <span>{item.authorName || "未知作者"}</span>
              {item.partitionName && <span>{item.partitionName}</span>}
              <span>{formatDate(item.favoriteTime || item.publishedAt)}</span>
            </>
          )}
        </div>
        <div className="video-tag-line">
          <div className="card-tags">
            {item.tags.slice(0, 3).map((tag) => (
              <TagBadge key={tag.id} tag={tag} compact />
            ))}
            {item.tags.length > 3 && (
              <span className="muted">+{item.tags.length - 3}</span>
            )}
          </div>
          <button
            className="icon-button card-note-button"
            type="button"
            onClick={() => onEditNote(item)}
            title="编辑视频批注"
          >
            <FileText size={14} />
          </button>
          <button
            className="icon-button danger card-delete-button"
            type="button"
            onClick={() => onDelete(item)}
            title="删除本地视频"
          >
            <Trash2 size={14} />
          </button>
          <button
            className="icon-button card-edit-button"
            type="button"
            onClick={() => onEditTags(item)}
            title="编辑视频标签"
          >
            <Pencil size={14} />
          </button>
        </div>
      </div>
    </article>
  );
}
