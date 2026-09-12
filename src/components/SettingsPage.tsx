import { useEffect, useState } from "react";
import {
  Archive,
  AppWindow,
  Check,
  Copy,
  Database,
  FileText,
  FolderPlus,
  LogOut,
  Music,
  PenLine,
  Puzzle,
  RefreshCw,
  ShieldCheck,
  SunMoon,
  Trash2
} from "lucide-react";
import { api } from "../lib/api";
import { useToast } from "./Toast";
import type {
  BilibiliProfile,
  BridgeInfo,
  NeteaseSyncReport,
  NeteaseSyncSettings,
  ObsidianSettings,
  OpenTarget
} from "../types";
import { applyTheme, getStoredTheme, storeTheme, type ThemeMode } from "../lib/theme";
import { getRetentionDays, setRetentionDays, RETENTION_OPTIONS } from "../lib/retention";
import {
  buildBackupFileName,
  formatBackupTime,
  getBackupSettings,
  setBackupSettings,
  type BackupSettings
} from "../lib/backup";

interface SettingsPageProps {
  onOpenTrash?: () => void;
  /** Obsidian 设置保存后回调，供 App 递增版本号、通知收藏库刷新导出按钮显隐。 */
  onObsidianChanged?: () => void;
  /** 手动同步改动了库内容时回调，同样是让收藏库重新拉列表。 */
  onNeteaseSynced?: () => void;
  /** 打开方式偏好（客户端 / 浏览器）改动后回调，让收藏库重读。 */
  onOpenPrefsChanged?: () => void;
}

/** 同步间隔候选（分钟）。后端有 5 分钟下限，低于它会被夹回去。 */
const SYNC_INTERVAL_OPTIONS = [5, 15, 30, 60];

const formatSyncTime = (seconds: number | null) => {
  if (!seconds) return "尚未同步过";
  const date = new Date(seconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours()
  )}:${pad(date.getMinutes())}`;
};

export function SettingsPage({
  onOpenTrash,
  onObsidianChanged,
  onNeteaseSynced,
  onOpenPrefsChanged
}: SettingsPageProps) {
  const [profile, setProfile] = useState<BilibiliProfile | null>(null);
  const [neteaseProfile, setNeteaseProfile] = useState<BilibiliProfile | null>(null);
  const [sync, setSync] = useState<NeteaseSyncSettings | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [openTargets, setOpenTargets] = useState<Record<string, OpenTarget>>({});
  const [theme, setTheme] = useState<ThemeMode>(getStoredTheme());
  const [retention, setRetention] = useState<number>(getRetentionDays());
  const [recaching, setRecaching] = useState(false);
  const [bridge, setBridge] = useState<BridgeInfo | null>(null);
  const [floatEnabled, setFloatEnabled] = useState(true);
  const [floatHotkey, setFloatHotkey] = useState("Ctrl+Alt+S");
  const [copied, setCopied] = useState(false);
  const [obsidian, setObsidian] = useState<ObsidianSettings>({
    enabled: false,
    vaultPath: "",
    vaultName: "",
    subdir: "收藏"
  });
  const [backup, setBackup] = useState<BackupSettings>(getBackupSettings());
  const [backingUp, setBackingUp] = useState(false);
  const { toast } = useToast();

  useEffect(() => {
    void api.getProfile().then(setProfile);
  }, []);

  useEffect(() => {
    void api
      .neteaseProfile()
      .then(setNeteaseProfile)
      .catch(() => setNeteaseProfile(null));
    void api
      .getNeteaseSyncSettings()
      .then(setSync)
      .catch(() => setSync(null));
  }, []);

  // 速记浮窗的开关与快捷键
  useEffect(() => {
    void api.nowplayingEnabled().then(setFloatEnabled).catch(() => undefined);
    void api.nowplayingHotkey().then(setFloatHotkey).catch(() => undefined);
  }, []);

  // 打开方式偏好。读失败的兜底是 client-first（后端默认值），卡片因此不会变成打不开。
  useEffect(() => {
    void api
      .getOpenPrefs()
      .then((p) => setOpenTargets(p.targets ?? {}))
      .catch(() => setOpenTargets({}));
  }, []);

  useEffect(() => {
    void api.getObsidianSettings().then(setObsidian).catch(() => {});
  }, []);

  useEffect(() => {
    void api
      .getBridgeInfo()
      .then(setBridge)
      .catch(() => setBridge(null));
  }, []);

  const copyToken = async () => {
    if (!bridge?.token) return;
    try {
      await navigator.clipboard.writeText(bridge.token);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      toast("error", "复制失败，请手动选中下方令牌复制");
    }
  };

  const regenerateToken = async () => {
    try {
      const next = await api.regenerateBridgeToken();
      setBridge(next);
      toast("success", "已重新生成令牌，记得同步到扩展选项页");
    } catch (e) {
      toast("error", `重新生成失败: ${String(e)}`);
    }
  };

  const saveSync = async (patch: Partial<NeteaseSyncSettings>) => {
    if (!sync) return;
    const next = { ...sync, ...patch };
    setSync(next);
    try {
      // 后端会把低于下限的间隔夹回去，用返回值刷新展示
      setSync(await api.saveNeteaseSyncSettings(next));
    } catch (e) {
      toast("error", `保存同步设置失败: ${String(e)}`);
    }
  };

  const reportSync = (report: NeteaseSyncReport) => {
    if (report.skippedReason) {
      toast("info", report.skippedReason);
      return;
    }
    if (report.added > 0 || report.removed > 0) {
      const removedHint =
        report.removed > 0 ? `，${report.removed} 首取消收藏已移入回收站` : "";
      toast("success", `同步完成：新增 ${report.added} 首${removedHint}`);
      onNeteaseSynced?.();
    } else {
      toast("info", `同步完成：${report.playlists} 个歌单都没有变化`);
    }
    if (report.errors.length > 0) {
      toast("error", report.errors.slice(0, 3).join("; "));
    }
  };

  const syncNow = async () => {
    setSyncing(true);
    try {
      reportSync(await api.syncNetease(true));
      // 同步会更新水位和上次同步时间，重新拉一次展示最新值
      setSync(await api.getNeteaseSyncSettings());
    } catch (e) {
      toast("error", `同步失败: ${String(e)}`);
    } finally {
      setSyncing(false);
    }
  };

  const changeOpenTarget = async (source: string, target: OpenTarget) => {
    // 先乐观更新，点了立刻有反馈；失败了再回滚并说明。
    const previous = openTargets;
    setOpenTargets({ ...previous, [source]: target });
    try {
      setOpenTargets((await api.setOpenTarget(source, target)).targets ?? {});
      onOpenPrefsChanged?.();
    } catch (error) {
      setOpenTargets(previous);
      toast("error", `保存打开方式失败：${String(error)}`);
    }
  };

  const changeTheme = (mode: ThemeMode) => {
    setTheme(mode);
    storeTheme(mode);
    applyTheme(mode);
  };

  const saveObsidian = async (next: ObsidianSettings) => {
    setObsidian(next);
    try {
      await api.setObsidianSettings(next);
      toast("success", "已保存 Obsidian 设置");
      onObsidianChanged?.();
    } catch (e) {
      toast("error", `保存失败: ${String(e)}`);
    }
  };

  const pickVault = async () => {
    try {
      const path = await api.pickObsidianVault();
      if (path) {
        const name = path.split(/[\\/]/).pop() || path;
        await saveObsidian({ ...obsidian, vaultPath: path, vaultName: name });
      }
    } catch (e) {
      toast("error", `选择目录失败: ${String(e)}`);
    }
  };

  const updateBackup = (patch: Partial<BackupSettings>) => {
    const next = { ...backup, ...patch };
    setBackup(next);
    setBackupSettings(next);
  };

  const toggleBackup = async (enabled: boolean) => {
    if (enabled && !backup.folder) {
      // 还没选过文件夹：先引导选择，选完自动启用
      try {
        const folder = await api.pickBackupFolder();
        if (!folder) {
          toast("info", "未启用自动备份：请先选择备份文件夹");
          return;
        }
        updateBackup({ enabled: true, folder });
        toast("success", "已启用自动备份（每 3 天一次）");
      } catch (e) {
        toast("error", `选择文件夹失败: ${String(e)}`);
      }
      return;
    }
    updateBackup({ enabled });
    if (!enabled) toast("info", "已关闭自动备份");
  };

  const changeBackupFolder = async () => {
    try {
      const folder = await api.pickBackupFolder();
      if (folder) updateBackup({ folder });
    } catch (e) {
      toast("error", `选择文件夹失败: ${String(e)}`);
    }
  };

  const backupNow = async () => {
    if (!backup.folder) {
      toast("info", "请先选择备份文件夹");
      return;
    }
    setBackingUp(true);
    try {
      const path = await api.backupNow(backup.folder, buildBackupFileName());
      updateBackup({ lastRunAt: Date.now() });
      toast("success", `备份完成：${path}`);
    } catch (e) {
      toast("error", `备份失败：${String(e)}`);
    } finally {
      setBackingUp(false);
    }
  };

  return (
    <section className="page settings-page">
      <header className="page-header">
        <div>
          <h1>设置</h1>
          <p>查看账号状态与本地存储信息。</p>
        </div>
      </header>

      <div className="settings-grid">
        <div className="settings-card">
          <div className="settings-icon"><SunMoon size={20} /></div>
          <div>
            <h2>外观</h2>
            <p>选择浅色或深色主题，或跟随系统设置自动切换。</p>
            <div className="theme-options">
              {([
                { value: "light", label: "浅色" },
                { value: "dark", label: "深色" },
                { value: "system", label: "跟随系统" }
              ] as { value: ThemeMode; label: string }[]).map((option) => (
                <button
                  key={option.value}
                  type="button"
                  className={`theme-opt ${theme === option.value ? "is-active" : ""}`}
                  onClick={() => changeTheme(option.value)}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>
        </div>

        <div className="settings-card">
          <div className="settings-icon"><ShieldCheck size={20} /></div>
          <div>
            <h2>账号状态</h2>
            <p>
              {profile?.isLogin
                ? `B站已登录：${profile.name}（MID ${profile.mid}）`
                : "B站未登录。Cookie 仅保存在本机凭据管理器中。"}
            </p>
          </div>
          {profile?.isLogin && (
            <button
              className="ghost-button"
              type="button"
              onClick={async () => {
                await api.logout();
                setProfile({ isLogin: false });
              }}
            >
              <LogOut size={16} />
              退出登录
            </button>
          )}
        </div>

        <div className="settings-card is-wide">
          <div className="settings-icon"><AppWindow size={20} /></div>
          <div style={{ minWidth: 0 }}>
            <h2>打开方式</h2>
            <p>点卡片封面或标题时，是唤起桌面客户端还是打开网页版。</p>
            <div className="theme-options">
              {(["client", "browser"] as OpenTarget[]).map((target) => (
                <button
                  key={target}
                  type="button"
                  className={`theme-opt ${
                    (openTargets.netease ?? "client") === target ? "is-active" : ""
                  }`}
                  onClick={() => void changeOpenTarget("netease", target)}
                >
                  {target === "client" ? "客户端优先" : "浏览器"}
                </button>
              ))}
            </div>
            <p className="muted">
              适用于网易云音乐。选了浏览器之后，卡片 hover 菜单里会换成「客户端」按钮，两边都不会丢入口。
            </p>
          </div>
        </div>

        <div className="settings-card is-wide">
          <div className="settings-icon"><Music size={20} /></div>
          <div style={{ minWidth: 0 }}>
            <h2>网易云音乐同步</h2>
            <p>
              {neteaseProfile?.isLogin
                ? `已登录：${neteaseProfile.name ?? "网易云用户"}。导入过的歌单会自动登记，之后按下面的频率增量同步新收藏的歌。`
                : "尚未登录。到「导入」页粘贴 Cookie 登录并导入一次歌单，之后就能开启同步。"}
            </p>
            {neteaseProfile?.isLogin && sync && (
              <>
                <label style={{ display: "flex", alignItems: "center", gap: 8, margin: "8px 0 12px" }}>
                  <input
                    type="checkbox"
                    checked={sync.enabled}
                    onChange={(e) => void saveSync({ enabled: e.target.checked })}
                  />
                  <span>启用自动同步</span>
                </label>
                <div className="theme-options">
                  {SYNC_INTERVAL_OPTIONS.map((minutes) => (
                    <button
                      key={minutes}
                      type="button"
                      className={`theme-opt ${sync.intervalMinutes === minutes ? "is-active" : ""}`}
                      onClick={() => void saveSync({ intervalMinutes: minutes })}
                    >
                      {minutes} 分钟
                    </button>
                  ))}
                </div>
                <label style={{ display: "flex", alignItems: "center", gap: 8, margin: "12px 0 8px" }}>
                  <input
                    type="checkbox"
                    checked={sync.autoRemoveUnfavorited}
                    onChange={(e) => void saveSync({ autoRemoveUnfavorited: e.target.checked })}
                  />
                  <span>在网易云取消收藏后，自动移入回收站（保留期内可恢复）</span>
                </label>
                <p className="muted">
                  已登记 {sync.playlistIds.length} 个歌单 · 上次同步：
                  {formatSyncTime(sync.lastSyncAt)}
                </p>
              </>
            )}
          </div>
          <button
            className="ghost-button"
            type="button"
            disabled={syncing || !neteaseProfile?.isLogin}
            onClick={() => void syncNow()}
          >
            <RefreshCw size={16} className={syncing ? "spin" : ""} />
            {syncing ? "同步中..." : "立即同步"}
          </button>
        </div>

        <div className="settings-card is-wide">
          <div className="settings-icon"><PenLine size={20} /></div>
          <div style={{ minWidth: 0 }}>
            <h2>速记浮窗</h2>
            <p>
              听歌时按 <strong>{floatHotkey}</strong> 唤出一个贴边小面板，给当前这首记批注、
              插时间戳、打标签。不用就关掉，窗口随即销毁，不留后台。
            </p>
            <label style={{ display: "flex", alignItems: "center", gap: 8, margin: "8px 0" }}>
              <input
                type="checkbox"
                checked={floatEnabled}
                onChange={(e) => {
                  const next = e.target.checked;
                  setFloatEnabled(next);
                  void api
                    .nowplayingSetEnabled(next)
                    .catch((error) => {
                      setFloatEnabled(!next);
                      toast("error", `保存失败：${String(error)}`);
                    });
                }}
              />
              <span>启用全局快捷键</span>
            </label>
            <p className="muted">
              关掉后仍可从这里手动打开。快捷键改动需要重启应用生效；若组合被其它程序占用，
              启动日志会给出提示。
            </p>
          </div>
          <button
            className="ghost-button"
            type="button"
            onClick={() =>
              void api.nowplayingOpen().catch((e) => toast("error", `打开失败：${String(e)}`))
            }
          >
            <PenLine size={16} />
            打开面板
          </button>
        </div>

        <div className="settings-card">
          <div className="settings-icon"><Database size={20} /></div>
          <div>
            <h2>本地数据</h2>
            <p>收藏元数据、标签和导入记录存储在本机 SQLite 数据库中，不包含视频或文件内容。</p>
            <p className="settings-note">
              封面在导入后由后台慢慢缓存，不阻塞导入；中途关闭应用也不会丢，下次启动会自动接着缓存。
            </p>
          </div>
          <button
            className="ghost-button"
            type="button"
            disabled={recaching}
            onClick={async () => {
              setRecaching(true);
              try {
                const result = await api.recacheCovers();
                if (result.cached > 0 && result.failed === 0) {
                  toast("success", `已重新缓存 ${result.cached} 张封面`);
                } else if (result.cached > 0) {
                  toast("info", `已缓存 ${result.cached} 张封面，${result.failed} 张失败`);
                } else if (result.failed > 0) {
                  const detail = result.errors?.length
                    ? `：${result.errors.slice(0, 3).join("; ")}`
                    : "";
                  toast("error", `封面缓存失败 ${result.failed} 张${detail}`);
                } else if (result.errors?.length) {
                  // 后台任务正在跑时，Rust 端返回的是一句提示而不是统计数字
                  toast("info", result.errors[0]);
                } else {
                  toast("info", "没有需要缓存的封面");
                }
              } catch (e) {
                toast("error", `封面缓存请求失败: ${String(e)}`);
              } finally {
                setRecaching(false);
              }
            }}
          >
            <RefreshCw size={16} className={recaching ? "spin" : ""} />
            {recaching ? "缓存中..." : "重新缓存封面"}
          </button>
        </div>

        <div className="settings-card">
          <div className="settings-icon"><Trash2 size={20} /></div>
          <div>
            <h2>回收站</h2>
            <p>删除的收藏会先进入回收站，超过下方保留期后将在应用启动时自动清除。</p>
            <div className="theme-options">
              {RETENTION_OPTIONS.map((days) => (
                <button
                  key={days}
                  type="button"
                  className={`theme-opt ${retention === days ? "is-active" : ""}`}
                  onClick={() => {
                    setRetention(days);
                    setRetentionDays(days);
                  }}
                >
                  {days} 天
                </button>
              ))}
            </div>
          </div>
          <button className="ghost-button" type="button" onClick={() => onOpenTrash?.()}>
            <Trash2 size={16} />
            打开回收站
          </button>
        </div>

        <div className="settings-card is-wide">
          <div className="settings-icon"><Archive size={20} /></div>
          <div style={{ minWidth: 0 }}>
            <h2>自动备份</h2>
            <p>
              启用后应用会每 3 天自动把整库（收藏、标签、分类）导出一份 JSON 备份到指定文件夹。
              备份内容与手动「导出全部」一致，可在导入页恢复。
            </p>
            <label style={{ display: "flex", alignItems: "center", gap: 8, margin: "8px 0 12px" }}>
              <input
                type="checkbox"
                checked={backup.enabled}
                onChange={(e) => void toggleBackup(e.target.checked)}
              />
              <span>启用自动备份（每 3 天一次）</span>
            </label>
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8, flexWrap: "wrap" }}>
              <span className="muted" style={{ minWidth: 0, wordBreak: "break-all" }}>
                {backup.folder || "尚未选择备份文件夹"}
              </span>
              <button className="ghost-button small" type="button" onClick={changeBackupFolder}>
                <FolderPlus size={14} />
                选择文件夹
              </button>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
              <span className="muted">上次备份：{formatBackupTime(backup.lastRunAt)}</span>
              <button
                className="ghost-button small"
                type="button"
                disabled={backingUp || !backup.folder}
                onClick={backupNow}
              >
                <RefreshCw size={14} className={backingUp ? "spin" : ""} />
                {backingUp ? "备份中..." : "立即备份"}
              </button>
            </div>
          </div>
        </div>

        <div className="settings-card is-wide">
          <div className="settings-icon"><FileText size={20} /></div>
          <div>
            <h2>Obsidian 笔记联动</h2>
            <p>
              把写过批注的收藏单向同步成 Obsidian 笔记（收藏 → 笔记）。关闭时完全不读写你的仓库，
              行为与普通批注一致。
            </p>
            <label style={{ display: "flex", alignItems: "center", gap: 8, margin: "8px 0 12px" }}>
              <input
                type="checkbox"
                checked={obsidian.enabled}
                onChange={(e) => void saveObsidian({ ...obsidian, enabled: e.target.checked })}
              />
              <span>启用联动</span>
            </label>
            {obsidian.enabled && (
              <>
                <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 12, flexWrap: "wrap" }}>
                  <span className="muted" style={{ minWidth: 0, wordBreak: "break-all" }}>
                    {obsidian.vaultPath || "尚未选择仓库目录"}
                  </span>
                  <button className="ghost-button small" type="button" onClick={() => void pickVault()}>
                    选择目录
                  </button>
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: 4, marginBottom: 12 }}>
                  <label style={{ fontSize: 13 }}>笔记子目录</label>
                  <input
                    className="bridge-token-input"
                    type="text"
                    value={obsidian.subdir}
                    placeholder="收藏"
                    onChange={(e) => setObsidian({ ...obsidian, subdir: e.target.value })}
                    onBlur={() => void saveObsidian(obsidian)}
                    aria-label="笔记子目录"
                  />
                </div>
                {obsidian.vaultName && (
                  <p className="muted">仓库名：{obsidian.vaultName}（Obsidian URI 用于定位仓库）</p>
                )}
              </>
            )}
          </div>
        </div>

        <div className="settings-card is-wide">
          <div className="settings-icon"><Puzzle size={20} /></div>
          <div>
            <h2>浏览器扩展</h2>
            <p>
              安装 Edge 扩展后，可以在网页侧边栏里给当前页打标签、写备注并一键收藏。
              本机桥只监听 127.0.0.1，每次请求都要带令牌。
            </p>
            <div className="bridge-status">
              <span className={`bridge-dot ${bridge?.running ? "is-on" : ""}`} />
              {bridge?.running
                ? `桥已启动，监听端口 ${bridge.port}`
                : "桥未启动（重新启动应用后会自动拉起）"}
            </div>
            <div className="bridge-token">
              <input
                className="bridge-token-input"
                type="text"
                readOnly
                value={bridge?.token ?? ""}
                onFocus={(event) => event.currentTarget.select()}
                aria-label="本机令牌"
              />
              <button className="ghost-button small" type="button" onClick={copyToken}>
                {copied ? <Check size={14} /> : <Copy size={14} />}
                {copied ? "已复制" : "复制"}
              </button>
            </div>
            <ol className="bridge-steps">
              <li>Edge 打开 <code>edge://extensions</code>，开启「开发人员模式」，点「加载解压缩的扩展」选择项目里的 <code>extension</code> 目录。</li>
              <li>右键扩展图标 →「扩展选项」，把上面的令牌粘贴进去保存。</li>
              <li>浏览网页时点扩展图标，侧边栏里配好标签和备注，点「收藏」即可入库。</li>
            </ol>
          </div>
          <button className="ghost-button" type="button" onClick={regenerateToken}>
            <RefreshCw size={16} />
            重新生成
          </button>
        </div>
      </div>

      <div className="privacy-note">
        <h3>数据与隐私</h3>
        <p>
          应用只请求必要的平台公开接口和登录态接口。所有数据保存在本地，
          不会上传到任何服务器。Cookie 通过系统凭据管理器加密存储。
        </p>
      </div>
    </section>
  );
}
