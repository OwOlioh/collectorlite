import { useEffect, useRef, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FolderPlus,
  GitMerge,
  GripVertical,
  Layers,
  Pencil,
  Save,
  Search,
  Trash2,
  X
} from "lucide-react";
import { api } from "../lib/api";
import type { Tag, TagCategory } from "../types";
import { TagBadge } from "./TagBadge";
import { TagPoolInput } from "./TagPoolInput";
import { useToast } from "./Toast";

const categoryColors = ["#64748b", "#0f766e", "#b45309", "#7c3aed", "#be123c"];
// 分类编辑调色板（与标签颜色选择一致，另支持原生取色器自定义任意颜色）
const categoryColorOptions = [
  "#3b82f6",
  "#ef4444",
  "#f97316",
  "#10b981",
  "#8b5cf6",
  "#0891b2",
  "#db2777",
  "#ca8a04"
];

interface TagManagerPanelProps {
  tags: Tag[];
  onTagsChanged?: () => void | Promise<void>;
}

export function TagManagerPanel({ tags, onTagsChanged }: TagManagerPanelProps) {
  const [categories, setCategories] = useState<TagCategory[]>([]);
  const { toast } = useToast();
  const [newCategory, setNewCategory] = useState("");
  const [expandedCategoryId, setExpandedCategoryId] = useState<number | null>(null);
  // 分类多选（Ctrl+左键）与右键合并菜单
  const [multiSelectedCategoryIds, setMultiSelectedCategoryIds] = useState<number[]>([]);
  const [categoryContextMenu, setCategoryContextMenu] = useState<{
    x: number;
    y: number;
  } | null>(null);
  const [draggedTagId, setDraggedTagId] = useState<number | null>(null);
  const [dragOverCategoryId, setDragOverCategoryId] = useState<number | null>(null);
  const [dragOverUncategorized, setDragOverUncategorized] = useState(false);
  const [draggedCategoryId, setDraggedCategoryId] = useState<number | null>(null);
  const [categorySortIndicator, setCategorySortIndicator] = useState<{
    id: number;
    before: boolean;
  } | null>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editingName, setEditingName] = useState("");
  const [mergeSource, setMergeSource] = useState<Tag | null>(null);
  const [mergeTargetName, setMergeTargetName] = useState("");
  const [merging, setMerging] = useState(false);
  const [renamingCategoryId, setRenamingCategoryId] = useState<number | null>(null);
  const [renamingCategoryName, setRenamingCategoryName] = useState("");
  const [renamingCategoryColor, setRenamingCategoryColor] = useState<string>(categoryColorOptions[0]);
  const [searchQuery, setSearchQuery] = useState("");
  const suppressCategoryClick = useRef(false);
  const draggedTagRef = useRef<number | null>(null);
  const activePointerIdRef = useRef<number | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const autoScrollFrameRef = useRef<number | null>(null);
  const autoScrollVelocityRef = useRef(0);
  const dragTargetRef = useRef<{ categoryId: number | null } | null>(null);
  const draggedCategoryIdRef = useRef<number | null>(null);
  const categoryPointerRef = useRef<{
    id: number;
    startX: number;
    startY: number;
    active: boolean;
  } | null>(null);
  const sortIndicatorRef = useRef<{ id: number; before: boolean } | null>(null);
  const attachedCategoryMoveRef = useRef<((event: PointerEvent) => void) | null>(null);
  const attachedCategoryUpRef = useRef<((event: PointerEvent) => void) | null>(null);
  const dragListenersAttachedRef = useRef(false);
  const attachedPointerMoveRef = useRef<((event: PointerEvent) => void) | null>(null);
  const attachedPointerUpRef = useRef<((event: PointerEvent) => void) | null>(null);
  const attachedPointerCancelRef = useRef<((event: PointerEvent) => void) | null>(null);

  const refreshCategories = async () => {
    setCategories(await api.listTagCategories());
  };

  useEffect(() => {
    void refreshCategories();
  }, []);

  const createTag = async (name: string) => {
    await api.upsertTag({ namespace: "manual", name });
    await onTagsChanged?.();
  };

  const addCategory = async () => {
    if (!newCategory.trim()) return;
    const name = newCategory.trim();
    const color = categoryColors[Math.abs(name.length) % categoryColors.length];
    try {
      await api.createTagCategory(name, color);
      setNewCategory("");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const detail = error instanceof Error ? "" : ` :: ${JSON.stringify(error)}`;
      if (message.includes("UNIQUE constraint") || message.includes("normalized")) {
        setNewCategory("");
        toast("info", `分类「${name}」已存在`);
      } else {
        // 即使报错也先刷新：后端可能已写入（如返回值反序列化失败等瞬态错误），
        // 权威刷新能保证新建分类在数据库里确实存在时被显示出来。
        toast("error", `新建分类失败：${message}${detail}`);
      }
    } finally {
      // 无论成功还是报错，都从后端拉取权威列表刷新，确保列表与数据库一致。
      await refreshCategories();
    }
  };

  const deleteCategory = async (category: TagCategory) => {
    if (!window.confirm(`删除分类“${category.name}”吗？其中标签会回到未分类。`)) return;
    await api.deleteTagCategory(category.id);
    setExpandedCategoryId((current) => (current === category.id ? null : current));
    await refreshCategories();
    await onTagsChanged?.();
  };

  const beginRenameCategory = (category: TagCategory) => {
    setRenamingCategoryId(category.id);
    setRenamingCategoryName(category.name);
    setRenamingCategoryColor(category.color ?? categoryColorOptions[0]);
  };

  const renameCategory = async (category: TagCategory) => {
    if (!renamingCategoryName.trim()) return;
    await api.renameTagCategory(
      category.id,
      renamingCategoryName.trim(),
      renamingCategoryColor
    );
    setRenamingCategoryId(null);
    setRenamingCategoryName("");
    await refreshCategories();
  };

  const assignCategory = async (tagId: number, categoryId: number | null) => {
    await api.assignTagCategory(tagId, categoryId);
    await onTagsChanged?.();
  };

  // ---- 分类多选（Ctrl+左键）与右键「合并为组」----
  const toggleCategoryMultiSelect = (categoryId: number) => {
    setMultiSelectedCategoryIds((current) =>
      current.includes(categoryId)
        ? current.filter((id) => id !== categoryId)
        : [...current, categoryId]
    );
  };

  const clearMultiSelect = () => setMultiSelectedCategoryIds([]);

  const openCategoryContextMenu = (
    category: TagCategory,
    event: React.MouseEvent<HTMLElement>
  ) => {
    event.preventDefault();
    event.stopPropagation();
    // 右键落在未选中行上：把它设为唯一选中；已在集合中则保持多选
    if (!multiSelectedCategoryIds.includes(category.id)) {
      setMultiSelectedCategoryIds([category.id]);
    }
    setCategoryContextMenu({ x: event.clientX, y: event.clientY });
  };

  // 任意点击 / Esc 关闭右键菜单
  useEffect(() => {
    if (!categoryContextMenu) return;
    const close = () => setCategoryContextMenu(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", close);
    };
  }, [categoryContextMenu]);

  const mergeSelectedIntoGroup = async () => {
    const ids = multiSelectedCategoryIds;
    setCategoryContextMenu(null);
    if (ids.length < 2) {
      toast("info", "按住 Ctrl 多选至少 2 个分类后再合并为组");
      return;
    }
    setMultiSelectedCategoryIds([]);
    try {
      await api.groupTagCategories(ids);
      await refreshCategories();
      toast("success", `已把 ${ids.length} 个分类合并为组（颜色已统一为最上层分类颜色）`);
    } catch (error) {
      toast("error", `合并分组失败：${String(error)}`);
    }
  };

  // ---- 分类顺序拖拽（pointer 自实现，与标签拖拽同构）。
  // 早期用 HTML5 DnD（draggable）：WebView2 里 dragstart 不触发、拖不动，
  // 故改为 pointerdown + window pointermove/up，经拖动阈值判定后整列表重排。----
  const clearCategorySort = () => {
    draggedCategoryIdRef.current = null;
    sortIndicatorRef.current = null;
    setDraggedCategoryId(null);
    setCategorySortIndicator(null);
  };

  const updateCategorySortIndicator = (clientX: number, clientY: number) => {
    const element = document
      .elementFromPoint(clientX, clientY)
      ?.closest<HTMLElement>(".category-accordion-item[data-category-id]");
    const dragged = draggedCategoryIdRef.current;
    if (!element || dragged === null) {
      if (sortIndicatorRef.current) {
        sortIndicatorRef.current = null;
        setCategorySortIndicator(null);
      }
      return;
    }
    const targetId = Number(element.dataset.categoryId);
    if (targetId === dragged) return;
    const rect = element.getBoundingClientRect();
    const before = clientY < rect.top + rect.height / 2;
    const next = { id: targetId, before };
    const current = sortIndicatorRef.current;
    if (!current || current.id !== next.id || current.before !== next.before) {
      sortIndicatorRef.current = next;
      setCategorySortIndicator(next);
    }
  };

  const removeCategoryPointerListeners = () => {
    if (attachedCategoryMoveRef.current) {
      window.removeEventListener("pointermove", attachedCategoryMoveRef.current);
    }
    if (attachedCategoryUpRef.current) {
      window.removeEventListener("pointerup", attachedCategoryUpRef.current);
      window.removeEventListener("pointercancel", attachedCategoryUpRef.current);
    }
    attachedCategoryMoveRef.current = null;
    attachedCategoryUpRef.current = null;
  };

  const moveCategoryPointerDrag = (event: PointerEvent) => {
    const state = categoryPointerRef.current;
    if (!state) return;
    if (!state.active) {
      const dx = event.clientX - state.startX;
      const dy = event.clientY - state.startY;
      if (dx * dx + dy * dy < 36) return; // 移动 6px 才进入拖拽，避免误触点击
      state.active = true;
      draggedCategoryIdRef.current = state.id;
      setDraggedCategoryId(state.id);
    }
    updateCategorySortIndicator(event.clientX, event.clientY);
    updateAutoScroll(event.clientY);
  };

  const endCategoryPointerDrag = (event: PointerEvent) => {
    const state = categoryPointerRef.current;
    if (!state) return;
    categoryPointerRef.current = null;
    removeCategoryPointerListeners();
    if (!state.active) {
      return; // 未超过阈值：交由 onClick 处理展开/折叠
    }
    const dragged = draggedCategoryIdRef.current;
    const target = sortIndicatorRef.current;
    clearCategorySort();
    // 已判定为拖动：抑制随后可能派发的 click（避免同时折叠/展开分类）
    suppressCategoryClick.current = true;
    window.setTimeout(() => {
      suppressCategoryClick.current = false;
    }, 300);
    if (dragged !== null && target && target.id !== dragged) {
      void commitCategoryReorder(dragged, target.id, target.before);
    }
  };

  const addCategoryPointerListeners = () => {
    attachedCategoryMoveRef.current = moveCategoryPointerDrag;
    attachedCategoryUpRef.current = endCategoryPointerDrag;
    window.addEventListener("pointermove", attachedCategoryMoveRef.current);
    window.addEventListener("pointerup", attachedCategoryUpRef.current);
    window.addEventListener("pointercancel", attachedCategoryUpRef.current);
  };

  const beginCategoryPointerDrag = (
    categoryId: number,
    event: React.PointerEvent<HTMLElement>
  ) => {
    if (event.button !== 0) return;
    if (draggedTagRef.current !== null) return; // 正在拖标签，不抢占
    const target = event.target as HTMLElement | null;
    if (target?.closest("button, input, a, .tag-edit-line")) return;
    event.preventDefault();
    clearCategorySort();
    categoryPointerRef.current = {
      id: categoryId,
      startX: event.clientX,
      startY: event.clientY,
      active: false
    };
    addCategoryPointerListeners();
  };

  // 分类组作为连续块：返回 anchor 所在块的 [start, end]（未分组 = 单行块）
  const blockRange = (list: TagCategory[], anchorId: number) => {
    const index = list.findIndex((category) => category.id === anchorId);
    if (index < 0) return null;
    const gid = list[index].groupId ?? null;
    if (gid === null) return { start: index, end: index };
    let start = index;
    while (start > 0 && (list[start - 1].groupId ?? null) === gid) start--;
    let end = index;
    while (end + 1 < list.length && (list[end + 1].groupId ?? null) === gid) end++;
    return { start, end };
  };

  const commitCategoryReorder = async (
    draggedId: number,
    targetId: number,
    before: boolean
  ) => {
    if (draggedId === targetId) return;
    const src = blockRange(categories, draggedId);
    const tgt = blockRange(categories, targetId);
    if (!src || !tgt) return;
    // 拖动块与目标块重叠（目标在组内）时视为无效
    if (src.start <= tgt.end && tgt.start <= src.end) return;
    const next = [...categories];
    const srcBlock = next.splice(src.start, src.end - src.start + 1);
    const tgtBlockLen = tgt.end - tgt.start + 1;
    const targetHead = categories[tgt.start].id;
    const targetIndex = next.findIndex((category) => category.id === targetHead);
    if (targetIndex < 0) return;
    next.splice(before ? targetIndex : targetIndex + tgtBlockLen, 0, ...srcBlock);
    setCategories(next); // 乐观更新，松开即见新顺序（整组一起移动）
    try {
      await api.reorderTagCategories(next.map((category) => category.id));
    } catch (error) {
      toast("error", `保存分类顺序失败：${String(error)}`);
    } finally {
      await refreshCategories(); // 以后端权威顺序兜底
    }
  };

  const setDragTarget = (categoryId: number | null) => {
    if (dragTargetRef.current?.categoryId === categoryId) return;
    dragTargetRef.current = { categoryId };
    setDragOverCategoryId(categoryId);
    setDragOverUncategorized(categoryId === null);
  };

  const clearDragTarget = () => {
    if (!dragTargetRef.current) return;
    dragTargetRef.current = null;
    setDragOverCategoryId(null);
    setDragOverUncategorized(false);
  };

  const dropTargetFromPoint = (clientX: number, clientY: number) => {
    const element = document
      .elementFromPoint(clientX, clientY)
      ?.closest<HTMLElement>("[data-drop-target]");
    if (!element) return null;
    if (element.dataset.dropTarget === "uncategorized") {
      return { categoryId: null as number | null };
    }
    const categoryId = element.dataset.categoryId
      ? Number(element.dataset.categoryId)
      : null;
    return { categoryId };
  };

  // 面板现在可滚动，拖动标签到屏幕外的分类时需要边缘自动滚动，
  // 否则「拖到分类归类」在标签多的情况下会变得不可用。
  const stopAutoScroll = () => {
    autoScrollVelocityRef.current = 0;
    if (autoScrollFrameRef.current !== null) {
      window.cancelAnimationFrame(autoScrollFrameRef.current);
      autoScrollFrameRef.current = null;
    }
  };

  const updateAutoScroll = (clientY: number) => {
    const panel = panelRef.current;
    const rect = panel?.getBoundingClientRect();
    if (!panel || !rect) return;
    const edge = 56;
    const maxSpeed = 18;
    let velocity = 0;
    if (clientY < rect.top + edge) {
      velocity = -maxSpeed * Math.min(1, (rect.top + edge - clientY) / edge);
    } else if (clientY > rect.bottom - edge) {
      velocity = maxSpeed * Math.min(1, (clientY - (rect.bottom - edge)) / edge);
    }
    autoScrollVelocityRef.current = velocity;
    if (velocity === 0 || autoScrollFrameRef.current !== null) return;
    const step = () => {
      const element = panelRef.current;
      const current = autoScrollVelocityRef.current;
      if (!element || current === 0) {
        autoScrollFrameRef.current = null;
        return;
      }
      element.scrollTop += current;
      autoScrollFrameRef.current = window.requestAnimationFrame(step);
    };
    autoScrollFrameRef.current = window.requestAnimationFrame(step);
  };

  const removePointerListeners = () => {
    stopAutoScroll();
    if (attachedPointerMoveRef.current) {
      window.removeEventListener("pointermove", attachedPointerMoveRef.current);
    }
    if (attachedPointerUpRef.current) {
      window.removeEventListener("pointerup", attachedPointerUpRef.current);
    }
    if (attachedPointerCancelRef.current) {
      window.removeEventListener("pointercancel", attachedPointerCancelRef.current);
    }
    attachedPointerMoveRef.current = null;
    attachedPointerUpRef.current = null;
    attachedPointerCancelRef.current = null;
    dragListenersAttachedRef.current = false;
  };

  function handlePointerMove(event: PointerEvent) {
    if (
      activePointerIdRef.current !== event.pointerId ||
      draggedTagRef.current === null
    ) {
      return;
    }
    event.preventDefault();
    updateAutoScroll(event.clientY);
    const target = dropTargetFromPoint(event.clientX, event.clientY);
    if (!target) {
      clearDragTarget();
      return;
    }
    setDragTarget(target.categoryId);
  }

  function handlePointerUp(event: PointerEvent) {
    if (
      activePointerIdRef.current !== event.pointerId ||
      draggedTagRef.current === null
    ) {
      return;
    }
    event.preventDefault();
    const tagId = draggedTagRef.current;
    const target = dropTargetFromPoint(event.clientX, event.clientY);
    removePointerListeners();
    activePointerIdRef.current = null;
    draggedTagRef.current = null;
    clearDragTarget();
    setDraggedTagId(null);
    suppressCategoryClick.current = true;
    window.setTimeout(() => {
      suppressCategoryClick.current = false;
    }, 200);
    if (tagId && target) {
      void assignCategory(tagId, target.categoryId);
    }
  }

  function handlePointerCancel(event: PointerEvent) {
    if (activePointerIdRef.current !== event.pointerId) return;
    removePointerListeners();
    activePointerIdRef.current = null;
    draggedTagRef.current = null;
    clearDragTarget();
    setDraggedTagId(null);
    window.setTimeout(() => {
      suppressCategoryClick.current = false;
    }, 200);
  }

  useEffect(() => {
    return () => {
      stopAutoScroll();
      removeCategoryPointerListeners();
      if (dragListenersAttachedRef.current) {
        removePointerListeners();
      }
    };
  }, []);

  const beginPointerDrag = (
    tagId: number,
    event: React.PointerEvent<HTMLElement>
  ) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    draggedTagRef.current = tagId;
    activePointerIdRef.current = event.pointerId;
    setDraggedTagId(tagId);
    suppressCategoryClick.current = true;
    if (!dragListenersAttachedRef.current) {
      dragListenersAttachedRef.current = true;
      attachedPointerMoveRef.current = handlePointerMove;
      attachedPointerUpRef.current = handlePointerUp;
      attachedPointerCancelRef.current = handlePointerCancel;
      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerup", handlePointerUp);
      window.addEventListener("pointercancel", handlePointerCancel);
    }
  };

  const beginRowPointerDrag = (
    tagId: number,
    event: React.PointerEvent<HTMLElement>
  ) => {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest("button:not(.tag-badge), input, a, .tag-edit-line")) {
      return;
    }
    beginPointerDrag(tagId, event);
  };

  const renameTag = async (id: number) => {
    if (!editingName.trim()) return;
    const current = tags.find((tag) => tag.id === id);
    await api.upsertTag({
      id,
      namespace: current?.namespace ?? "manual",
      name: editingName.trim(),
      categoryId: current?.categoryId
    });
    setEditingId(null);
    await onTagsChanged?.();
  };

  const deleteTag = async (tag: Tag) => {
    if (!window.confirm(`确定删除标签“${tag.name}”吗？该操作不会删除视频。`)) return;
    await api.deleteTag(tag.id);
    await onTagsChanged?.();
  };

  // 执行合并：源标签视频并入目标；目标名匹配池中已有标签则并入之，否则自动生成新标签
  const executeMerge = async () => {
    if (!mergeSource) {
      toast("info", "请先选择要合并的源标签");
      return;
    }
    const name = mergeTargetName.trim();
    if (!name) {
      toast("info", "请输入合并后的目标标签名");
      return;
    }
    if (mergeSource.name.toLowerCase() === name.toLowerCase()) {
      toast("info", "目标标签不能与源标签同名");
      return;
    }
    if (merging) return;
    setMerging(true);
    try {
      const existing = tags.find((tag) => tag.name.toLowerCase() === name.toLowerCase());
      const target =
        existing ?? (await api.upsertTag({ namespace: "manual", name }));
      await api.mergeTags(mergeSource.id, target.id);
      setMergeSource(null);
      setMergeTargetName("");
      setExpandedCategoryId((current) => (current === mergeSource.id ? null : current));
      await onTagsChanged?.();
      toast("success", `已将「${mergeSource.name}」合并到「${target.name}」`);
    } catch (error) {
      toast("error", `合并失败：${String(error)}`);
    } finally {
      setMerging(false);
    }
  };

  const uncategorized = tags.filter((tag) => !tag.categoryId);
  const activeCategory = categories.find((category) => category.id === expandedCategoryId) || null;
  const categoryById = new Map(categories.map((category) => [category.id, category]));

  // 标签检索：跨全部分类匹配标签名（大小写不敏感），搜到即可直接操作，
  // 不必先记住它属于哪个分类。
  const trimmedQuery = searchQuery.trim().toLowerCase();
  const isSearching = trimmedQuery.length > 0;
  const searchResults = isSearching
    ? tags.filter(
        (tag) =>
          tag.name.toLowerCase().includes(trimmedQuery) ||
          tag.normalized.toLowerCase().includes(trimmedQuery)
      )
    : [];
  const visibleTags = isSearching
    ? searchResults
    : activeCategory
      ? tags.filter((tag) => tag.categoryId === activeCategory.id)
      : uncategorized;

  const clearSearch = () => setSearchQuery("");

  const renderTagRow = (tag: Tag) => (
    <div
      className={`draggable-tag-row ${draggedTagId === tag.id ? "is-dragging" : ""}`}
      key={tag.id}
      onPointerDown={(event) => beginRowPointerDrag(tag.id, event)}
    >
      <span
        className="tag-drag-handle"
        title={isSearching ? "拖到左侧分类即可归位" : "拖动到分类"}
      >
        <GripVertical size={15} />
      </span>
      {editingId === tag.id ? (
        <div className="tag-edit-line">
          <input
            autoFocus
            value={editingName}
            onChange={(event) => setEditingName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void renameTag(tag.id);
              if (event.key === "Escape") setEditingId(null);
            }}
          />
          <button className="icon-button" type="button" onClick={() => renameTag(tag.id)}>
            <Save size={16} />
          </button>
        </div>
      ) : (
        <TagBadge tag={tag} />
      )}
      {isSearching && (
        <span className="tag-search-category" title="所属分类">
          <span
            className="category-color"
            style={{
              background:
                (tag.categoryId ? categoryById.get(tag.categoryId)?.color : null) ||
                "#64748b"
            }}
          />
          {(tag.categoryId ? categoryById.get(tag.categoryId)?.name : null) || "未分类"}
        </span>
      )}
      <button
        className="ghost-button small"
        type="button"
        onClick={() => {
          setEditingId(tag.id);
          setEditingName(tag.name);
        }}
      >
        重命名
      </button>
      <button
        className="icon-button danger"
        type="button"
        onClick={() => deleteTag(tag)}
        title="删除标签"
      >
        <Trash2 size={15} />
      </button>
    </div>
  );

  return (
    <div className="tag-manager-panel" ref={panelRef}>
      <div className="category-manager-layout">
        <aside className="category-sidebar">
          <div className="new-tag-card">
            <h2>新建标签</h2>
            <TagPoolInput
              pool={tags}
              selected={[]}
              onAdd={() => undefined}
              onRemove={() => undefined}
              onCreate={createTag}
              placeholder="输入后按空格或回车加入标签池"
            />

            <h2>新建分类</h2>
            <div className="category-create-row">
              <input
                value={newCategory}
                onChange={(event) => setNewCategory(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void addCategory();
                }}
                placeholder="例如：学习、影视"
              />
              <button className="icon-button" type="button" onClick={addCategory}>
                <FolderPlus size={16} />
              </button>
            </div>
          </div>

          <div className="category-accordion">
            <div className="category-sidebar-title">分类</div>
            <div
              role="button"
              tabIndex={0}
              data-drop-target="uncategorized"
              className={`category-accordion-item ${expandedCategoryId === null ? "is-active" : ""} ${
                dragOverUncategorized ? "is-drag-over" : ""
              }`}
              onClick={() => {
                if (suppressCategoryClick.current) return;
                if (multiSelectedCategoryIds.length > 0) clearMultiSelect();
                clearSearch();
                setExpandedCategoryId(null);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  clearSearch();
                  setExpandedCategoryId(null);
                }
              }}
            >
              <ChevronRight size={15} />
              <span>未分类</span>
              <span className="category-count">{uncategorized.length}</span>
            </div>
            {categories.map((category) => {
              const categoryTags = tags.filter((tag) => tag.categoryId === category.id);
              const expanded = expandedCategoryId === category.id;
              const multiSelected = multiSelectedCategoryIds.includes(category.id);
              return (
                <div
                  role="button"
                  tabIndex={0}
                  data-drop-target="category"
                  data-category-id={category.id}
                  className={`category-accordion-item ${expanded ? "is-active" : ""} ${
                    multiSelected ? "is-multi-selected" : ""
                  } ${dragOverCategoryId === category.id ? "is-drag-over" : ""} ${
                    draggedCategoryId === category.id ? "is-sort-source" : ""
                  } ${
                    categorySortIndicator?.id === category.id
                      ? categorySortIndicator.before
                        ? "is-sort-before"
                        : "is-sort-after"
                      : ""
                  }`}
                  key={category.id}
                  onClick={(event) => {
                    if (suppressCategoryClick.current) return;
                    // Ctrl / Cmd + 左键：多选切换（不改变展开），用于右键合并为组
                    if (event.ctrlKey || event.metaKey) {
                      event.preventDefault();
                      event.stopPropagation();
                      toggleCategoryMultiSelect(category.id);
                      return;
                    }
                    if (multiSelectedCategoryIds.length > 0) clearMultiSelect();
                    clearSearch();
                    setExpandedCategoryId(expanded ? null : category.id);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      clearSearch();
                      setExpandedCategoryId(expanded ? null : category.id);
                    }
                  }}
                  onContextMenu={(event) => openCategoryContextMenu(category, event)}
                  onPointerDown={(event) => beginCategoryPointerDrag(category.id, event)}
                  title="拖动调整分类顺序"
                >
                  {expanded ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
                  <span
                    className="category-color"
                    style={{ background: category.color || "#64748b" }}
                  />
                  <span className="category-accordion-name">{category.name}</span>
                  <span className="category-count">{categoryTags.length}</span>
                  {category.groupId != null && (
                    <span
                      className="category-group-mark"
                      title="已分组（拖动会移动整组；改色会整组同步）"
                    >
                      <Layers size={12} />
                    </span>
                  )}
                  <span
                    className="category-drag-handle"
                    title="拖动调整分类顺序"
                    aria-hidden="true"
                  >
                    <GripVertical size={13} />
                  </span>
                </div>
              );
            })}
          </div>
        </aside>

        <section
          className="category-detail"
          // 检索态下右侧不作为放置目标：避免把搜索结果误拖到右侧而意外改分类，
          // 拖到左侧分类归类仍然有效。
          data-drop-target={isSearching ? undefined : "detail"}
          data-category-id={
            isSearching ? undefined : activeCategory ? String(activeCategory.id) : ""
          }
        >
          <label className="tag-search-box tag-manager-search">
            <Search size={16} />
            <input
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") clearSearch();
              }}
              placeholder="检索标签（跨全部分类）"
              aria-label="检索标签"
            />
            {isSearching && (
              <button
                className="tag-search-clear"
                type="button"
                onClick={clearSearch}
                title="清空检索（Esc）"
                aria-label="清空检索"
              >
                <X size={14} />
              </button>
            )}
          </label>

          {/* 独立合并操作行：源标签检索 + 目标命名 + 执行 */}
          <div className="tag-merge-bar">
            <TagPoolInput
              pool={tags}
              selected={mergeSource ? [mergeSource] : []}
              single
              onAdd={(tag) => setMergeSource(tag)}
              onRemove={() => setMergeSource(null)}
              onCreate={() => undefined}
              placeholder="合并：检索或选择源标签"
            />
            <input
              className="merge-target-name-input"
              value={mergeTargetName}
              onChange={(event) => setMergeTargetName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void executeMerge();
                }
              }}
              placeholder="目标标签名（不存在将自动新建）"
            />
            <button
              className="secondary-button"
              type="button"
              disabled={merging || !mergeSource || !mergeTargetName.trim()}
              onClick={() => void executeMerge()}
            >
              {merging ? "合并中..." : "合并"}
            </button>
          </div>
          {mergeSource && (
            <p className="merge-bar-hint">
              将把「{mergeSource.name}」（{mergeSource.count ?? 0} 条收藏）合并到：
              {mergeTargetName.trim()
                ? tags.some(
                    (tag) =>
                      tag.name.toLowerCase() === mergeTargetName.trim().toLowerCase()
                  )
                  ? `已有标签「${mergeTargetName.trim()}」`
                  : `新标签「${mergeTargetName.trim()}」（自动创建）`
                : "（请输入目标标签名）"}
            </p>
          )}

          <div className="category-detail-head">
            <div>
              {activeCategory && renamingCategoryId === activeCategory.id && !isSearching ? (
                <div className="category-rename-panel">
                  <div className="category-rename-line">
                    <input
                      autoFocus
                      value={renamingCategoryName}
                      onChange={(event) => setRenamingCategoryName(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") void renameCategory(activeCategory);
                        if (event.key === "Escape") {
                          setRenamingCategoryId(null);
                          setRenamingCategoryName("");
                        }
                      }}
                    />
                    <button
                      className="icon-button"
                      type="button"
                      onClick={() => renameCategory(activeCategory)}
                      title="保存分类名称与颜色"
                    >
                      <Save size={16} />
                    </button>
                    <button
                      className="icon-button"
                      type="button"
                      onClick={() => {
                        setRenamingCategoryId(null);
                        setRenamingCategoryName("");
                      }}
                      title="取消重命名"
                    >
                      <X size={16} />
                    </button>
                  </div>
                  <div className="color-picker category-color-picker">
                    {categoryColorOptions.map((item) => (
                      <button
                        type="button"
                        key={item}
                        className={renamingCategoryColor === item ? "is-active" : ""}
                        style={{ background: item }}
                        onClick={() => setRenamingCategoryColor(item)}
                        aria-label={`选择分类颜色 ${item}`}
                      />
                    ))}
                    <label
                      className="category-color-custom"
                      title="自定义任意颜色"
                      aria-label="自定义分类颜色"
                    >
                      <input
                        type="color"
                        value={renamingCategoryColor}
                        onChange={(event) => setRenamingCategoryColor(event.target.value)}
                      />
                    </label>
                  </div>
                </div>
              ) : (
                <h2>{isSearching ? "检索结果" : activeCategory?.name || "未分类标签"}</h2>
              )}
              <p>
                {isSearching
                  ? `匹配到 ${visibleTags.length} 个标签`
                  : `${visibleTags.length} 个标签`}
              </p>
            </div>
            {activeCategory && !isSearching && (
              <div className="category-detail-actions">
                <button
                  className="ghost-button"
                  type="button"
                  onClick={() => beginRenameCategory(activeCategory)}
                >
                  <Pencil size={15} />
                  重命名
                </button>
                <button
                  className="ghost-button"
                  type="button"
                  onClick={() => deleteCategory(activeCategory)}
                >
                  <Trash2 size={15} />
                  删除分类
                </button>
              </div>
            )}
          </div>
          <div className="category-detail-tags">
            {visibleTags.length === 0 ? (
              <div className="empty-state">
                {isSearching ? (
                  <>
                    没有匹配「{searchQuery.trim()}」的标签。
                    <button
                      className="ghost-button small"
                      type="button"
                      onClick={clearSearch}
                    >
                      清空检索
                    </button>
                  </>
                ) : activeCategory ? (
                  "这个分类还没有标签，把未分类标签拖进来。"
                ) : (
                  "没有未分类标签。"
                )}
              </div>
            ) : (
              visibleTags.map(renderTagRow)
            )}
          </div>
        </section>
      </div>

      {categoryContextMenu && (
        <div
          className="category-context-menu"
          style={{ left: categoryContextMenu.x, top: categoryContextMenu.y }}
          onMouseDown={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            className="category-context-item"
            disabled={multiSelectedCategoryIds.length < 2}
            onClick={() => void mergeSelectedIntoGroup()}
            title={
              multiSelectedCategoryIds.length < 2
                ? "按住 Ctrl 逐个点击分类进行多选后再合并"
                : undefined
            }
          >
            <GitMerge size={15} />
            合并为组（{multiSelectedCategoryIds.length}）
            <span className="category-context-hint">
              颜色统一为最上层；整组一起拖动排序
            </span>
          </button>
        </div>
      )}
    </div>
  );
}
