import {
  Bookmark,
  Download,
  Library,
  Settings,
  Trash2
} from "lucide-react";
import type { AppView } from "../types";

interface SidebarProps {
  active: AppView;
  trashCount: number;
  onChange: (view: AppView) => void;
}

const navItems: Array<{ id: AppView; label: string; icon: typeof Library }> = [
  { id: "library", label: "收藏库", icon: Library },
  { id: "import", label: "导入", icon: Download },
  { id: "trash", label: "回收站", icon: Trash2 },
  { id: "settings", label: "设置", icon: Settings }
];

export function Sidebar({ active, trashCount, onChange }: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark">
          <Bookmark size={18} />
        </div>
        <div>
          <div className="brand-name">收藏管理器</div>
          <div className="brand-sub">本地标签化收藏</div>
        </div>
      </div>

      <nav className="side-nav" aria-label="主导航">
        {navItems.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type="button"
            className={`side-nav-item ${active === id ? "is-active" : ""}`}
            onClick={() => onChange(id)}
          >
            <Icon size={18} />
            <span>{label}</span>
            {id === "trash" && trashCount > 0 && (
              <span className="side-nav-badge">{trashCount > 99 ? "99+" : trashCount}</span>
            )}
          </button>
        ))}
      </nav>

      <div className="sidebar-foot">
        <span>仅保存元数据与链接</span>
      </div>
    </aside>
  );
}
