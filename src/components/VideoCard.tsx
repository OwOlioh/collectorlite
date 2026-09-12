import {
  AppWindow,
  ExternalLink,
  FileText,
  Globe,
  Pencil,
  Star,
  Trash2
} from "lucide-react";
import { resolveCoverUrl } from "../lib/api";
import { authorProfileUrl, formatDate, formatDuration } from "../lib/format";
import type { OpenTarget, VideoItem } from "../types";
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
  onToggleStar?: (item: VideoItem) => void;
  /** 在原生客户端打开（目前仅网易云有此能力），无客户端时由调用方回退浏览器 */
  onOpenInClient?: (item: VideoItem) => void;
  /** 该来源的打开方式偏好；不给 = 客户端优先 */
  openTarget?: OpenTarget;
}

export function VideoCard({
  item,
  isSelected,
  onToggleSelect,
  onOpen,
  onEditTags,
  onEditNote,
  onDelete,
  onToggleStar,
  onOpenInClient,
  openTarget
}: VideoCardProps) {
  const isBrowser = item.source === "browser";
  const isNetease = item.source === "netease";
  // 只配了浏览器偏好时，才是"浏览器优先"。没配 / 配了 client / 压根没开过设置页
  // 都按「客户端优先」处理 —— 这是 2026-09-11 拍板的默认（DEVELOPMENT.md 9.11-1），
  // 也让老数据自然落到最想要的分支上。
  const supportsClient = isNetease && !!onOpenInClient;
  const clientFirst = supportsClient && openTarget !== "browser";
  const openPrimary = () => {
    if (clientFirst) {
      onOpenInClient?.(item);
      return;
    }
    onOpen(item.sourceUrl);
  };
  const cover = resolveCoverUrl(item.coverUrl, item.coverLocalPath);
  const authorUrl = authorProfileUrl(item.source, item.authorId);
  const starred = item.starred === true;

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
        onClick={openPrimary}
        title={clientFirst ? "在网易云客户端打开并播放" : "在浏览器打开"}
      >
        {isBrowser && item.coverLocalPath ? (
          // 浏览器来源有本地图标（og:image 或 favicon 已落盘）→ 走标准封面大图，
          // 不再保留占位风格 —— og:image 是大图，跟 favicon 视觉规格不一样。
          <CoverImage src={cover} alt="" />
        ) : isBrowser ? (
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
        {onToggleStar && (
          <span
            className={`card-star-button ${starred ? "is-starred" : ""}`}
            role="button"
            tabIndex={0}
            title={starred ? "取消星标（不再置顶）" : "打星置顶"}
            onClick={(event) => {
              event.stopPropagation();
              onToggleStar(item);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                event.stopPropagation();
                onToggleStar(item);
              }
            }}
          >
            <Star size={17} fill={starred ? "currentColor" : "none"} />
          </span>
        )}
      </button>
      <div className="video-card-body">
        <button
          type="button"
          className="video-title"
          onClick={openPrimary}
        >
          {item.title}
        </button>
        <div className="video-meta">
          {isBrowser ? (
            <span>{formatDate(item.favoriteTime || item.publishedAt)}</span>
          ) : (
            <>
              {authorUrl && item.authorName ? (
                <button
                  type="button"
                  className="author-link"
                  onClick={() => onOpen(authorUrl)}
                  title="在浏览器打开作者主页"
                >
                  {item.authorName}
                </button>
              ) : (
                <span>{item.authorName || "未知作者"}</span>
              )}
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
            title="编辑视频批注（可同步到 Obsidian）"
          >
            <FileText size={14} />
          </button>
          {isNetease &&
            (clientFirst ? (
              // 默认走客户端时，这里是"反悔去网页版"的出口
              <button
                className="icon-button card-browser-button"
                type="button"
                onClick={() => onOpen(item.sourceUrl)}
                title="在浏览器打开网页版"
              >
                <ExternalLink size={14} />
              </button>
            ) : (
              // 偏好改成浏览器之后，这里反过来提供"去客户端"的出口，
              // 否则用户改了设置就等于永久失去客户端入口。
              onOpenInClient && (
                <button
                  className="icon-button card-browser-button"
                  type="button"
                  onClick={() => onOpenInClient(item)}
                  title="在网易云客户端打开并播放"
                >
                  <AppWindow size={14} />
                </button>
              )
            ))}
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
