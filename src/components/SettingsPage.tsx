import { useEffect, useState } from "react";
import { Database, LogOut, ShieldCheck } from "lucide-react";
import { api } from "../lib/api";
import type { BilibiliProfile } from "../types";

export function SettingsPage() {
  const [profile, setProfile] = useState<BilibiliProfile | null>(null);

  useEffect(() => {
    void api.getProfile().then(setProfile);
  }, []);

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
