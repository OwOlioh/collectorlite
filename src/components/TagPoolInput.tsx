import { useMemo, useRef, useState } from "react";
import { Plus, Search } from "lucide-react";
import type { Tag, TagNamespace } from "../types";
import { TagBadge } from "./TagBadge";

interface TagPoolInputProps {
  pool: Tag[];
  selected: Tag[];
  onAdd: (tag: Tag) => void;
  onRemove: (tag: Tag) => void;
  onCreate: (
    name: string,
    namespace: TagNamespace
  ) => Tag | void | Promise<Tag | void>;
  placeholder?: string;
  namespace?: TagNamespace;
  single?: boolean;
  disabled?: boolean;
}

export function TagPoolInput({
  pool,
  selected,
  onAdd,
  onRemove,
  onCreate,
  placeholder = "输入标签，空格或回车创建",
  namespace = "manual",
  single = false,
  disabled = false
}: TagPoolInputProps) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const blurTimer = useRef<number | undefined>(undefined);
  const effectiveOnRemove = disabled ? () => {} : onRemove;

  const normalized = query.trim().toLowerCase();
  const suggestions = useMemo(() => {
    if (!normalized) return [];
    return pool
      .filter((tag) => tag.normalized.toLowerCase().includes(normalized))
      .filter((tag) => !selected.some((item) => item.id === tag.id))
      .slice(0, 8);
  }, [normalized, pool, selected]);

  const commitQuery = async () => {
    const name = query.trim();
    if (!name) return;
    const exact = pool.find((tag) => tag.normalized.toLowerCase() === name.toLowerCase());
    if (exact) {
      onAdd(exact);
    } else {
      const created = await onCreate(name, namespace);
      if (created) {
        onAdd(created);
      }
    }
    setQuery("");
    setOpen(false);
  };

  const handleKeyDown = async (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === " " || event.key === "Enter" || event.key === ",") {
      event.preventDefault();
      await commitQuery();
      return;
    }
    if (event.key === "Backspace" && !query && !single && selected.length > 0) {
      event.preventDefault();
      onRemove(selected[selected.length - 1]);
    }
  };

  return (
    <div className={`tag-pool-input ${disabled ? "is-disabled" : ""}`}>
      <div className="tag-pool-input-row">
        <div className="tag-pool-selected">
          {selected.map((tag) => (
            <TagBadge
              key={tag.id}
              tag={tag}
              onRemove={() => effectiveOnRemove(tag)}
            />
          ))}
        </div>
        <Search size={16} />
        <input
          value={query}
          disabled={disabled}
          onChange={(event) => {
            setQuery(event.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onBlur={() => {
            blurTimer.current = window.setTimeout(() => setOpen(false), 120);
          }}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
        />
        {normalized && !disabled && (
          <button
            type="button"
            className="tag-pool-create"
            onMouseDown={(event) => {
              event.preventDefault();
              void commitQuery();
            }}
          >
            <Plus size={14} />
            创建
          </button>
        )}
      </div>
      {open && !disabled && suggestions.length > 0 && (
        <div className="tag-pool-menu">
          {suggestions.map((tag) => (
            <button
              type="button"
              key={tag.id}
              onMouseDown={(event) => {
                event.preventDefault();
                onAdd(tag);
                setQuery("");
                setOpen(false);
              }}
            >
              <TagBadge tag={tag} compact />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
