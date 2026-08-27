import type { Tag } from "../types";
import { Pencil } from "lucide-react";

interface TagBadgeProps {
  tag: Tag;
  selected?: boolean;
  onClick?: () => void;
  onRemove?: () => void;
  onEdit?: () => void;
  compact?: boolean;
}

export function TagBadge({
  tag,
  selected = false,
  onClick,
  onRemove,
  onEdit,
  compact = false
}: TagBadgeProps) {
  return (
    <button
      type="button"
      className={`tag-badge ${selected ? "is-selected" : ""} ${onClick ? "is-clickable" : ""}`}
      style={{ "--tag-color": tag.color || "#64748b" } as React.CSSProperties}
      onClick={onClick}
      title={tag.description || tag.name}
    >
      <span className="tag-dot" />
      <span>{tag.name}</span>
      {!compact && tag.count !== undefined && <span className="tag-count">{tag.count}</span>}
      {onRemove && (
        <span
          className="tag-remove"
          role="button"
          aria-label={`移除 ${tag.name}`}
          onClick={(event) => {
            event.stopPropagation();
            onRemove();
          }}
        >
          ×
        </span>
      )}
      {onEdit && (
        <span
          className="tag-edit"
          role="button"
          aria-label={`编辑 ${tag.name}`}
          onClick={(event) => {
            event.stopPropagation();
            onEdit();
          }}
        >
          <Pencil size={11} />
        </span>
      )}
    </button>
  );
}
