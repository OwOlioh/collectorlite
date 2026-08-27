import {
  Bookmark,
  Download,
  Library,
  Settings
} from "lucide-react";
import type { AppView } from "../types";

interface SidebarProps {
  active: AppView;
  onChange: (view: AppView) => void;
}

const navItems: Array<{ id: AppView; label: string; icon: typeof Library }> = [
  { id: "library", label: "视频库", icon: Library },
  { id: "import", label: "导入", icon: Download },
  { id: "settings", label: "设置", icon: Settings }
];

export function Sidebar({ active, onChange }: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark">
          <Bookmark size={18} />
        </div>
        <div>
          <div className="brand-name">B 站收藏管理器</div>
          <div className="brand-sub">本地标签化收藏库</div>
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
          </button>
        ))}
      </nav>

      <div className="sidebar-foot">
        <span>仅保存元数据与链接</span>
      </div>
    </aside>
  );
}
