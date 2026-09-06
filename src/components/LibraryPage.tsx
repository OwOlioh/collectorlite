import { useCallback, useEffect, useState } from "react";
import {
  Code2,
  Download,
  Github,
  Globe,
  LayoutGrid,
  List,
  Search,
  Tags,
  Trash2
} from "lucide-react";
import { api } from "../lib/api";
import { getRetentionDays } from "../lib/retention";
import type { ItemFilters, ObsidianSettings, Tag, VideoItem } from "../types";
import { TagBadge } from "./TagBadge";
import { TagManagerPanel } from "./TagManagerPanel";
import { TagPoolInput } from "./TagPoolInput";
import { VideoNoteEditorModal } from "./VideoNoteEditorModal";
import { VideoTagEditorModal } from "./VideoTagEditorModal";
import { VirtuosoGrid } from "react-virtuoso";
import { VideoCard } from "./VideoCard";
import { useToast } from "./Toast";
import { BatchTagEditorModal } from "./BatchTagEditorModal";

type LibrarySection = "search" | "manage";

interface LibraryPageProps {
  tags: Tag[];
  onTagsChanged: () => void;
  onTrashChanged: () => void;
  /** 数值变化即重新拉列表。浏览器扩展入库后由 App 递增。 */
  refreshToken?: number;
}

const initialFilters: ItemFilters = {
  query: "",
  tagIds: [],
  tagMode: "and",
  strict: false,
  untagged: false,
  sort: "favorite_desc",
  sources: []
};

function BilibiliIcon({ size = 15 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d="M17.813 3.5H6.187A3.187 3.187 0 0 0 3 6.687v8.626A3.187 3.187 0 0 0 6.187 18.5h1.889l-1.062 2.125h1.555l1.062-2.125h4.738l1.062 2.125h1.555l-1.062-2.125h1.889A3.187 3.187 0 0 0 21 15.313V6.687A3.187 3.187 0 0 0 17.813 3.5zm-9.338 8.594a.703.703 0 0 1 0 1.406H7.172a.703.703 0 0 1 0-1.406h1.303zm1.406 0h1.406a.703.703 0 0 1 0 1.406H9.881a.703.703 0 0 1 0-1.406zm2.813 0h1.406a.703.703 0 0 1 0 1.406h-1.406a.703.703 0 0 1 0-1.406zm2.813 0h1.303a.703.703 0 0 1 0 1.406H15.507a.703.703 0 0 1 0-1.406z" />
    </svg>
  );
}

export function LibraryPage({
  tags,
  onTagsChanged,
  onTrashChanged,
  refreshToken
}: LibraryPageProps) {
  const [section, setSection] = useState<LibrarySection>("search");
  const [filters, setFilters] = useState<ItemFilters>(initialFilters);
  const [items, setItems] = useState<VideoItem[]>([]);
  const [view, setView] = useState<"grid" | "list">("grid");
  const [loading, setLoading] = useState(true);
  const [editingVideo, setEditingVideo] = useState<VideoItem | null>(null);
  const [noteVideo, setNoteVideo] = useState<VideoItem | null>(null);
  const [selectedIds, setSelectedIds] = useState<number[]>([]);
  const [deleting, setDeleting] = useState(false);
  const { toast } = useToast();
  const [batchTagging, setBatchTagging] = useState(false);
  const [obsidianEnabled, setObsidianEnabled] = useState(false);

  // 依赖 refreshToken：设置页开启/配置 Obsidian 联动后（App 递增 libraryVersion），
  // 这里会重新拉取开关状态，否则导出按钮会停留在旧状态、迟迟不出现。
  useEffect(() => {
    void api
      .getObsidianSettings()
      .then((s: ObsidianSettings) => setObsidianEnabled(s.enabled))
      .catch(() => setObsidianEnabled(false));
  }, [refreshToken]);

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
  }, [loadItems, section, refreshToken]);

  const selectedFilterTags = tags.filter((tag) => filters.tagIds.includes(tag.id));

  // 单条导出已整合进批注弹窗（VideoNoteEditorModal），这里只保留批量导出
  const exportSelectedToObsidian = async () => {
    if (selectedIds.length === 0) return;
    try {
      const n = await api.exportItemsToObsidian(selectedIds);
      toast("success", `已导出 ${n} 条到 Obsidian`);
    } catch (e) {
      toast("error", `批量导出失败: ${String(e)}`);
    }
  };

  useEffect(() => {
    setSelectedIds([]);
  }, [filters]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const typing =
        !!target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable);
      if (typing) return;
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "a") {
        event.preventDefault();
        setSelectedIds(items.map((item) => item.id));
      } else if (event.key === "Escape") {
        setSelectedIds([]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [items]);

  const createFilterTag = async (name: string) => {
    const tag = await api.upsertTag({ namespace: "manual", name });
    onTagsChanged();
    setFilters((current) => ({
      ...current,
      tagIds: [...current.tagIds, tag.id],
      untagged: false
    }));
    return tag;
  };

  const deleteVideo = async (item: VideoItem) => {
    if (!window.confirm(`将本地收藏"${item.title}"移入回收站吗？${getRetentionDays()} 天内可恢复。`)) {
      return;
    }
    try {
      await api.deleteItem(item.id);
      setItems((current) => current.filter((video) => video.id !== item.id));
      onTagsChanged();
      onTrashChanged();
      toast("success", `已移入回收站（${getRetentionDays()} 天内可恢复）`, {
        action: {
          label: "撤销",
          onClick: async () => {
            try {
              await api.restoreItem(item.id);
              setItems((current) => [item, ...current]);
              onTrashChanged();
            } catch (error) {
              toast("error", `恢复失败：${String(error)}`);
            }
          }
        }
      });
    } catch (error) {
      toast("error", `删除失败：${String(error)}`);
    }
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
    if (!window.confirm(`将选中的 ${selectedIds.length} 条本地收藏移入回收站吗？`)) return;
    setDeleting(true);
    try {
      await api.deleteItems(selectedIds);
      setItems((current) =>
        current.filter((item) => !selectedIds.includes(item.id))
      );
      setSelectedIds([]);
      onTagsChanged();
      onTrashChanged();
      toast("success", `已移入回收站 ${selectedIds.length} 条收藏（${getRetentionDays()} 天内可恢复）`);
    } catch (error) {
      toast("error", `删除失败：${String(error)}`);
    } finally {
      setDeleting(false);
    }
  };

  const deleteVideosByTag = async (tag: Tag) => {
    if (!window.confirm(`将标签“${tag.name}”下的本地收藏移入回收站吗？该操作不会删除标签本身。`)) {
      return;
    }
    setDeleting(true);
    try {
      await api.deleteItemsByTag(tag.id);
      setSelectedIds([]);
      await loadItems();
      onTagsChanged();
      onTrashChanged();
      toast("success", `已移入回收站（${getRetentionDays()} 天内可恢复）`);
    } catch (error) {
      toast("error", `删除失败：${String(error)}`);
    } finally {
      setDeleting(false);
    }
  };

  const exportSelected = async () => {
    if (selectedIds.length === 0) return;
    try {
      const json = await api.exportCollection(selectedIds);
      const suggested = `collection-export-${selectedIds.length}-${new Date().toISOString().slice(0, 10)}.json`;
      const savedPath = await api.saveExportFile(json, suggested);
      toast("success", `已导出 ${selectedIds.length} 条收藏到：${savedPath}`);
    } catch (error) {
      const message = String(error);
      if (message.includes("取消保存")) {
        toast("info", "已取消导出");
      } else {
        toast("error", `导出失败：${message}`);
      }
    }
  };

  const exportAll = async () => {
    try {
      const json = await api.exportCollection();
      const suggested = `collection-export-all-${new Date().toISOString().slice(0, 10)}.json`;
      const savedPath = await api.saveExportFile(json, suggested);
      toast("success", `已导出全部收藏到：${savedPath}`);
    } catch (error) {
      const message = String(error);
      if (message.includes("取消保存")) {
        toast("info", "已取消导出");
      } else {
        toast("error", `导出失败：${message}`);
      }
    }
  };

  const saveBatchTags = async (addedTags: Tag[]) => {
    const targets = items.filter((item) => selectedIds.includes(item.id));
    if (targets.length === 0) return;
    try {
      for (const item of targets) {
        const merged = mergeTags(item.tags, addedTags);
        await api.updateItemTags(
          item.id,
          merged.map((tag) => ({
            id: tag.id,
            namespace: tag.namespace,
            name: tag.name,
            color: tag.color
          }))
        );
      }
      onTagsChanged();
      setSelectedIds([]);
      setBatchTagging(false);
      toast("success", `已为 ${targets.length} 条收藏更新标签`);
    } catch (error) {
      toast("error", `批量打标签失败：${String(error)}`);
    }
  };

  return (
    <section className="page library-page">
      <header className="page-header">
        <div>
          <h1>收藏库</h1>
          <p>检索收藏内容，或维护标签体系。</p>
        </div>
        <div className="page-header-right">
          <div className="source-filter" role="group" aria-label="来源筛选">
            <button
              type="button"
              className={filters.sources.includes("bilibili") ? "is-active" : ""}
              onClick={() =>
                setFilters((current) => ({
                  ...current,
                  sources: current.sources.includes("bilibili")
                    ? current.sources.filter((s) => s !== "bilibili")
                    : [...current.sources, "bilibili"]
                }))
              }
              title="B站视频"
            >
              <BilibiliIcon size={15} />
            </button>
            <button
              type="button"
              className={filters.sources.includes("browser") ? "is-active" : ""}
              onClick={() =>
                setFilters((current) => ({
                  ...current,
                  sources: current.sources.includes("browser")
                    ? current.sources.filter((s) => s !== "browser")
                    : [...current.sources, "browser"]
                }))
              }
              title="浏览器书签"
            >
              <Globe size={15} />
            </button>
            <button
              type="button"
              className={filters.sources.includes("zhihu") ? "is-active" : ""}
              onClick={() =>
                setFilters((current) => ({
                  ...current,
                  sources: current.sources.includes("zhihu")
                    ? current.sources.filter((s) => s !== "zhihu")
                    : [...current.sources, "zhihu"]
                }))
              }
              title="知乎收藏"
            >
              <span style={{fontSize: "13px", fontWeight: 700}}>知</span>
            </button>
            <button
              type="button"
              className={filters.sources.includes("csdn") ? "is-active" : ""}
              onClick={() =>
                setFilters((current) => ({
                  ...current,
                  sources: current.sources.includes("csdn")
                    ? current.sources.filter((s) => s !== "csdn")
                    : [...current.sources, "csdn"]
                }))
              }
              title="CSDN 收藏"
            >
              <Code2 size={15} />
            </button>
            <button
              type="button"
              className={filters.sources.includes("github") ? "is-active" : ""}
              onClick={() =>
                setFilters((current) => ({
                  ...current,
                  sources: current.sources.includes("github")
                    ? current.sources.filter((s) => s !== "github")
                    : [...current.sources, "github"]
                }))
              }
              title="GitHub Stars"
            >
              <Github size={15} />
            </button>
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
                      : [...current.tagIds, tag.id],
                    untagged: false
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
              <label
                className={`untagged-toggle${
                  filters.tagIds.length === 0 ? "" : " is-disabled"
                }`}
                title={
                  filters.tagIds.length === 0
                    ? "仅显示未挂任何标签的收藏"
                    : "先清空标签筛选后可用"
                }
              >
                <input
                  type="checkbox"
                  checked={filters.untagged === true}
                  disabled={filters.tagIds.length > 0}
                  onChange={(event) => {
                    const checked = event.target.checked;
                    setFilters((current) => ({
                      ...current,
                      untagged: checked,
                      // 与标签筛选互斥：勾选无标签时清空已选标签并关闭严格匹配
                      tagIds: checked ? [] : current.tagIds,
                      strict: checked ? false : current.strict
                    }));
                  }}
                />
                无标签
              </label>
              <label
                className={`strict-match-toggle${
                  filters.tagIds.length === 0 ? " is-disabled" : ""
                }`}
                title={
                  filters.tagIds.length === 0
                    ? "先选择标签后可用"
                    : "仅匹配恰好含有所选标签的内容"
                }
              >
                <input
                  type="checkbox"
                  checked={filters.strict === true}
                  disabled={filters.tagIds.length === 0}
                  onChange={(event) =>
                    setFilters((current) => ({ ...current, strict: event.target.checked }))
                  }
                />
                严格匹配
              </label>
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

          </div>

          {loading ? (
            <div className="empty-state">正在读取收藏库...</div>
          ) : items.length === 0 ? (
            <div className="empty-state">
              <h2>没有匹配的视频</h2>
              <p>调整检索条件，或到导入页添加收藏。</p>
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
                  <span>全选当前结果（{items.length}）</span>
                </label>
                <button
                  className="secondary-button"
                  type="button"
                  onClick={exportAll}
                >
                  <Download size={16} />
                  导出全部
                </button>
                {selectedIds.length > 0 && (
                  <>
                    <button
                      className="secondary-button"
                      type="button"
                      onClick={() => setBatchTagging(true)}
                    >
                      <Tags size={16} />
                      批量打标签（{selectedIds.length}）
                    </button>
                    <button
                      className="secondary-button"
                      type="button"
                      onClick={exportSelected}
                    >
                      <Download size={16} />
                      导出（{selectedIds.length}）
                    </button>
                    {obsidianEnabled && (
                      <button
                        className="secondary-button"
                        type="button"
                        onClick={exportSelectedToObsidian}
                      >
                        <Code2 size={16} />
                        导出到 Obsidian（{selectedIds.length}）
                      </button>
                    )}
                    <button
                      className="secondary-button danger-action"
                      type="button"
                      onClick={deleteSelected}
                      disabled={deleting}
                    >
                      <Trash2 size={16} />
                      删除选中（{selectedIds.length}）
                    </button>
                  </>
                )}
              </div>

              <div className="library-list-wrap">
                <VirtuosoGrid
                  data={items}
                  style={{ height: "100%" }}
                  className="video-list-region"
                  listClassName={`video-grid ${view === "list" ? "is-list" : ""}`}
                  itemClassName="video-grid-cell"
                  overscan={400}
                  itemContent={(_index, item) => (
                    <VideoCard
                      item={item}
                      isSelected={selectedIds.includes(item.id)}
                      onToggleSelect={toggleSelected}
                      onOpen={(url) => api.openUrl(url)}
                      onEditTags={setEditingVideo}
                      onEditNote={setNoteVideo}
                      onDelete={deleteVideo}
                    />
                  )}
                />
              </div>
            </>
          )}
        </>
      )}

      {batchTagging && (
        <BatchTagEditorModal
          count={selectedIds.length}
          tagPool={tags}
          onClose={() => setBatchTagging(false)}
          onSave={saveBatchTags}
          onTagsChanged={onTagsChanged}
        />
      )}

      {editingVideo && (
        <VideoTagEditorModal
          item={editingVideo}
          tagPool={tags}
          onClose={() => setEditingVideo(null)}
          onSaved={() => { loadItems(); toast("success", "标签已保存"); }}
          onTagsChanged={onTagsChanged}
        />
      )}

      {noteVideo && (
        <VideoNoteEditorModal
          item={noteVideo}
          onClose={() => setNoteVideo(null)}
          onSaved={() => { loadItems(); toast("success", "批注已保存"); }}
          onExported={() => { loadItems(); }}
        />
      )}
    </section>
  );
}

function mergeTags(current: Tag[], additions: Tag[]): Tag[] {
  const map = new Map<number, Tag>();
  current.forEach((tag) => map.set(tag.id, tag));
  additions.forEach((tag) => {
    if (!map.has(tag.id)) map.set(tag.id, tag);
  });
  return [...map.values()];
}
