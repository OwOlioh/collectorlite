import { useCallback, useEffect, useMemo, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  ClipboardPaste,
  FolderDown,
  Globe,
  Link,
  LoaderCircle,
  LogIn,
  RefreshCcw,
  Tag,
  Undo2,
  Upload
} from "lucide-react";
import { api } from "../lib/api";
import type {
  BilibiliProfile,
  BrowserImportRequest,
  CollectionInfo,
  ImportPreview,
  ImportResult,
  ItemTagAssignment,
  QrSession,
  Tag as AppTag,
  TagInput,
  TagNamespace,
  VideoItem
} from "../types";
import { TagPoolInput } from "./TagPoolInput";

type ImportMode = "login" | "public" | "browser" | "zhihu" | "zhihu_public";
type ImportStep = "source" | "tags" | "done";

interface PerVideoTagState {
  partitionTag: string;
  partitionManuallyEdited: boolean;
  otherTags: AppTag[];
}

interface ImportPageProps {
  tagPool: AppTag[];
  onTagsChanged: () => void;
}

const PAGE_SIZE = 8;

export function ImportPage({ tagPool, onTagsChanged }: ImportPageProps) {
  const [mode, setMode] = useState<ImportMode>("login");
  const [step, setStep] = useState<ImportStep>("source");
  const [profile, setProfile] = useState<BilibiliProfile | null>(null);
  const [zhihuProfile, setZhihuProfile] = useState<BilibiliProfile | null>(null);
  const [qr, setQr] = useState<QrSession | null>(null);
  const [loginBusy, setLoginBusy] = useState(false);
  const [collections, setCollections] = useState<CollectionInfo[]>([]);
  const [zhihuCollections, setZhihuCollections] = useState<CollectionInfo[]>([]);
  const [selectedCollectionId, setSelectedCollectionId] = useState("");
  const [zhihuSelectedCollectionId, setZhihuSelectedCollectionId] = useState("");
  const [publicUrl, setPublicUrl] = useState("");
  const [zhihuPublicUrl, setZhihuPublicUrl] = useState("");
  const [parsedCollection, setParsedCollection] = useState<CollectionInfo | null>(null);
  const [zhihuParsedCollection, setZhihuParsedCollection] = useState<CollectionInfo | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [folderPartitionEnabled, setFolderPartitionEnabled] = useState(false);
  const [folderPartitionTag, setFolderPartitionTag] = useState("");
  const [perVideoTags, setPerVideoTags] = useState<Record<string, PerVideoTagState>>({});
  const [currentPage, setCurrentPage] = useState(1);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [result, setResult] = useState<ImportResult | null>(null);
  const [browserHtmlContent, setBrowserHtmlContent] = useState("");
  const [browserFileName, setBrowserFileName] = useState("");
  const [browserItems, setBrowserItems] = useState<VideoItem[]>([]);

  const loadCollections = useCallback(async () => {
    setBusy(true);
    setError("");
    try {
      setCollections(await api.listBilibiliFavorites());
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }, []);

  const refreshProfile = useCallback(async () => {
    try {
      const next = await api.getProfile();
      setProfile(next);
      if (next.isLogin) {
        await loadCollections();
      }
    } catch (err) {
      setError(String(err));
    }
  }, [loadCollections]);

  useEffect(() => {
    void refreshProfile();
  }, [refreshProfile]);

  useEffect(() => {
    if (!qr) return;
    let stopped = false;
    let timer: number | undefined;

    const poll = async () => {
      try {
        const status = await api.pollQrLogin(qr.qrcodeKey);
        if (status.code === 0) {
          setQr(null);
          setProfile(status.profile || null);
          await loadCollections();
          return;
        }
        if (status.code === 86038) {
          setQr(null);
          setError("二维码已失效，请重新生成。");
          return;
        }
      } catch (err) {
        setError(String(err));
        setQr(null);
        return;
      }
      if (!stopped) {
        timer = window.setTimeout(poll, 1800);
      }
    };

    timer = window.setTimeout(poll, 1200);
    return () => {
      stopped = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [qr, loadCollections]);

  const startQr = async () => {
    setLoginBusy(true);
    setError("");
    try {
      setQr(await api.startQrLogin());
    } catch (err) {
      setError(String(err));
    } finally {
      setLoginBusy(false);
    }
  };

  const parsePublic = async () => {
    if (!publicUrl.trim()) {
      setError("请先粘贴公开收藏夹链接。");
      return;
    }
    setBusy(true);
    setError("");
    try {
      setParsedCollection(await api.parsePublicFavoriteUrl(publicUrl.trim()));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const parseZhihu = async () => {
    if (!zhihuPublicUrl.trim()) {
      setError("请先粘贴知乎收藏夹链接。");
      return;
    }
    setBusy(true);
    setError("");
    try {
      setZhihuParsedCollection(await api.parseZhihuCollectionUrl(zhihuPublicUrl.trim()));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const loadZhihuCollections = async () => {
    setBusy(true);
    setError("");
    try {
      setZhihuCollections(await api.listZhihuCollections());
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const parseBrowserHtmlLocally = async (html: string): Promise<VideoItem[]> => {
    const parser = new DOMParser();
    const doc = parser.parseFromString(html, "text/html");
    const items: VideoItem[] = [];
    let index = 0;

    // Hash function matching backend: SHA256 first 16 hex chars
    const hashUrl = async (url: string): Promise<string> => {
      const encoder = new TextEncoder();
      const data = encoder.encode(url);
      const hashBuffer = await crypto.subtle.digest("SHA-256", data);
      const hashArray = Array.from(new Uint8Array(hashBuffer));
      return "bk_" + hashArray.slice(0, 8).map(b => b.toString(16).padStart(2, "0")).join("");
    };

    const walk = async (node: Element, folderPath: string) => {
      const children = Array.from(node.children);
      for (const child of children) {
        if (child.tagName === "DT") {
          const h3 = child.querySelector(":scope > H3");
          const a = child.querySelector(":scope > A");
          if (h3) {
            const folderName = h3.textContent?.trim() || "";
            const dl = child.querySelector(":scope > DL");
            if (dl) {
              await walk(dl, folderPath ? `${folderPath} / ${folderName}` : folderName);
            }
          } else if (a) {
            const href = a.getAttribute("href") || "";
            const title = a.textContent?.trim() || "";
            const addDate = a.getAttribute("add_date");
            const icon = a.getAttribute("icon");
            const favoriteTime = addDate ? parseInt(addDate, 10) : undefined;
            const externalId = await hashUrl(href);
            index += 1;
            items.push({
              id: -index,
              source: "browser",
              externalId,
              sourceUrl: href,
              title: title || href,
              description: "",
              coverUrl: icon || undefined,
              authorName: undefined,
              partitionName: undefined,
              favoriteTime,
              tags: [],
              duration: undefined,
              publishedAt: undefined,
              authorId: undefined,
              coverLocalPath: undefined,
              notes: undefined
            });
          }
        }
        if (child.tagName === "DL") {
          await walk(child, folderPath);
        }
      }
    };

    const body = doc.querySelector("body");
    if (body) {
      const dl = body.querySelector("DL");
      if (dl) {
        await walk(dl, "");
      }
    }
    return items;
  };

  const handleBrowserFile = (file: File) => {
    setBrowserFileName(file.name);
    const reader = new FileReader();
    reader.onload = async () => {
      const html = reader.result as string;
      setBrowserHtmlContent(html);
      const items = await parseBrowserHtmlLocally(html);
      setBrowserItems(items);
      setError("");
    };
    reader.onerror = () => {
      setError("读取文件失败。");
    };
    reader.readAsText(file);
  };

  const handleBrowserDrop = (event: React.DragEvent) => {
    event.preventDefault();
    const file = event.dataTransfer.files[0];
    if (file) {
      handleBrowserFile(file);
    }
  };

  const handleBrowserFileInput = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (file) {
      handleBrowserFile(file);
    }
  };

  const currentCollection = useMemo(() => {
    if (mode === "public") return parsedCollection;
    if (mode === "zhihu_public") return zhihuParsedCollection;
    if (mode === "browser" && browserItems.length > 0) {
      return {
        source: "browser",
        id: "browser-bookmarks",
        title: browserFileName || "浏览器书签",
        owner: undefined,
        count: browserItems.length,
        url: undefined
      } as CollectionInfo;
    }
    if (mode === "zhihu") {
      return zhihuCollections.find((item) => item.id === zhihuSelectedCollectionId) || null;
    }
    return collections.find((item) => item.id === selectedCollectionId) || null;
  }, [mode, parsedCollection, zhihuParsedCollection, collections, zhihuCollections, selectedCollectionId, zhihuSelectedCollectionId, browserItems, browserFileName]);

  const startPreview = async () => {
    if (!currentCollection) {
      setError("请先选择或解析一个收藏夹。");
      return;
    }
    setBusy(true);
    setError("");

    if (mode === "browser") {
      const items = browserItems;
      const states: Record<string, PerVideoTagState> = {};
      items.forEach((item) => {
        states[item.externalId] = {
          partitionTag: "",
          partitionManuallyEdited: false,
          otherTags: []
        };
      });
      const previewData: ImportPreview = {
        collection: currentCollection,
        items,
        partitionSuggestions: []
      };
      setPreview(previewData);
      setPerVideoTags(states);
      setFolderPartitionEnabled(false);
      setFolderPartitionTag("");
      setCurrentPage(1);
      setStep("tags");
      setBusy(false);
      return;
    }

    try {
      const isZhihu = mode === "zhihu" || mode === "zhihu_public";
      const input = {
        kind: (mode === "login" || mode === "zhihu") ? ("favorites" as const) : ("public_url" as const),
        mediaId: (mode === "login" || mode === "zhihu") ? currentCollection.id : undefined,
        url: (mode === "public" || mode === "zhihu_public") ? (mode === "zhihu_public" ? zhihuPublicUrl.trim() : publicUrl.trim()) : undefined,
        tagSpecs: [],
        itemTagAssignments: []
      };
      const next = isZhihu ? await api.previewZhihuImport(input) : await api.previewImport(input);
      const states: Record<string, PerVideoTagState> = {};
      next.items.forEach((item) => {
        states[item.externalId] = {
          partitionTag: item.partitionName || "",
          partitionManuallyEdited: false,
          otherTags: []
        };
      });
      setPreview(next);
      setPerVideoTags(states);
      setFolderPartitionEnabled(false);
      setFolderPartitionTag("");
      setCurrentPage(1);
      setStep("tags");
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const updateVideoTag = (
    externalId: string,
    patch: Partial<PerVideoTagState>
  ) => {
    setPerVideoTags((current) => ({
      ...current,
      [externalId]: {
        ...current[externalId],
        ...patch
      }
    }));
  };

  const updateFolderPartitionTag = (value: string) => {
    setFolderPartitionTag(value);
    if (!folderPartitionEnabled) return;
    setPerVideoTags((current) => {
      const next = { ...current };
      Object.entries(next).forEach(([externalId, state]) => {
        if (!state.partitionManuallyEdited) {
          next[externalId] = { ...state, partitionTag: value };
        }
      });
      return next;
    });
  };

  const createTag = async (name: string, namespace: TagNamespace) => {
    const tag = await api.upsertTag({ namespace, name });
    onTagsChanged();
    return tag;
  };

  const buildTagSpecs = (item: VideoItem, state: PerVideoTagState): TagInput[] => {
    const specs: TagInput[] = [];
    if (state.partitionTag.trim()) {
      specs.push({ namespace: "auto", name: state.partitionTag.trim() });
    }
    state.otherTags.forEach((tag) => {
      specs.push({
        id: tag.id,
        namespace: tag.namespace,
        name: tag.name,
        color: tag.color
      });
    });
    return specs;
  };

  const execute = async () => {
    if (!currentCollection || !preview) return;
    setBusy(true);
    setError("");
    try {
      if (mode === "browser") {
        // Build per-item tag assignments from preview state
        const itemTagAssignments: ItemTagAssignment[] = (preview?.items || []).map((item) => {
          const state = perVideoTags[item.externalId];
          return {
            externalId: item.externalId,
            tagSpecs: state ? buildTagSpecs(item, state) : []
          };
        });
        const request: BrowserImportRequest = {
          htmlContent: browserHtmlContent,
          tagSpecs: [],
          itemTagAssignments
        };
        const next = await api.importBrowserBookmarks(request);
        setResult(next);
        setStep("done");
        window.setTimeout(() => {
          setStep("source");
          setPreview(null);
          setResult(null);
          setPerVideoTags({});
          setBrowserHtmlContent("");
          setBrowserFileName("");
          setBrowserItems([]);
        }, 1200);
        return;
      }

      const assignments: ItemTagAssignment[] = preview.items.map((item) => ({
        externalId: item.externalId,
        tagSpecs: buildTagSpecs(item, perVideoTags[item.externalId])
      }));
      const isZhihu = mode === "zhihu" || mode === "zhihu_public";
      const input = {
        kind: (mode === "login" || mode === "zhihu") ? ("favorites" as const) : ("public_url" as const),
        mediaId: (mode === "login" || mode === "zhihu") ? currentCollection.id : undefined,
        url: (mode === "public" || mode === "zhihu_public") ? (mode === "zhihu_public" ? zhihuPublicUrl.trim() : publicUrl.trim()) : undefined,
        tagSpecs: [],
        itemTagAssignments: assignments
      };
      const next = isZhihu ? await api.executeZhihuImport(input) : await api.executeImport(input);
      setResult(next);
      setStep("done");
      window.setTimeout(() => {
        setStep("source");
        setPreview(null);
        setResult(null);
        setPerVideoTags({});
      }, 1200);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const totalPages = Math.max(1, Math.ceil((preview?.items.length || 0) / PAGE_SIZE));
  const visibleItems = (preview?.items || []).slice(
    (currentPage - 1) * PAGE_SIZE,
    currentPage * PAGE_SIZE
  );

  return (
    <section className="page import-page">
      <header className="page-header">
        <div>
          <h1>导入收藏内容</h1>
          <p>从 B 站或浏览器导入收藏，配置标签整理。</p>
        </div>
      </header>

      {error && <div className="alert">{error}</div>}

      {step === "source" && (
        <div className="import-source-grid">
          <div className="import-modes">
            <button
              type="button"
              className={`import-mode-card ${mode === "login" ? "is-active" : ""}`}
              onClick={() => setMode("login")}
            >
              <LogIn size={20} />
              <strong>登录 B 站</strong>
              <span>读取自己的收藏夹，并可在导入后清理原收藏夹。</span>
            </button>
            <button
              type="button"
              className={`import-mode-card ${mode === "public" ? "is-active" : ""}`}
              onClick={() => setMode("public")}
            >
              <Link size={20} />
              <strong>公开收藏夹链接</strong>
              <span>不登录，仅复制公开收藏夹内容到本地。</span>
            </button>
            <button
              type="button"
              className={`import-mode-card ${mode === "browser" ? "is-active" : ""}`}
              onClick={() => setMode("browser")}
            >
              <Globe size={20} />
              <strong>浏览器书签</strong>
              <span>从浏览器导出的书签 HTML 文件中导入链接。</span>
            </button>
            <button
              type="button"
              className={`import-mode-card ${mode === "zhihu" ? "is-active" : ""}`}
              onClick={() => setMode("zhihu")}
            >
              <LogIn size={20} />
              <strong>登录知乎</strong>
              <span>读取自己的知乎收藏夹，并导入到本地。</span>
            </button>
            <button
              type="button"
              className={`import-mode-card ${mode === "zhihu_public" ? "is-active" : ""}`}
              onClick={() => setMode("zhihu_public")}
            >
              <Link size={20} />
              <strong>知乎公开收藏夹</strong>
              <span>不登录，输入知乎收藏夹链接导入内容。</span>
            </button>
          </div>

          <div className="import-form-panel">
            {mode === "login" ? (
              <div className="login-block">
                <div className="account-line">
                  {profile?.isLogin ? (
                    <>
                      <span className="avatar">
                        {profile.face ? (
                          <img src={profile.face} alt="" />
                        ) : (
                          profile.name?.slice(0, 1) || "B"
                        )}
                      </span>
                      <span>
                        <strong>{profile.name}</strong>
                        <small>MID {profile.mid}</small>
                      </span>
                      <button
                        className="ghost-button"
                        type="button"
                        onClick={() => {
                          void api.logout();
                          setProfile(null);
                          setCollections([]);
                          setSelectedCollectionId("");
                        }}
                      >
                        退出
                      </button>
                    </>
                  ) : (
                    <button
                      className="primary-button"
                      type="button"
                      onClick={startQr}
                      disabled={loginBusy || Boolean(qr)}
                    >
                      {loginBusy ? <LoaderCircle className="spin" size={17} /> : <LogIn size={17} />}
                      扫码登录
                    </button>
                  )}
                </div>

                {qr && (
                  <div className="qr-panel">
                    <div className="qr-frame">
                      <QRCodeSVG value={qr.qrcodeUrl} size={188} />
                    </div>
                    <p>使用 Bilibili 客户端扫码并确认登录。</p>
                    <button className="ghost-button" type="button" onClick={startQr}>
                      <RefreshCcw size={15} />
                      刷新二维码
                    </button>
                  </div>
                )}

                <label className="field-label">选择收藏夹</label>
                <select
                  className="select-control full"
                  value={selectedCollectionId}
                  onChange={(event) => setSelectedCollectionId(event.target.value)}
                  disabled={!profile?.isLogin || collections.length === 0}
                >
                  <option value="">请选择收藏夹</option>
                  {collections.map((collection) => (
                    <option key={collection.id} value={collection.id}>
                      {collection.title}（{collection.count}）
                    </option>
                  ))}
                </select>
                {profile?.isLogin && (
                  <button className="ghost-button" type="button" onClick={loadCollections}>
                    <RefreshCcw size={15} />
                    刷新收藏夹
                  </button>
                )}
              </div>
            ) : mode === "public" ? (
              <div className="public-block">
                <label className="field-label">B站公开收藏夹 URL</label>
                <div className="input-with-button">
                  <input
                    value={publicUrl}
                    onChange={(event) => setPublicUrl(event.target.value)}
                    placeholder="https://space.bilibili.com/.../favlist?fid=..."
                  />
                  <button className="secondary-button" type="button" onClick={parsePublic} disabled={busy}>
                    <ClipboardPaste size={16} />
                    解析
                  </button>
                </div>
                {parsedCollection && (
                  <div className="parsed-card">
                    <strong>{parsedCollection.title}</strong>
                    <span>{parsedCollection.owner || "公开用户"}</span>
                    <span>{parsedCollection.count} 条</span>
                  </div>
                )}
              </div>
            ) : mode === "zhihu_public" ? (
              <div className="public-block">
                <label className="field-label">知乎收藏夹 URL</label>
                <div className="input-with-button">
                  <input
                    value={zhihuPublicUrl}
                    onChange={(event) => setZhihuPublicUrl(event.target.value)}
                    placeholder="https://www.zhihu.com/collection/123456"
                  />
                  <button className="secondary-button" type="button" onClick={parseZhihu} disabled={busy}>
                    <ClipboardPaste size={16} />
                    解析
                  </button>
                </div>
                {zhihuParsedCollection && (
                  <div className="parsed-card">
                    <strong>{zhihuParsedCollection.title}</strong>
                    <span>{zhihuParsedCollection.owner || "公开用户"}</span>
                    <span>{zhihuParsedCollection.count} 条</span>
                  </div>
                )}
              </div>
            ) : mode === "zhihu" ? (
              <div className="login-block">
                <div className="account-line">
                  {zhihuProfile?.isLogin ? (
                    <>
                      <span className="avatar">
                        {zhihuProfile.name?.slice(0, 1) || "知"}
                      </span>
                      <span>
                        <strong>{zhihuProfile.name}</strong>
                      </span>
                      <button
                        className="ghost-button"
                        type="button"
                        onClick={() => {
                          void api.zhihuLogout();
                          setZhihuProfile(null);
                          setZhihuCollections([]);
                          setZhihuSelectedCollectionId("");
                        }}
                      >
                        退出
                      </button>
                    </>
                  ) : (
                    <div style={{display: "grid", gap: "8px"}}>
                      <p style={{margin: 0, color: "var(--muted)", fontSize: "13px"}}>
                        在浏览器中登录知乎后，复制 cookie 粘贴到下方：
                      </p>
                      <input
                        style={{minHeight: "36px", padding: "0 10px", border: "1px solid var(--border)", borderRadius: "7px"}}
                        placeholder="粘贴知乎 cookie 字符串"
                        onKeyDown={async (e) => {
                          if (e.key === "Enter") {
                            const cookie = (e.target as HTMLInputElement).value.trim();
                            if (cookie) {
                              setLoginBusy(true);
                              try {
                                await api.zhihuSetCookie(cookie);
                                const p = await api.zhihuProfile();
                                setZhihuProfile(p);
                              } catch (err) {
                                setError(String(err));
                              } finally {
                                setLoginBusy(false);
                              }
                            }
                          }
                        }}
                      />
                    </div>
                  )}
                </div>
                <label className="field-label">选择收藏夹</label>
                <select
                  className="select-control full"
                  value={zhihuSelectedCollectionId}
                  onChange={(event) => setZhihuSelectedCollectionId(event.target.value)}
                  disabled={!zhihuProfile?.isLogin || zhihuCollections.length === 0}
                >
                  <option value="">请选择收藏夹</option>
                  {zhihuCollections.map((collection) => (
                    <option key={collection.id} value={collection.id}>
                      {collection.title}（{collection.count}）
                    </option>
                  ))}
                </select>
                {zhihuProfile?.isLogin && (
                  <button className="ghost-button" type="button" onClick={loadZhihuCollections}>
                    <RefreshCcw size={15} />
                    刷新收藏夹
                  </button>
                )}
              </div>
            ) : (
              <div className="browser-block">
                <label className="field-label">浏览器书签文件</label>
                <div
                  className="browser-drop-zone"
                  onDragOver={(event) => event.preventDefault()}
                  onDrop={handleBrowserDrop}
                >
                  <Upload size={24} />
                  <p>拖拽书签 HTML 文件到此处，或点击选择文件。</p>
                  <input
                    type="file"
                    accept=".html,.htm"
                    onChange={handleBrowserFileInput}
                    className="browser-file-input"
                  />
                </div>
                {browserFileName && (
                  <div className="parsed-card">
                    <strong>{browserFileName}</strong>
                    <span>{browserItems.length} 条</span>
                  </div>
                )}
              </div>
            )}

            <button
              className="primary-button wide"
              type="button"
              onClick={startPreview}
              disabled={!currentCollection || busy}
            >
              {busy ? <LoaderCircle className="spin" size={17} /> : <FolderDown size={17} />}
              预览并配置标签
            </button>
          </div>
        </div>
      )}

      {step === "tags" && preview && (
        <div className="import-tag-editor">
          <div className="import-editor-toolbar">
            <button className="ghost-button" type="button" onClick={() => setStep("source")}>
              <Undo2 size={16} />
              返回
            </button>
            <div>
              <h2>{preview.collection.title}</h2>
              <p>共 {preview.items.length} 条视频，当前第 {currentPage} / {totalPages} 页。</p>
            </div>
          </div>

          <div className="folder-tag-config">
            <label className="checkbox-line">
              <input
                type="checkbox"
                checked={folderPartitionEnabled}
                onChange={(event) => {
                  setFolderPartitionEnabled(event.target.checked);
                  if (!event.target.checked) return;
                  setPerVideoTags((current) => {
                    const next = { ...current };
                    Object.entries(next).forEach(([externalId, state]) => {
                      if (!state.partitionManuallyEdited) {
                        next[externalId] = {
                          ...state,
                          partitionTag: folderPartitionTag
                        };
                      }
                    });
                    return next;
                  });
                }}
              />
              <span>
                <strong>为整个收藏夹设置分区标签</strong>
                <small>该标签会先应用到所有视频，后续可逐条修改。</small>
              </span>
            </label>
            <div className="folder-partition-input">
              <Tag size={16} />
              <input
                value={folderPartitionTag}
                onChange={(event) => updateFolderPartitionTag(event.target.value)}
                disabled={!folderPartitionEnabled}
                list="folder-partition-options"
                placeholder="例如：知识、科技"
              />
              <datalist id="folder-partition-options">
                {preview.partitionSuggestions.map((item) => (
                  <option key={item.name} value={item.name} />
                ))}
                {tagPool.map((tag) => (
                  <option key={tag.id} value={tag.name} />
                ))}
              </datalist>
            </div>
          </div>

          <div className="per-video-tag-list">
            {visibleItems.map((item) => {
              const state = perVideoTags[item.externalId];
              if (!state) return null;
              return (
                <article className="per-video-tag-card" key={item.externalId}>
                  <div className="video-tag-summary">
                    <div className="preview-cover">
                      {item.coverUrl ? <img src={item.coverUrl} alt="" /> : <span>无封面</span>}
                    </div>
                    <div>
                      <strong>{item.title}</strong>
                      <span>{item.authorName} · {item.partitionName || "未分区"}</span>
                    </div>
                  </div>

                  <div className="video-tag-fields">
                    <label>
                      <span>分区标签</span>
                      <input
                        value={state.partitionTag}
                        onChange={(event) =>
                          updateVideoTag(item.externalId, {
                            partitionTag: event.target.value,
                            partitionManuallyEdited: true
                          })
                        }
                        list="folder-partition-options"
                        placeholder="输入或修改分区标签"
                      />
                    </label>

                  </div>

                  <div className="video-other-tags">
                    <TagPoolInput
                      pool={tagPool}
                      selected={state.otherTags}
                      onAdd={(tag) =>
                        updateVideoTag(item.externalId, {
                          otherTags: [...state.otherTags, tag]
                        })
                      }
                      onRemove={(tag) =>
                        updateVideoTag(item.externalId, {
                          otherTags: state.otherTags.filter((item) => item.id !== tag.id)
                        })
                      }
                      onCreate={createTag}
                      placeholder="检索或新建其他标签"
                    />
                  </div>
                </article>
              );
            })}
          </div>

          <div className="pagination-bar">
            <button
              className="ghost-button"
              type="button"
              disabled={currentPage <= 1}
              onClick={() => setCurrentPage((page) => Math.max(1, page - 1))}
            >
              <ChevronLeft size={16} />
              上一页
            </button>
            <span>{currentPage} / {totalPages}</span>
            <button
              className="ghost-button"
              type="button"
              disabled={currentPage >= totalPages}
              onClick={() => setCurrentPage((page) => Math.min(totalPages, page + 1))}
            >
              下一页
              <ChevronRight size={16} />
            </button>
          </div>

          <div className="import-confirm-row">
            <button className="primary-button" type="button" onClick={execute} disabled={busy}>
              {busy ? <LoaderCircle className="spin" size={17} /> : <Check size={17} />}
              确认导入
            </button>
          </div>
        </div>
      )}

      {step === "done" && result && (
        <div className="result-card">
          <div className="result-icon">
            <Check size={28} />
          </div>
          <h2>导入完成</h2>
          <div className="result-stats">
            <span><strong>{result.total}</strong> 总计</span>
            <span><strong>{result.imported}</strong> 新增</span>
            <span><strong>{result.skipped}</strong> 跳过</span>
            <span><strong>{result.failed}</strong> 失败</span>
          </div>
          {result.errors && result.errors.length > 0 && (
            <div className="result-errors">
              <strong>部分视频导入失败</strong>
              <ul>
                {result.errors.map((message, index) => (
                  <li key={`${index}-${message}`}>{message}</li>
                ))}
              </ul>
            </div>
          )}
          <button
            className="primary-button"
            type="button"
            onClick={() => {
              setStep("source");
              setPreview(null);
              setResult(null);
              setPerVideoTags({});
            }}
          >
            继续导入
          </button>
        </div>
      )}
    </section>
  );
}
