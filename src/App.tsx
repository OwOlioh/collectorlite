import { useCallback, useEffect, useState } from "react";
import { api } from "./lib/api";
import type { AppView, Tag } from "./types";
import { LibraryPage } from "./components/LibraryPage";
import { ImportPage } from "./components/ImportPage";
import { SettingsPage } from "./components/SettingsPage";
import { TrashPage } from "./components/TrashPage";
import { Sidebar } from "./components/Sidebar";
import { ToastProvider } from "./components/Toast";
import { applyTheme, getStoredTheme, watchSystemTheme } from "./lib/theme";
import { getRetentionDays } from "./lib/retention";

export default function App() {
  const [active, setActive] = useState<AppView>("library");
  const [tags, setTags] = useState<Tag[]>([]);
  const [trashCount, setTrashCount] = useState(0);

  const refreshTags = useCallback(async () => {
    setTags(await api.listTags());
  }, []);

  const refreshTrashCount = useCallback(async () => {
    try {
      setTrashCount(await api.getTrashCount());
    } catch {
      /* 忽略：回收站计数不影响主流程 */
    }
  }, []);

  useEffect(() => {
    void refreshTags();
    void refreshTrashCount();
  }, [refreshTags, refreshTrashCount]);

  useEffect(() => {
    const mode = getStoredTheme();
    applyTheme(mode);
    return watchSystemTheme(() => applyTheme(getStoredTheme()));
  }, []);

  // 应用启动时自动清理超过保留期的回收站条目
  useEffect(() => {
    void api.autoPurgeTrash(getRetentionDays()).then(() => {
      void refreshTrashCount();
    });
  }, [refreshTrashCount]);

  return (
    <ToastProvider>
      <div className="app-shell">
        <Sidebar active={active} trashCount={trashCount} onChange={setActive} />
        <main className="main-panel">
          <div className={`view-panel ${active === "library" ? "is-active" : ""}`}>
            <LibraryPage
              tags={tags}
              onTagsChanged={refreshTags}
              onTrashChanged={refreshTrashCount}
            />
          </div>
          <div className={`view-panel ${active === "import" ? "is-active" : ""}`}>
            <ImportPage tagPool={tags} onTagsChanged={refreshTags} />
          </div>
          <div className={`view-panel ${active === "trash" ? "is-active" : ""}`}>
            <TrashPage onTrashChanged={refreshTrashCount} />
          </div>
          <div className={`view-panel ${active === "settings" ? "is-active" : ""}`}>
            <SettingsPage onOpenTrash={() => setActive("trash")} />
          </div>
        </main>
      </div>
    </ToastProvider>
  );
}
