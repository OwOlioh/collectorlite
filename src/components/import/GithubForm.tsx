import { LoaderCircle, RefreshCcw } from "lucide-react";
import type { CollectionInfo } from "../../types";

interface GithubFormProps {
  busy: boolean;
  username: string;
  setUsername: (u: string) => void;
  collections: CollectionInfo[];
  onLoadStars: () => void;
}

export function GithubForm({ busy, username, setUsername, collections, onLoadStars }: GithubFormProps) {
  return (
    <div className="login-block">
      <div className="account-line">
        <div style={{ display: "grid", gap: "8px", width: "100%" }}>
          <p style={{ margin: 0, color: "var(--muted)", fontSize: "13px" }}>
            GitHub Stars 列表是公开的，无需登录。输入你的 GitHub <strong>用户名</strong> 即可获取。
          </p>
          <div className="input-with-button">
            <input value={username} onChange={(e) => setUsername(e.target.value)}
              placeholder="GitHub 用户名，如 OwOlioh"
              onKeyDown={(e) => { if (e.key === "Enter" && username.trim()) onLoadStars(); }} />
            <button className="secondary-button" type="button"
              onClick={onLoadStars} disabled={busy || !username.trim()}>
              {busy ? <LoaderCircle className="spin" size={16} /> : <RefreshCcw size={16} />}
              获取 Stars
            </button>
          </div>
        </div>
      </div>
      {collections.length > 0 && (
        <div className="parsed-card">
          <strong>{collections[0].title}</strong>
          <span>{collections[0].owner}</span>
        </div>
      )}
    </div>
  );
}