import { useCallback, useEffect, useRef, useState } from "react";
import {
  Code2,
  Download,
  Github,
  Globe,
  LayoutGrid,
  List,
  Music,
  Search,
  Sparkles,
  Tags,
  Trash2
} from "lucide-react";
import { api, inTauri } from "../lib/api";
import { getRetentionDays } from "../lib/retention";
import type { ItemFilters, OpenTarget, SmartTagCandidate, SmartTagMatch, Tag, VideoItem } from "../types";
import { TagBadge } from "./TagBadge";
import { TagManagerPanel } from "./TagManagerPanel";
import { TagPoolInput } from "./TagPoolInput";
import { VideoNoteEditorModal } from "./VideoNoteEditorModal";
import { VideoTagEditorModal } from "./VideoTagEditorModal";
import { VirtuosoGrid } from "react-virtuoso";
import { VideoCard } from "./VideoCard";
import { useToast } from "./Toast";
import { BatchTagEditorModal } from "./BatchTagEditorModal";
import { SmartTagModal } from "./SmartTagModal";

type LibrarySection = "search" | "manage";

interface LibraryPageProps {
  tags: Tag[];
  onTagsChanged: () => void;
  onTrashChanged: () => void;
  /** 数值变化即重新拉列表。浏览器扩展入库后由 App 递增。 */
  refreshToken?: number;
  /** 当前是否显示本视图（App 按 active view 传入）。从其他页切回时自动静默刷新，
   *  让导入/扩展入库的新内容无需手动刷新即可出现。 */
  isActive?: boolean;
  /** 打开方式偏好变更（设置页改了客户端/浏览器）后由 App 递增，触发重读。 */
  openPrefsVersion?: number;
  /** 统计页下钻意图：非 null 时应用一次来源/标签筛选并切到检索区。
   *  `source` 直接对应 filters.sources；`tagId` 对应 filters.tagIds。 */
  drill?: { source?: string; tagId?: number } | null;
  /** 应用完下钻筛选后回调，让 App 清空 drill（否则相同来源/标签的后续点击被旧状态吞掉）。 */
  onDrillConsumed?: () => void;
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
  refreshToken,
  isActive = true,
  openPrefsVersion,
  drill,
  onDrillConsumed
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
  const [smartTagging, setSmartTagging] = useState(false);

  // 各来源的「客户端 / 浏览器」打开偏好。读失败就留空 —— 空会落到客户端优先，
  // 也就是默认值，用户的卡片不会因此变成打不开。
  const [openTargets, setOpenTargets] = useState<Record<string, OpenTarget>>({});
  useEffect(() => {
    void api
      .getOpenPrefs()
      .then((p) => setOpenTargets(p.targets ?? {}))
      .catch(() => setOpenTargets({}));
  }, [refreshToken, openPrefsVersion]);

  const loadItems = useCallback(async () => {
    setLoading(true);
    try {
      setItems(await api.listItems(filters));
    } finally {
      setLoading(false);
    }
  }, [filters]);

  // 保存标签/批注等「原地修改」后的静默刷新：不切 loading 态、不卸载列表，
  // VirtuosoGrid 保持挂载与 scrollTop，避免编辑长列表中间的内容后跳回顶部。
  const reloadSilently = useCallback(async () => {
    try {
      setItems(await api.listItems(filters));
    } catch {
      /* 静默失败：列表保持现状，下次常规刷新兜底 */
    }
  }, [filters]);

  // 检索条件 / 内部分区（检索↔管理标签）变化 → 带 loading 常规加载
  useEffect(() => {
    if (section !== "search") return;
    const timer = window.setTimeout(loadItems, 120);
    return () => window.clearTimeout(timer);
  }, [loadItems, section]);

  // 外部版本号变化（浏览器扩展入库，App 递增 refreshToken）→ 静默刷新，不切 loading、不跳顶
  const firstTokenRef = useRef(true);
  useEffect(() => {
    if (firstTokenRef.current) {
      firstTokenRef.current = false;
      return;
    }
    if (section !== "search") return;
    void reloadSilently();
  }, [refreshToken, section, reloadSilently]);

  // 从其他页面切回收藏库视图时 → 自动静默刷新，新导入/入库内容即时可见
  const wasActiveRef = useRef(isActive);
  useEffect(() => {
    if (isActive && !wasActiveRef.current && section === "search") {
      void reloadSilently();
    }
    wasActiveRef.current = isActive;
  }, [isActive, section, reloadSilently]);

  // 统计页下钻：收到非零 drill 时，切换到检索区并应用一次来源/标签筛选，随后回调清空。
  // 用全新的 filters 对象（不残留旧 query/严格匹配），让下钻结果干净可预期。
  useEffect(() => {
    if (!drill) return;
    setSection("search");
    setFilters({
      ...initialFilters,
      sources: drill.source ? [drill.source] : [],
      tagIds: drill.tagId != null ? [drill.tagId] : []
    });
    onDrillConsumed?.();
  }, [drill, onDrillConsumed]);

  const selectedFilterTags = tags.filter((tag) => filters.tagIds.includes(tag.id));

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

  // 打星 / 取消星标：后端更新后静默重拉，让星标项立即置顶（或取消后回到原位）
  const toggleStar = async (video: VideoItem) => {
    try {
      await api.setItemStar(video.id, !(video.starred === true));
      await reloadSilently();
    } catch (error) {
      toast("error", `星标操作失败：${String(error)}`);
    }
  };

  // 网易云「客户端优先」：优先唤起桌面客户端并播放；后端在客户端不可用时已自动回退浏览器，
  // 这里只是把回退如实告诉用户（DEVELOPMENT.md 9.11-1）。
  const openInNeteaseClient = async (video: VideoItem) => {
    // 浏览器预览模式（npm run dev 单起前端）下 `open_in_netease` 走的是 mock，
    // 恒返回 false。若不加区分地提示"未检测到客户端"，会让人误以为深链不可用作罢 ——
    // 实测 `orpheus://` 协议是好的，只是这条 toast 把人带偏了。
    if (!inTauri()) {
      toast("info", "浏览器预览模式无法唤起客户端，请用 cargo run 启动桌面端");
      window.open(video.sourceUrl, "_blank", "noopener,noreferrer");
      return;
    }
    // 唤起要等 Windows 走一遍 Shell（App 首次调用实测 ~310 ms，之后降到 ~10 ms）。
    // 这段时间 UI 不该是「点了没反应」的样子 —— 先给一句即时反馈把空窗填掉，
    // 这是本次优化里唯一真正作用于「卡顿感」的部分：延迟测不出 improvement，
    // 但主观上的空等没了。
    // 客户端窗口弹出来之后它自己会在 1.2 s 内消失，不需要也没法手动撤。
    toast("info", "正在唤起网易云客户端...", { duration: 1200 });
    try {
      const usedClient = await api.openInNetease(video.externalId, video.sourceUrl);
      if (!usedClient) {
        toast("info", "未检测到网易云客户端，已在浏览器打开");
      }
    } catch (error) {
      const message = String(error);
      // 命令不存在 = Rust 侧没注册，99% 是改了后端没重编译（本项目经典坑：无 HMR）
      if (message.includes("command") && message.includes("not found")) {
        toast("error", "后端未包含该命令，请重新编译 Rust 后再试");
      } else {
        toast("error", `打开失败：${message}`);
      }
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
      await reloadSilently();
      toast("success", `已为 ${targets.length} 条收藏更新标签`);
    } catch (error) {
      toast("error", `批量打标签失败：${String(error)}`);
    }
  };

  // 从选中项中移除其共有的标签（用户勾选的若干个）。复用 updateItemTags，
  // 把每条收藏的标签集合减去待删集合后整体回写即可，无需新增后端命令。
  const removeBatchTags = async (tagIds: number[]) => {
    if (tagIds.length === 0) return;
    const targets = items.filter((item) => selectedIds.includes(item.id));
    if (targets.length === 0) return;
    try {
      for (const item of targets) {
        const remaining = item.tags.filter((tag) => !tagIds.includes(tag.id));
        await api.updateItemTags(
          item.id,
          remaining.map((tag) => ({
            id: tag.id,
            namespace: tag.namespace,
            name: tag.name,
            color: tag.color
          }))
        );
      }
      onTagsChanged();
      await reloadSilently();
      toast("success", `已从 ${targets.length} 条收藏移除 ${tagIds.length} 个标签`);
    } catch (error) {
      toast("error", `批量删除标签失败：${String(error)}`);
    }
  };

  // 智能匹配打标签：把候选标签加到各自命中的收藏上（命中的项里已挂载的跳过，保持幂等）。
  // 智能匹配：按「内容 → 标签」视角逐条应用。只对单条收藏打上用户确认的标签。
  // 不在此处 toast —— 失败由弹窗捕获并提示，避免双层提示。
  const applySmartTagsToItem = async (itemId: number, tagIds: number[]) => {
    if (tagIds.length === 0) return;
    const item = items.find((it) => it.id === itemId);
    if (!item) return;
    const toAdd = tags.filter(
      (t) => tagIds.includes(t.id) && !item.tags.some((e) => e.id === t.id)
    );
    if (toAdd.length === 0) return;
    const merged = mergeTags(item.tags, toAdd);
    await api.updateItemTags(
      item.id,
      merged.map((t) => ({ id: t.id, namespace: t.namespace, name: t.name, color: t.color }))
    );
    onTagsChanged();
    await reloadSilently();
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
              className={filters.sources.includes("netease") ? "is-active" : ""}
              onClick={() =>
                setFilters((current) => ({
                  ...current,
                  sources: current.sources.includes("netease")
                    ? current.sources.filter((s) => s !== "netease")
                    : [...current.sources, "netease"]
                }))
              }
              title="网易云音乐"
            >
              <Music size={15} />
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
                      批量更改标签（{selectedIds.length}）
                    </button>
                    <button
                      className="secondary-button"
                      type="button"
                      onClick={() => setSmartTagging(true)}
                    >
                      <Sparkles size={16} />
                      智能标签（{selectedIds.length}）
                    </button>
                    <button
                      className="secondary-button"
                      type="button"
                      onClick={exportSelected}
                    >
                      <Download size={16} />
                      导出（{selectedIds.length}）
                    </button>
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
                      onOpenInClient={openInNeteaseClient}
                      openTarget={openTargets[item.source]}
                      onEditTags={setEditingVideo}
                      onEditNote={setNoteVideo}
                      onDelete={deleteVideo}
                      onToggleStar={toggleStar}
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
          commonTags={commonTagsOf(items.filter((item) => selectedIds.includes(item.id)))}
          onClose={() => setBatchTagging(false)}
          onSave={saveBatchTags}
          onRemove={removeBatchTags}
          onTagsChanged={onTagsChanged}
        />
      )}

      {smartTagging && (
        <SmartTagModal
          count={selectedIds.length}
          tagPool={tags}
          smartCandidates={smartMatchCandidates(
            items.filter((item) => selectedIds.includes(item.id)),
            tags
          )}
          selectedItems={items
            .filter((item) => selectedIds.includes(item.id))
            .map((item) => ({ id: item.id, title: item.title }))}
          onClose={() => setSmartTagging(false)}
          onApplySmartItem={applySmartTagsToItem}
        />
      )}

      {editingVideo && (
        <VideoTagEditorModal
          item={editingVideo}
          tagPool={tags}
          onClose={() => setEditingVideo(null)}
          onSaved={() => { void reloadSilently(); toast("success", "标签已保存"); }}
          onTagsChanged={onTagsChanged}
        />
      )}

      {noteVideo && (
        <VideoNoteEditorModal
          item={noteVideo}
          onClose={() => setNoteVideo(null)}
          onSaved={() => { void reloadSilently(); }}
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

// 选中项「共有」的标签 = 所有选中收藏标签集合的交集（按 tag id 判定）。
// 任意一条没有标签，或彼此没有重合，交集即空 → 调用处展示「无共同标签」。
// 单条选中时交集就是它自身全部标签（语义上仍成立）。
function commonTagsOf(items: VideoItem[]): Tag[] {
  if (items.length === 0) return [];
  const idSets = items.map((item) => new Set(item.tags.map((tag) => tag.id)));
  const first = idSets[0];
  const commonIds = [...first].filter((id) => idSets.every((set) => set.has(id)));
  const byId = new Map<number, Tag>();
  items[0].tags.forEach((tag) => byId.set(tag.id, tag));
  return commonIds
    .map((id) => byId.get(id))
    .filter((tag): tag is Tag => tag !== undefined);
}

// 智能匹配标签：扫描选中项的来源文本，从已有标签库检索能匹配的项（不新建标签）。
// 返回每个候选标签 + 它命中的选中项 id；已挂载该标签的项不计入，保持幂等。
// 关键优化：一次性构建全部标签名的正则，每条文本只扫一遍（避免 S×T 朴素子串爆炸）。
function smartMatchCandidates(items: VideoItem[], tagPool: Tag[]): SmartTagCandidate[] {
  if (items.length === 0 || tagPool.length === 0) return [];
  const byId = new Map<number, Tag>(tagPool.map((t) => [t.id, t]));
  const nameToTag = new Map<string, Tag>();
  for (const t of tagPool) nameToTag.set(t.name.toLowerCase(), t);

  const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  // ASCII 标签加词边界（避免 "go" 误中 "google"）；含非 ASCII（如 CJK）直接子串。
  const pattern = new RegExp(
    tagPool
      .map((t) => (/^[A-Za-z0-9]+$/.test(t.normalized) ? `\\b${esc(t.name)}\\b` : esc(t.name)))
      .join("|"),
    "gi"
  );

  // 逐字段扫描，这样才能记录「在哪个字段、哪段文字命中」，供 UI 展开核对。
  const FIELDS: { key: keyof VideoItem; label: string }[] = [
    { key: "title", label: "标题" },
    { key: "description", label: "描述" },
    { key: "authorName", label: "作者" },
    { key: "partitionName", label: "分区" },
    { key: "source", label: "来源" }
  ];

  // 命中上下文窗口：命中词前后各取 RADIUS 个字符，超出则加 …
  const RADIUS = 36;
  const windowSnippet = (text: string, start: number, end: number) => {
    const s = Math.max(0, start - RADIUS);
    const e = Math.min(text.length, end + RADIUS);
    return {
      before: (s > 0 ? "…" : "") + text.slice(s, start),
      hit: text.slice(start, end),
      after: text.slice(end, e) + (e < text.length ? "…" : "")
    };
  };

  // 候选：tag.id -> 命中溯源列表
  const matchesByTag = new Map<number, SmartTagMatch[]>();
  for (const it of items) {
    const owned = new Set(it.tags.map((t) => t.id));
    const itemTitle = it.title ?? "";
    for (const f of FIELDS) {
      const raw = it[f.key];
      if (typeof raw !== "string" || raw.length === 0) continue;
      const lower = raw.toLowerCase();
      pattern.lastIndex = 0;
      let m: RegExpExecArray | null;
      while ((m = pattern.exec(lower)) !== null) {
        const tag = nameToTag.get(m[0].toLowerCase());
        if (tag && !owned.has(tag.id)) {
          // lower 与原文字符数一致（仅大小写变化），索引可直接用于原文切片。
          const start = m.index;
          const end = m.index + m[0].length;
          const { before, hit, after } = windowSnippet(raw, start, end);
          const list = matchesByTag.get(tag.id);
          const entry: SmartTagMatch = {
            itemId: it.id,
            itemTitle,
            field: f.key,
            fieldLabel: f.label,
            before,
            hit,
            after
          };
          if (list) list.push(entry);
          else matchesByTag.set(tag.id, [entry]);
        }
        if (m.index === pattern.lastIndex) pattern.lastIndex++; // 防零宽死循环
      }
    }
  }

  return [...matchesByTag.entries()]
    .map(([id, matches]) => ({
      tag: byId.get(id)!,
      matchedItemIds: [...new Set(matches.map((mt) => mt.itemId))],
      matches
    }))
    .filter((c) => c.tag !== undefined)
    .sort((a, b) => b.matchedItemIds.length - a.matchedItemIds.length);
}
