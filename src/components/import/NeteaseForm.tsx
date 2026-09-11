import { ClipboardPaste, FolderDown, LoaderCircle, RefreshCcw } from "lucide-react";
import type { BilibiliProfile, CollectionInfo } from "../../types";
import { api } from "../../lib/api";

interface NeteaseFormProps {
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

export function NeteaseForm({
  busy, setError, setLoginBusy, profile, setProfile, collections, setCollections,
  selectedCollectionId, setSelectedCollectionId, publicUrl, setPublicUrl,
  parsedCollection, setParsedCollection, loadCollections, parseUrl,
  onPreviewFavorites, onPreviewPublic,
}: NeteaseFormProps) {
  return (
    <div className="login-block">
      <div className="account-line">
        {profile?.isLogin ? (
          <>
            <span className="avatar">{profile.name?.slice(0, 1) || "云"}</span>
            <span><strong>{profile.name}</strong></span>
            <button className="ghost-button" type="button" onClick={() => {
              void api.neteaseLogout();
              setProfile(null);
              setCollections([]);
              setSelectedCollectionId("");
            }}>退出</button>
          </>
        ) : (
          <div style={{ display: "grid", gap: "8px" }}>
            <p style={{ margin: 0, color: "var(--muted)", fontSize: "13px" }}>
              点击下方按钮打开网易云网页版并登录。登录后按 F12 → Network →
              任选一个请求 → 在 Request Headers 里复制 <strong>cookie</strong>，
              关键是要有 <strong>MUSIC_U</strong>，粘贴到下方后回车。
              <br />
              （扫���登录会被网易云风控拦截，所以这里用手动 cookie，与知乎一致。）
            </p>
            <button type="button" className="secondary-button" style={{ justifySelf: "start" }}
              onClick={() => api.openUrl("https://music.163.com/")}>
              打开网易云登录
            </button>
            <input
              style={{ minHeight: "36px", padding: "0 10px", border: "1px solid var(--border)", borderRadius: "7px" }}
              placeholder="MUSIC_U=xxx; ..."
              onKeyDown={async (e) => {
                if (e.key === "Enter") {
                  const cookie = (e.target as HTMLInputElement).value.trim();
                  if (cookie) {
                    setLoginBusy(true);
                    setError("");
                    try {
                      await api.neteaseSetCookie(cookie);
                      const p = await api.neteaseProfile();
                      setProfile(p);
                      if (p.isLogin) {
                        setCollections(await api.listNeteaseCollections());
                      } else {
                        setError("cookie 无效或已过期，请重新复制（需包含 MUSIC_U）。");
                      }
                    } catch (err) { setError(String(err)); }
                    finally { setLoginBusy(false); }
                  }
                }
              }}
            />
          </div>
        )}
      </div>

      <label className="field-label">选择歌单</label>
      <select className="select-control full" value={selectedCollectionId}
        onChange={(e) => setSelectedCollectionId(e.target.value)}
        disabled={!profile?.isLogin || collections.length === 0}>
        <option value="">请选择歌单</option>
        {collections.map((c) => (
          <option key={c.id} value={c.id}>{c.title}（{c.count}）</option>
        ))}
      </select>
      {profile?.isLogin && (
        <button className="ghost-button" type="button" onClick={loadCollections}>
          <RefreshCcw size={15} /> 刷新歌单
        </button>
      )}

      <button className="primary-button wide" type="button" onClick={onPreviewFavorites}
        disabled={!profile?.isLogin || !selectedCollectionId || busy}>
        {busy ? <LoaderCircle className="spin" size={17} /> : <FolderDown size={17} />}
        预览并配置标签（我的歌单）
      </button>

      <div className="import-section-divider" />
      <label className="field-label">或者粘贴歌单链接</label>
      <div className="input-with-button">
        <input value={publicUrl} onChange={(e) => setPublicUrl(e.target.value)}
          placeholder="https://music.163.com/#/playlist?id=123456" />
        <button className="secondary-button" type="button" onClick={parseUrl} disabled={busy}>
          <ClipboardPaste size={16} /> 解析
        </button>
      </div>
      {parsedCollection && (
        <div className="parsed-card">
          <strong>{parsedCollection.title}</strong>
          <span>{parsedCollection.owner || "公开用户"}</span>
          <span>{parsedCollection.count} 首</span>
        </div>
      )}

      <button className="primary-button wide" type="button" onClick={onPreviewPublic}
        disabled={!parsedCollection || busy}>
        {busy ? <LoaderCircle className="spin" size={17} /> : <FolderDown size={17} />}
        预览并配置标签（歌单链接）
      </button>
    </div>
  );
}
