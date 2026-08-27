import { useCallback, useEffect, useState } from "react";
import {
  FileText,
  LayoutGrid,
  List,
  Pencil,
  Search,
  Tags,
  Trash2
} from "lucide-react";
import { api, resolveCoverUrl } from "../lib/api";
import type { ItemFilters, Tag, VideoItem } from "../types";
import { TagBadge } from "./TagBadge";
import { TagManagerPanel } from "./TagManagerPanel";
import { TagPoolInput } from "./TagPoolInput";
import { VideoNoteEditorModal } from "./VideoNoteEditorModal";
import { VideoTagEditorModal } from "./VideoTagEditorModal";

type LibrarySection = "search" | "manage";

interface LibraryPageProps {
  tags: Tag[];
  onTagsChanged: () => void;
}

const initialFilters: ItemFilters = {
  query: "",
  tagIds: [],
  tagMode: "and",
  sort: "favorite_desc"
};

function formatDuration(seconds?: number) {
  if (!seconds) return "未知";
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${mins}:${String(secs).padStart(2, "0")}`;
}

function formatDate(timestamp?: number) {
  if (!timestamp) return "未知";
  return new Date(timestamp * 1000).toLocaleDateString("zh-CN");
}

export function LibraryPage({ tags, onTagsChanged }: LibraryPageProps) {
  const [section, setSection] = useState<LibrarySection>("search");
  const [filters, setFilters] = useState<ItemFilters>(initialFilters);
  const [items, setItems] = useState<VideoItem[]>([]);
  const [view, setView] = useState<"grid" | "list">("grid");
  const [loading, setLoading] = useState(true);
  const [editingVideo, setEditingVideo] = useState<VideoItem | null>(null);
  const [noteVideo, setNoteVideo] = useState<VideoItem | null>(null);
  const [selectedIds, setSelectedIds] = useState<number[]>([]);
  const [deleting, setDeleting] = useState(false);

  const loadItems = useCallback(async () => {
    setLoading(true);
    try {
      setItems(await api.listItems(filters));
    } finally {
      setLoading(false);
    }
  }, [filters]);

  useEffect(() => {
    if (section !== "search") return;
    const timer = window.setTimeout(loadItems, 120);
    return () => window.clearTimeout(timer);
  }, [loadItems, section]);

  const selectedFilterTags = tags.filter((tag) => filters.tagIds.includes(tag.id));

  useEffect(() => {
    setSelectedIds([]);
  }, [filters]);

  const createFilterTag = async (name: string) => {
    const tag = await api.upsertTag({ namespace: "manual", name });
    onTagsChanged();
    setFilters((current) => ({
      ...current,
      tagIds: [...current.tagIds, tag.id]
    }));
    return tag;
  };

  const deleteVideo = async (item: VideoItem) => {
    if (!window.confirm(`删除本地视频“${item.title}”吗？该操作不会影响 B 站原收藏夹。`)) {
      return;
    }
    await api.deleteItem(item.id);
    setItems((current) => current.filter((video) => video.id !== item.id));
    onTagsChanged();
  };

  const toggleSelected = (itemId: number) => {
    setSelectedIds((current) =>
      current.includes(itemId)
        ? current.filter((id) => id !== itemId)
        : [...current, itemId]
    );
  };

  const allSelected = items.length > 0 && items.every((item) => selectedIds.includes(item.id));

  const toggleSelectAll = () => {
    setSelectedIds(allSelected ? [] : items.map((item) => item.id));
  };

  const deleteSelected = async () => {
    if (selectedIds.length === 0) return;
    if (!window.confirm(`删除选中的 ${selectedIds.length} 条本地视频吗？`)) return;
    setDeleting(true);
    try {
      await api.deleteItems(selectedIds);
      setItems((current) =>
        current.filter((item) => !selectedIds.includes(item.id))
      );
      setSelectedIds([]);
      onTagsChanged();
    } catch (error) {
      window.alert(String(error));
    } finally {
      setDeleting(false);
    }
  };

  const deleteVideosByTag = async (tag: Tag) => {
    if (!window.confirm(`删除标签“${tag.name}”下的本地视频吗？该操作不会删除标签本身。`)) {
      return;
    }
    setDeleting(true);
    try {
      await api.deleteItemsByTag(tag.id);
      setSelectedIds([]);
      await loadItems();
      onTagsChanged();
    } catch (error) {
      window.alert(String(error));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <section className="page library-page">
      <header className="page-header">
        <div>
          <h1>视频库</h1>
          <p>检索收藏内容，或维护标签体系。</p>
        </div>
        <div className="view-toggle" role="group" aria-label="视图切换">
          <button
            type="button"
            className={view === "grid" ? "is-active" : ""}
            onClick={() => setView("grid")}
            title="网格视图"
          >
            <LayoutGrid size={17} />
          </button>
          <button
            type="button"
            className={view === "list" ? "is-active" : ""}
            onClick={() => setView("list")}
            title="列表视图"
          >
            <List size={17} />
          </button>
        </div>
      </header>

      <div className="library-section-tabs">
        <button
          type="button"
          className={section === "search" ? "is-active" : ""}
          onClick={() => setSection("search")}
        >
          <Search size={16} />
          检索视频
        </button>
        <button
          type="button"
          className={section === "manage" ? "is-active" : ""}
          onClick={() => setSection("manage")}
        >
          <Tags size={16} />
          管理标签
        </button>
      </div>

      {section === "manage" ? (
        <TagManagerPanel tags={tags} onTagsChanged={onTagsChanged} />
      ) : (
        <>
          <div className="unified-filter">
            <label className="search-box">
              <Search size={17} />
              <input
                value={filters.query}
                onChange={(event) =>
                  setFilters((current) => ({ ...current, query: event.target.value }))
                }
                placeholder="输入文本检索标题、简介或 UP 主名称"
              />
            </label>

            <div className="tag-filter-line">
              <TagPoolInput
                pool={tags}
                selected={selectedFilterTags}
                onAdd={(tag) =>
                  setFilters((current) => ({
                    ...current,
                    tagIds: current.tagIds.includes(tag.id)
                      ? current.tagIds
                      : [...current.tagIds, tag.id]
                  }))
                }
                onRemove={(tag) =>
                  setFilters((current) => ({
                    ...current,
                    tagIds: current.tagIds.filter((id) => id !== tag.id)
                  }))
                }
                onCreate={createFilterTag}
                placeholder="输入标签名称进行检索筛选"
              />
            </div>

            {selectedFilterTags.length === 1 && (
              <button
                className="secondary-button danger-action"
                type="button"
                onClick={() => deleteVideosByTag(selectedFilterTags[0])}
                disabled={deleting}
              >
                <Trash2 size={16} />
                删除该标签下的本地视频
              </button>
            )}

            <select
              className="select-control"
              value={filters.sort}
              onChange={(event) =>
                setFilters((current) => ({
                  ...current,
                  sort: event.target.value as ItemFilters["sort"]
                }))
              }
            >
              <option value="favorite_desc">收藏时间：最近优先</option>
              <option value="published_desc">发布时间：最近优先</option>
              <option value="duration_desc">时长：最长优先</option>
              <option value="title_asc">标题：字母升序</option>
              <option value="imported_desc">入库时间：最近优先</option>
            </select>
          </div>

          {loading ? (
            <div className="empty-state">正在读取视频库...</div>
          ) : items.length === 0 ? (
            <div className="empty-state">
              <h2>没有匹配的视频</h2>
              <p>调整检索条件，或到导入页添加 B 站收藏。</p>
            </div>
          ) : (
            <>
              <div className="selection-toolbar">
                <label className="select-all-line">
                  <input
                    type="checkbox"
                    checked={allSelected}
                    onChange={toggleSelectAll}
                  />
                  <span>全选当前结果</span>
                </label>
                {selectedIds.length > 0 && (
                  <button
                    className="secondary-button danger-action"
                    type="button"
                    onClick={deleteSelected}
                    disabled={deleting}
                  >
                    <Trash2 size={16} />
                    删除选中（{selectedIds.length}）
                  </button>
                )}
              </div>

              <div className={`video-grid ${view === "list" ? "is-list" : ""}`}>
                {items.map((item) => (
                  <article className="video-card" key={item.id}>
                    <label
                      className={`video-select-checkbox ${
                        selectedIds.includes(item.id) ? "is-checked" : ""
                      }`}
                      title="选择视频"
                    >
                      <input
                        type="checkbox"
                        checked={selectedIds.includes(item.id)}
                        onChange={() => toggleSelected(item.id)}
                      />
                    </label>
                    <button
                      className="video-cover-button"
                      type="button"
                      onClick={() => api.openUrl(item.sourceUrl)}
                      title="在浏览器打开"
                    >
                      {resolveCoverUrl(item.coverUrl, item.coverLocalPath) ? (
                        <img
                          src={resolveCoverUrl(item.coverUrl, item.coverLocalPath)}
                          alt=""
                          loading="lazy"
                        />
                      ) : (
                        <div className="cover-placeholder">无封面</div>
                      )}
                      <span className="duration">{formatDuration(item.duration)}</span>
                    </button>
                    <div className="video-card-body">
                      <button
                        type="button"
                        className="video-title"
                        onClick={() => api.openUrl(item.sourceUrl)}
                      >
                        {item.title}
                      </button>
                      <div className="video-meta">
                        <span>{item.authorName || "未知作者"}</span>
                        {item.partitionName && <span>{item.partitionName}</span>}
                        <span>{formatDate(item.favoriteTime || item.publishedAt)}</span>
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
                        onClick={() => setNoteVideo(item)}
                        title="编辑视频批注"
                      >
                        <FileText size={14} />
                      </button>
                      <button
                        className="icon-button danger card-delete-button"
                          type="button"
                          onClick={() => deleteVideo(item)}
                          title="删除本地视频"
                        >
                          <Trash2 size={14} />
                        </button>
                        <button
                          className="icon-button card-edit-button"
                          type="button"
                          onClick={() => setEditingVideo(item)}
                          title="编辑视频标签"
                        >
                          <Pencil size={14} />
                        </button>
                      </div>
                    </div>
                  </article>
                ))}
              </div>
            </>
          )}
        </>
      )}

      {editingVideo && (
        <VideoTagEditorModal
          item={editingVideo}
          tagPool={tags}
          onClose={() => setEditingVideo(null)}
          onSaved={loadItems}
          onTagsChanged={onTagsChanged}
        />
      )}

      {noteVideo && (
        <VideoNoteEditorModal
          item={noteVideo}
          onClose={() => setNoteVideo(null)}
          onSaved={loadItems}
        />
      )}
    </section>
  );
}
