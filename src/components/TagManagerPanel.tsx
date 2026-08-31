import { useEffect, useRef, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FolderPlus,
  GripVertical,
  Pencil,
  Save,
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
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editingName, setEditingName] = useState("");
  const [renamingCategoryId, setRenamingCategoryId] = useState<number | null>(null);
  const [renamingCategoryName, setRenamingCategoryName] = useState("");
  const suppressCategoryClick = useRef(false);
  const draggedTagRef = useRef<number | null>(null);
  const activePointerIdRef = useRef<number | null>(null);
  const dragTargetRef = useRef<{ categoryId: number | null } | null>(null);
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

  const removePointerListeners = () => {
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

  const uncategorized = tags.filter((tag) => !tag.categoryId);
  const activeCategory = categories.find((category) => category.id === expandedCategoryId) || null;
  const visibleTags = activeCategory
    ? tags.filter((tag) => tag.categoryId === activeCategory.id)
    : uncategorized;

  const renderTagRow = (tag: Tag) => (
    <div
      className={`draggable-tag-row ${draggedTagId === tag.id ? "is-dragging" : ""}`}
      key={tag.id}
      onPointerDown={(event) => beginRowPointerDrag(tag.id, event)}
    >
      <span
        className="tag-drag-handle"
        title="拖动到分类"
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
    <div className="tag-manager-panel">
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
                setExpandedCategoryId(null);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  setExpandedCategoryId(null);
                }
              }}
            >
              <ChevronRight size={15} />
              <span>未分类</span>
              <span>{uncategorized.length}</span>
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
                  }`}
                  key={category.id}
                  onClick={() => {
                    if (suppressCategoryClick.current) return;
                    setExpandedCategoryId(expanded ? null : category.id);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      setExpandedCategoryId(expanded ? null : category.id);
                    }
                  }}
                >
                  {expanded ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
                  <span
                    className="category-color"
                    style={{ background: category.color || "#64748b" }}
                  />
                  <span>{category.name}</span>
                  <span>{categoryTags.length}</span>
                </div>
              );
            })}
          </div>
        </aside>

        <section
          className="category-detail"
          data-drop-target="detail"
          data-category-id={activeCategory ? String(activeCategory.id) : ""}
        >
          <div className="category-detail-head">
            <div>
              {activeCategory && renamingCategoryId === activeCategory.id ? (
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
                <h2>{activeCategory?.name || "未分类标签"}</h2>
              )}
              <p>{visibleTags.length} 个标签</p>
            </div>
            {activeCategory && (
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
                {activeCategory ? "这个分类还没有标签，把未分类标签拖进来。" : "没有未分类标签。"}
              </div>
            ) : (
              visibleTags.map(renderTagRow)
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
