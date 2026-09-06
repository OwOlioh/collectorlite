import { useEffect, useRef, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FolderPlus,
  GripVertical,
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

interface TagManagerPanelProps {
  tags: Tag[];
  onTagsChanged?: () => void | Promise<void>;
}

export function TagManagerPanel({ tags, onTagsChanged }: TagManagerPanelProps) {
  const [categories, setCategories] = useState<TagCategory[]>([]);
  const { toast } = useToast();
  const [newCategory, setNewCategory] = useState("");
  const [expandedCategoryId, setExpandedCategoryId] = useState<number | null>(null);
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
  const [mergingTag, setMergingTag] = useState<Tag | null>(null);
  const [renamingCategoryId, setRenamingCategoryId] = useState<number | null>(null);
  const [renamingCategoryName, setRenamingCategoryName] = useState("");
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
  };

  const renameCategory = async (category: TagCategory) => {
    if (!renamingCategoryName.trim()) return;
    await api.renameTagCategory(
      category.id,
      renamingCategoryName.trim(),
      category.color
    );
    setRenamingCategoryId(null);
    setRenamingCategoryName("");
    await refreshCategories();
  };

  const assignCategory = async (tagId: number, categoryId: number | null) => {
    await api.assignTagCategory(tagId, categoryId);
    await onTagsChanged?.();
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

  const commitCategoryReorder = async (
    draggedId: number,
    targetId: number,
    before: boolean
  ) => {
    if (draggedId === targetId) return;
    const next = [...categories];
    const from = next.findIndex((category) => category.id === draggedId);
    if (from < 0) return;
    const [moved] = next.splice(from, 1);
    const targetIndex = next.findIndex((category) => category.id === targetId);
    if (targetIndex < 0) return;
    next.splice(before ? targetIndex : targetIndex + 1, 0, moved);
    setCategories(next); // 乐观更新，松开即见新顺序
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

  // 合并标签：把 source 名下的收藏全部并入 target，随后删除 source
  const mergeTagInto = async (source: Tag, target: Tag) => {
    setMergingTag(null);
    try {
      await api.mergeTags(source.id, target.id);
      setExpandedCategoryId((current) => (current === source.id ? null : current));
      await onTagsChanged?.();
      toast("success", `已将「${source.name}」合并到「${target.name}」`);
    } catch (error) {
      toast("error", `合并失败：${String(error)}`);
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
        title="把该标签合并到另一个标签（两者视频并入目标名下）"
        onClick={() => setMergingTag(tag)}
      >
        合并
      </button>
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
              return (
                <div
                  role="button"
                  tabIndex={0}
                  data-drop-target="category"
                  data-category-id={category.id}
                  className={`category-accordion-item ${expanded ? "is-active" : ""} ${
                    dragOverCategoryId === category.id ? "is-drag-over" : ""
                  } ${draggedCategoryId === category.id ? "is-sort-source" : ""} ${
                    categorySortIndicator?.id === category.id
                      ? categorySortIndicator.before
                        ? "is-sort-before"
                        : "is-sort-after"
                      : ""
                  }`}
                  key={category.id}
                  onClick={() => {
                    if (suppressCategoryClick.current) return;
                    clearSearch();
                    setExpandedCategoryId(expanded ? null : category.id);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      clearSearch();
                      setExpandedCategoryId(expanded ? null : category.id);
                    }
                  }}
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

          <div className="category-detail-head">
            <div>
              {activeCategory && renamingCategoryId === activeCategory.id && !isSearching ? (
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
                    title="保存分类名称"
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

      {mergingTag && (
        <div className="modal-backdrop" role="presentation" onMouseDown={() => setMergingTag(null)}>
          <div
            className="merge-tag-modal"
            role="dialog"
            aria-modal="true"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <div className="modal-head">
              <strong>合并标签</strong>
              <button className="icon-button" type="button" onClick={() => setMergingTag(null)}>
                <X size={16} />
              </button>
            </div>
            <p className="merge-tag-hint">
              将「{mergingTag.name}」（{mergingTag.count ?? 0} 条收藏）并入另一个标签：
              两个标签下的视频将合并到目标标签名下，「{mergingTag.name}」随后被删除。
            </p>
            <label className="field-label">合并到</label>
            <div className="merge-target-list">
              {tags
                .filter((candidate) => candidate.id !== mergingTag.id)
                .map((candidate) => (
                  <button
                    type="button"
                    className="merge-target-item"
                    key={candidate.id}
                    onClick={() => void mergeTagInto(mergingTag, candidate)}
                  >
                    <span
                      className="category-color"
                      style={{
                        background:
                          (candidate.categoryId
                            ? categoryById.get(candidate.categoryId)?.color
                            : null) || "#64748b"
                      }}
                    />
                    <span className="merge-target-name">{candidate.name}</span>
                    <span className="merge-target-count">{candidate.count ?? 0}</span>
                  </button>
                ))}
            </div>
            {tags.length <= 1 && (
              <p className="muted">标签池里没有其它标签可合并。</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
