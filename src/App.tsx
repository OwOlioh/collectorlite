import { useCallback, useEffect, useState } from "react";
import { api } from "./lib/api";
import type { AppView, Tag } from "./types";
import { LibraryPage } from "./components/LibraryPage";
import { ImportPage } from "./components/ImportPage";
import { SettingsPage } from "./components/SettingsPage";
import { TrashPage } from "./components/TrashPage";
import { StatsPage } from "./components/StatsPage";
import { DuplicatesPage } from "./components/DuplicatesPage";
import { Sidebar } from "./components/Sidebar";
import { CaptureBridgeListener } from "./components/CaptureBridgeListener";
import { CoverCacheListener } from "./components/CoverCacheListener";
import { NeteaseSyncListener } from "./components/NeteaseSyncListener";
import { ToastProvider, useToast } from "./components/Toast";
import { AutoBackupRunner } from "./components/AutoBackupRunner";
import { applyTheme, getStoredTheme, watchSystemTheme } from "./lib/theme";
import { getRetentionDays } from "./lib/retention";

export default function App() {
  const [active, setActive] = useState<AppView>("library");
  const [tags, setTags] = useState<Tag[]>([]);
  const [trashCount, setTrashCount] = useState(0);
  const [duplicateCount, setDuplicateCount] = useState(0);
  // 浏览器扩展入库后递增，用来通知收藏库重新拉列表
  const [libraryVersion, setLibraryVersion] = useState(0);
  // 统计页下钻：点来源扇区 / 标签条后，把筛选意图带去收藏库。消费后由 LibraryPage 回调清空。
  const [libraryDrill, setLibraryDrill] = useState<{
    source?: string;
    tagId?: number;
  } | null>(null);
  // 打开方式偏好独立成一个版本号：`libraryVersion` 递增会让收藏库整列表重刷，
  // 而切「客户端 / 浏览器」只是改卡片的行为提示，没必要重新拉一次数据。
  const [openPrefsVersion, setOpenPrefsVersion] = useState(0);

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

  const refreshDuplicateCount = useCallback(async () => {
    try {
      const groups = await api.getDuplicateGroups();
      setDuplicateCount(groups.length);
    } catch {
      /* 忽略：重复项计数不影响主流程 */
    }
  }, []);

  const handleCaptured = useCallback(() => {
    void refreshTags();
    void refreshTrashCount();
    setLibraryVersion((version) => version + 1);
  }, [refreshTags, refreshTrashCount]);

  // 合并重复项后：重复组数下降、被删项进回收站、收藏库需重刷
  const handleDuplicatesChanged = useCallback(() => {
    void refreshDuplicateCount();
    void refreshTrashCount();
    setLibraryVersion((version) => version + 1);
  }, [refreshDuplicateCount, refreshTrashCount]);

  // 统计页下钻：来源扇区 → 收藏库按该来源筛选；标签条 → 用标签名解析 id 后按标签筛选
  const handleDrillSource = useCallback((source: string) => {
    setLibraryDrill({ source });
    setActive("library");
  }, []);

  const handleDrillTag = useCallback(
    (name: string) => {
      const tag = tags.find((t) => t.name === name);
      setLibraryDrill(tag ? { tagId: tag.id } : null);
      setActive("library");
    },
    [tags]
  );

  // LibraryPage 应用完下钻筛选后回调，避免相同来源/标签的后续点击被旧状态吞掉
  const handleDrillConsumed = useCallback(() => setLibraryDrill(null), []);

  useEffect(() => {
    void refreshTags();
    void refreshTrashCount();
    void refreshDuplicateCount();
  }, [refreshTags, refreshTrashCount, refreshDuplicateCount]);

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
      <AutoBackupRunner />
      <CaptureBridgeListener onCaptured={handleCaptured} />
      <CoverCacheListener onCoversCached={handleCaptured} />
      <NeteaseSyncListener onSynced={handleCaptured} />
      <DuplicatesListener version={libraryVersion} />
      <div className="app-shell">
        <Sidebar
          active={active}
          trashCount={trashCount}
          duplicateCount={duplicateCount}
          onChange={setActive}
        />
        <main className="main-panel">
          <div className={`view-panel ${active === "library" ? "is-active" : ""}`}>
            <LibraryPage
              tags={tags}
              refreshToken={libraryVersion}
              isActive={active === "library"}
              openPrefsVersion={openPrefsVersion}
              drill={libraryDrill}
              onDrillConsumed={handleDrillConsumed}
              onTagsChanged={refreshTags}
              onTrashChanged={refreshTrashCount}
            />
          </div>
          <div className={`view-panel ${active === "import" ? "is-active" : ""}`}>
            <ImportPage tagPool={tags} onTagsChanged={refreshTags} />
          </div>
          <div className={`view-panel ${active === "stats" ? "is-active" : ""}`}>
            <StatsPage
              refreshToken={libraryVersion}
              isActive={active === "stats"}
              onDrillSource={handleDrillSource}
              onDrillTag={handleDrillTag}
            />
          </div>
          <div className={`view-panel ${active === "duplicates" ? "is-active" : ""}`}>
            <DuplicatesPage
              refreshToken={libraryVersion}
              isActive={active === "duplicates"}
              onChanged={handleDuplicatesChanged}
            />
          </div>
          <div className={`view-panel ${active === "trash" ? "is-active" : ""}`}>
            <TrashPage onTrashChanged={refreshTrashCount} isActive={active === "trash"} />
          </div>
          <div className={`view-panel ${active === "settings" ? "is-active" : ""}`}>
            <SettingsPage
              onOpenTrash={() => setActive("trash")}
              onObsidianChanged={() => setLibraryVersion((version) => version + 1)}
              onNeteaseSynced={() => setLibraryVersion((version) => version + 1)}
              onOpenPrefsChanged={() => setOpenPrefsVersion((version) => version + 1)}
            />
          </div>
        </main>
      </div>
    </ToastProvider>
  );
}

/**
 * 重复项提醒：挂在 ToastProvider 内才能用 useToast。
 * 启动、导入新书签、合并重复后都会触发（version 变化）——但只在「疑似重复组数」比上次记录多时才 toast，
 * 避免每次打开 app 或合并后弹打扰。记录存 localStorage，合并使组数减少时不提示、只更新记录。
 */
function DuplicatesListener({ version }: { version: number }) {
  const { toast } = useToast();
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const groups = await api.getDuplicateGroups();
        const count = groups.length;
        const last = Number(localStorage.getItem("dup:lastGroupCount") || "0");
        if (!cancelled && count > last) {
          toast("info", `发现 ${count} 组疑似重复，可在「重复项」页处理`);
        }
        localStorage.setItem("dup:lastGroupCount", String(count));
      } catch {
        /* 忽略：重复项计数不影响主流程 */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [version, toast]);
  return null;
}
