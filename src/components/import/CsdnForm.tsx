import { ClipboardPaste, FolderDown, LoaderCircle, RefreshCcw } from "lucide-react";
import type { CollectionInfo } from "../../types";

interface CsdnFormProps {
  busy: boolean;
  username: string;
  setUsername: (u: string) => void;
  collections: CollectionInfo[];
  selectedCollectionId: string;
  setSelectedCollectionId: (id: string) => void;
  publicUrl: string;
  setPublicUrl: (url: string) => void;
  parsedCollection: CollectionInfo | null;
  onLoadCollections: () => void;
  onParseUrl: () => void;
  onPreviewFavorites: () => Promise<void>;
  onPreviewPublic: () => Promise<void>;
}

export function CsdnForm({
  busy, username, setUsername, collections, selectedCollectionId,
  setSelectedCollectionId, publicUrl, setPublicUrl, parsedCollection,
  onLoadCollections, onParseUrl,
  onPreviewFavorites, onPreviewPublic,
}: CsdnFormProps) {
  return (
    <div className="login-block">
      <div className="account-line">
        <div style={{ display: "grid", gap: "8px", width: "100%" }}>
          <p style={{ margin: 0, color: "var(--muted)", fontSize: "13px" }}>
            CSDN 收藏夹是公开的，无需登录。请输入你的 CSDN <strong>英文用户名</strong>（非中文昵称）。
          </p>
          <p style={{ margin: 0, color: "var(--muted)", fontSize: "12px" }}>
            如何找到？登录 CSDN → 点击右上角头像 → 「我的主页」→ 地址栏中 <code>blog.csdn.net/</code> 后面的部分即为英文用户名。
          </p>
          <div className="input-with-button">
            <input value={username} onChange={(e) => setUsername(e.target.value)}
              placeholder="CSDN 英文用户名，如 LOVEmy134611"
              onKeyDown={(e) => { if (e.key === "Enter" && username.trim()) onLoadCollections(); }} />
            <button className="secondary-button" type="button"
              onClick={onLoadCollections} disabled={busy || !username.trim()}>
              {busy ? <LoaderCircle className="spin" size={16} /> : <RefreshCcw size={16} />}
              获取收藏夹
            </button>
          </div>
        </div>
      </div>

      <label className="field-label">选择收藏夹</label>
      <select className="select-control full" value={selectedCollectionId}
        onChange={(e) => setSelectedCollectionId(e.target.value)}
        disabled={collections.length === 0}>
        <option value="">请选择收藏夹</option>
        {collections.map((c) => (
          <option key={c.id} value={c.id}>{c.title}（{c.count}）</option>
        ))}
      </select>
      {collections.length > 0 && (
        <button className="ghost-button" type="button" onClick={onLoadCollections}>
          <RefreshCcw size={15} /> 刷新收藏夹
        </button>
      )}
      {collections.length > 0 && (
        <button className="primary-button wide" type="button" onClick={onPreviewFavorites}
          disabled={busy || !selectedCollectionId}>
          {busy ? <LoaderCircle className="spin" size={17} /> : <FolderDown size={17} />}
          预览并配置标签（用户名收藏夹）
        </button>
      )}

      <div className="import-section-divider" />
      <label className="field-label">或者粘贴收藏夹链接</label>
      <div className="input-with-button">
        <input value={publicUrl} onChange={(e) => setPublicUrl(e.target.value)}
          placeholder="https://blog.csdn.net/用户名/favorites?folderId=123" />
        <button className="secondary-button" type="button" onClick={onParseUrl} disabled={busy}>
          <ClipboardPaste size={16} /> 解析
        </button>
      </div>
      {parsedCollection && (
        <div className="parsed-card">
          <strong>{parsedCollection.title}</strong>
          <span>{parsedCollection.owner || "公开用户"}</span>
          <span>{parsedCollection.count} 条</span>
          <button className="primary-button wide" type="button" onClick={onPreviewPublic} disabled={busy}>
            {busy ? <LoaderCircle className="spin" size={17} /> : <FolderDown size={17} />}
            预览并配置标签（公开链接）
          </button>
        </div>
      )}
    </div>
  );
}