import { useEffect, useState } from "react";
import {
  Check,
  Copy,
  Database,
  LogOut,
  Puzzle,
  RefreshCw,
  ShieldCheck,
  SunMoon,
  Trash2
} from "lucide-react";
import { api } from "../lib/api";
import { useToast } from "./Toast";
import type { BilibiliProfile, BridgeInfo } from "../types";
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
  const [bridge, setBridge] = useState<BridgeInfo | null>(null);
  const [copied, setCopied] = useState(false);
  const { toast } = useToast();

  useEffect(() => {
    void api.getProfile().then(setProfile);
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
