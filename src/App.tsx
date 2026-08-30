import { useCallback, useEffect, useState } from "react";
import { api } from "./lib/api";
import type { AppView, Tag } from "./types";
import { LibraryPage } from "./components/LibraryPage";
import { ImportPage } from "./components/ImportPage";
import { SettingsPage } from "./components/SettingsPage";
import { Sidebar } from "./components/Sidebar";
import { ToastProvider } from "./components/Toast";
import { applyTheme, getStoredTheme, watchSystemTheme } from "./lib/theme";

export default function App() {
  const [active, setActive] = useState<AppView>("library");
  const [tags, setTags] = useState<Tag[]>([]);

  const refreshTags = useCallback(async () => {
    setTags(await api.listTags());
  }, []);

  useEffect(() => {
    void refreshTags();
  }, [refreshTags]);

  useEffect(() => {
    const mode = getStoredTheme();
    applyTheme(mode);
    return watchSystemTheme(() => applyTheme(getStoredTheme()));
  }, []);

  return (
    <ToastProvider>
      <div className="app-shell">
        <Sidebar active={active} onChange={setActive} />
        <main className="main-panel">
          <div className={`view-panel ${active === "library" ? "is-active" : ""}`}>
            <LibraryPage tags={tags} onTagsChanged={refreshTags} />
          </div>
          <div className={`view-panel ${active === "import" ? "is-active" : ""}`}>
            <ImportPage tagPool={tags} onTagsChanged={refreshTags} />
          </div>
          <div className={`view-panel ${active === "settings" ? "is-active" : ""}`}>
            <SettingsPage />
          </div>
        </main>
      </div>
    </ToastProvider>
  );
}
