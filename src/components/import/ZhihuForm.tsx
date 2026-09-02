import { ClipboardPaste, FolderDown, LoaderCircle, RefreshCcw } from "lucide-react";
import type { BilibiliProfile, CollectionInfo } from "../../types";
import { api } from "../../lib/api";

interface ZhihuFormProps {
  busy: boolean;
  setError: (e: string) => void;
  setLoginBusy: (b: boolean) => void;
  profile: BilibiliProfile | null;
  setProfile: (p: BilibiliProfile | null) => void;
  collections: CollectionInfo[];
  setCollections: (c: CollectionInfo[]) => void;
  selectedCollectionId: string;
  setSelectedCollectionId: (id: string) => void;
  publicUrl: string;
  setPublicUrl: (url: string) => void;
  parsedCollection: CollectionInfo | null;
  setParsedCollection: (c: CollectionInfo | null) => void;
  loadCollections: () => Promise<void>;
  parseUrl: () => Promise<void>;
  onPreviewFavorites: () => void;
  onPreviewPublic: () => void;
}

export function ZhihuForm({
  busy, setError, setLoginBusy, profile, setProfile, collections, setCollections,
  selectedCollectionId, setSelectedCollectionId, publicUrl, setPublicUrl,
  parsedCollection, setParsedCollection, loadCollections, parseUrl,
  onPreviewFavorites, onPreviewPublic,
}: ZhihuFormProps) {
  return (
    <div className="login-block">
      <div className="account-line">
        {profile?.isLogin ? (
          <>
            <span className="avatar">{profile.name?.slice(0, 1) || "知"}</span>
            <span><strong>{profile.name}</strong></span>
            <button className="ghost-button" type="button" onClick={() => {
              void api.zhihuLogout();
              setProfile(null);
              setCollections([]);
              setSelectedCollectionId("");
            }}>退出</button>
          </>
        ) : (
          <div style={{display: "grid", gap: "8px"}}>
            <p style={{margin: 0, color: "var(--muted)", fontSize: "13px"}}>
              点击下方按钮在浏览器中打开知乎并登录。登录后按 F12 →
              Application → Cookies → zhihu.com →
              分别复制 <strong>z_c0</strong> 和 <strong>d_c0</strong> 的值，
              用分号拼接粘贴到下方，格式：<code>z_c0=xxx; d_c0=xxx</code>
            </p>
            <button type="button" className="secondary-button" style={{ justifySelf: "start" }}
              onClick={() => api.openUrl("https://www.zhihu.com/signin")}>
              打开知乎登录
            </button>
            <input
              style={{minHeight: "36px", padding: "0 10px", border: "1px solid var(--border)", borderRadius: "7px"}}
              placeholder="z_c0=xxx; d_c0=xxx"
              onKeyDown={async (e) => {
                if (e.key === "Enter") {
                  const cookie = (e.target as HTMLInputElement).value.trim();
                  if (cookie) {
                    setLoginBusy(true);
                    setError("");
                    try {
                      const p = await api.zhihuBrowserLogin(cookie);
                      setProfile(p);
                      if (p.isLogin) setCollections(await api.listZhihuCollections());
                    } catch (err) { setError(String(err)); }
                    finally { setLoginBusy(false); }
                  }
                }
              }}
            />
          </div>
        )}
      </div>

      <label className="field-label">选择收藏夹</label>
      <select className="select-control full" value={selectedCollectionId}
        onChange={(e) => setSelectedCollectionId(e.target.value)}
        disabled={!profile?.isLogin || collections.length === 0}>
        <option value="">请选择收藏夹</option>
        {collections.map((c) => (
          <option key={c.id} value={c.id}>{c.title}（{c.count}）</option>
        ))}
      </select>
      {profile?.isLogin && (
        <button className="ghost-button" type="button" onClick={loadCollections}>
          <RefreshCcw size={15} /> 刷新收藏夹
        </button>
      )}

      <button className="primary-button wide" type="button" onClick={onPreviewFavorites}
        disabled={!profile?.isLogin || !selectedCollectionId || busy}>
        {busy ? <LoaderCircle className="spin" size={17} /> : <FolderDown size={17} />}
        预览并配置标签（登录收藏夹）
      </button>

      <div className="import-section-divider" />
      <label className="field-label">或者粘贴收藏夹链接</label>
      <div className="input-with-button">
        <input value={publicUrl} onChange={(e) => setPublicUrl(e.target.value)}
          placeholder="https://www.zhihu.com/collection/123456" />
        <button className="secondary-button" type="button" onClick={parseUrl} disabled={busy}>
          <ClipboardPaste size={16} /> 解析
        </button>
      </div>
      {parsedCollection && (
        <div className="parsed-card">
          <strong>{parsedCollection.title}</strong>
          <span>{parsedCollection.owner || "公开用户"}</span>
          <span>{parsedCollection.count} 条</span>
        </div>
      )}

      <button className="primary-button wide" type="button" onClick={onPreviewPublic}
        disabled={!parsedCollection || busy}>
        {busy ? <LoaderCircle className="spin" size={17} /> : <FolderDown size={17} />}
        预览并配置标签（收藏夹链接）
      </button>
    </div>
  );
}
