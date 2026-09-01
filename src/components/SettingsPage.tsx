import { useEffect, useState } from "react";
import { Database, LogOut, RefreshCw, ShieldCheck, SunMoon, Trash2 } from "lucide-react";
import { api } from "../lib/api";
import { useToast } from "./Toast";
import type { BilibiliProfile } from "../types";
import { applyTheme, getStoredTheme, storeTheme, type ThemeMode } from "../lib/theme";
import { getRetentionDays, setRetentionDays, RETENTION_OPTIONS } from "../lib/retention";

interface SettingsPageProps {
  onOpenTrash?: () => void;
}

export function SettingsPage({ onOpenTrash }: SettingsPageProps) {
  const [profile, setProfile] = useState<BilibiliProfile | null>(null);
  const [theme, setTheme] = useState<ThemeMode>(getStoredTheme());
  const [retention, setRetention] = useState<number>(getRetentionDays());
  const [recaching, setRecaching] = useState(false);
  const { toast } = useToast();

  useEffect(() => {
    void api.getProfile().then(setProfile);
  }, []);

  const changeTheme = (mode: ThemeMode) => {
    setTheme(mode);
    storeTheme(mode);
    applyTheme(mode);
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

        <div className="settings-card">
          <div className="settings-icon"><Database size={20} /></div>
          <div>
            <h2>本地数据</h2>
            <p>收藏元数据、标签和导入记录存储在本机 SQLite 数据库中，不包含视频或文件内容。</p>
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
