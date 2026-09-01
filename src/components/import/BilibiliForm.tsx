import { QRCodeSVG } from "qrcode.react";
import { ClipboardPaste, FolderDown, LoaderCircle, LogIn, RefreshCcw } from "lucide-react";
import type { BilibiliProfile, CollectionInfo, QrSession } from "../../types";
import { api } from "../../lib/api";

interface BilibiliFormProps {
  busy: boolean;
  setBusy: (v: boolean) => void;
  setError: (e: string) => void;
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
  qr: QrSession | null;
  setQr: (q: QrSession | null) => void;
  loginBusy: boolean;
  setLoginBusy: (b: boolean) => void;
  loadCollections: () => Promise<void>;
  startQr: () => Promise<void>;
  parsePublic: () => Promise<void>;
  onPreviewFavorites: () => Promise<void>;
  onPreviewPublic: () => Promise<void>;
}

export function BilibiliForm({
  busy, profile, setProfile, collections, setCollections,
  selectedCollectionId, setSelectedCollectionId, publicUrl, setPublicUrl,
  parsedCollection, qr, loginBusy, startQr, parsePublic, loadCollections,
  onPreviewFavorites, onPreviewPublic,
}: BilibiliFormProps) {
  return (
    <div className="login-block">
      <div className="account-line">
        {profile?.isLogin ? (
          <>
            <span className="avatar">
              {profile.face ? <img src={profile.face} alt="" /> : profile.name?.slice(0, 1) || "B"}
            </span>
            <span>
              <strong>{profile.name}</strong>
              <small>MID {profile.mid}</small>
            </span>
            <button className="ghost-button" type="button" onClick={() => {
              void api.logout();
              setProfile(null);
              setCollections([]);
              setSelectedCollectionId("");
            }}>退出</button>
          </>
        ) : (
          <button className="primary-button" type="button" onClick={startQr} disabled={loginBusy || Boolean(qr)}>
            {loginBusy ? <LoaderCircle className="spin" size={17} /> : <LogIn size={17} />}
            扫码登录
          </button>
        )}
      </div>

      {qr && (
        <div className="qr-panel">
          <div className="qr-frame"><QRCodeSVG value={qr.qrcodeUrl} size={188} /></div>
          <p>使用 Bilibili 客户端扫码并确认登录。</p>
          <button className="ghost-button" type="button" onClick={startQr}>
            <RefreshCcw size={15} /> 刷新二维码
          </button>
        </div>
      )}

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
      {profile?.isLogin && (
        <button className="primary-button wide" type="button" onClick={onPreviewFavorites}
          disabled={busy || !selectedCollectionId}>
          {busy ? <LoaderCircle className="spin" size={17} /> : <FolderDown size={17} />}
          预览并配置标签（登录收藏夹）
        </button>
      )}

      <div className="import-section-divider" />
      <label className="field-label">或者粘贴公开收藏夹链接</label>
      <div className="input-with-button">
        <input value={publicUrl} onChange={(e) => setPublicUrl(e.target.value)}
          placeholder="收藏夹/合集/系列链接，如 .../favlist?fid=... 或 .../channel/collectiondetail?sid=..." />
        <button className="secondary-button" type="button" onClick={parsePublic} disabled={busy}>
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