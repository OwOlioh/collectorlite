import { useCallback, useEffect, useMemo, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  ClipboardPaste,
  Code2,
  FolderDown,
  Github,
  Globe,
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

type ImportMode = "login" | "public" | "browser" | "zhihu" | "zhihu_public" | "csdn" | "csdn_public" | "github";
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

  // CSDN state
  const [csdnUsername, setCsdnUsername] = useState("");
  const [csdnCollections, setCsdnCollections] = useState<CollectionInfo[]>([]);
  const [csdnSelectedCollectionId, setCsdnSelectedCollectionId] = useState("");
  const [csdnPublicUrl, setCsdnPublicUrl] = useState("");
  const [csdnParsedCollection, setCsdnParsedCollection] = useState<CollectionInfo | null>(null);

  // GitHub state
  const [githubUsername, setGithubUsername] = useState("");
  const [githubCollections, setGithubCollections] = useState<CollectionInfo[]>([]);

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

  const refreshZhihuProfile = useCallback(async () => {
    try {
      const next = await api.zhihuProfile();
      setZhihuProfile(next);
      if (next.isLogin) {
        setZhihuCollections(await api.listZhihuCollections());
      }
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void refreshProfile();
    void refreshZhihuProfile();
  }, [refreshProfile, refreshZhihuProfile]);

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

  const loadCsdnCollections = async (username: string) => {
    setBusy(true);
    setError("");
    setCsdnSelectedCollectionId("");
    try {
      setCsdnCollections(await api.listCsdnCollections(username));
    } catch (err) {
      setError(String(err));
      setCsdnCollections([]);
    } finally {
      setBusy(false);
    }
  };

  const loadGithubStars = async (username: string) => {
    setBusy(true);
    setError("");
    try {
      setGithubCollections(await api.listGithubStars(username));
    } catch (err) {
      setError(String(err));
      setGithubCollections([]);
    } finally {
      setBusy(false);
    }
  };

  const parseCsdnUrl = async () => {
    if (!csdnPublicUrl.trim()) {
      setError("请先粘贴 CSDN 收藏夹链接。");
      return;
    }
    setBusy(true);
    setError("");
    try {
      setCsdnParsedCollection(await api.parseCsdnCollectionUrl(csdnPublicUrl.trim()));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const currentCollection = useMemo(() => {
    if (mode === "public" || mode === "login") {
      return parsedCollection || collections.find((item) => item.id === selectedCollectionId) || null;
    }
    if (mode === "zhihu_public" || mode === "zhihu") {
      return zhihuParsedCollection || zhihuCollections.find((item) => item.id === zhihuSelectedCollectionId) || null;
    }
    if (mode === "csdn_public" || mode === "csdn") {
      return csdnParsedCollection || csdnCollections.find((item) => item.id === csdnSelectedCollectionId) || null;
    }
    if (mode === "github") {
      return githubCollections[0] || null;
    }
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
    return null;
  }, [mode, parsedCollection, zhihuParsedCollection, csdnParsedCollection, collections, zhihuCollections, csdnCollections, githubCollections, selectedCollectionId, zhihuSelectedCollectionId, csdnSelectedCollectionId, browserItems, browserFileName]);

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
      const isCsdn = mode === "csdn" || mode === "csdn_public";
      const isGithub = mode === "github";
      const isBili = mode === "login" || mode === "public";
      const input = {
        kind: (isBili || isZhihu || isCsdn) && currentCollection?.id && !parsedCollection && !zhihuParsedCollection && !csdnParsedCollection ? ("favorites" as const) : ("public_url" as const),
        mediaId: (isBili || isZhihu || isCsdn) && currentCollection?.id && !parsedCollection && !zhihuParsedCollection && !csdnParsedCollection ? currentCollection.id : undefined,
        url: isBili ? (publicUrl.trim() || undefined) : isZhihu ? (zhihuPublicUrl.trim() || undefined) : isCsdn ? (csdnPublicUrl.trim() || csdnUsername.trim() || undefined) : isGithub ? (githubUsername.trim() || undefined) : undefined,
        tagSpecs: [],
        itemTagAssignments: []
      };
      let next: ImportPreview;
      if (isZhihu) {
        next = await api.previewZhihuImport(input);
      } else if (isCsdn) {
        next = await api.previewCsdnImport(input);
      } else if (isGithub) {
        next = await api.previewGithubImport(input);
      } else {
        next = await api.previewImport(input);
      }
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
      const isCsdn = mode === "csdn" || mode === "csdn_public";
      const isGithub = mode === "github";
      const isBili = mode === "login" || mode === "public";
      const input = {
        kind: (isBili || isZhihu || isCsdn) && currentCollection?.id && !parsedCollection && !zhihuParsedCollection && !csdnParsedCollection ? ("favorites" as const) : ("public_url" as const),
        mediaId: (isBili || isZhihu || isCsdn) && currentCollection?.id && !parsedCollection && !zhihuParsedCollection && !csdnParsedCollection ? currentCollection.id : undefined,
        url: isBili ? (publicUrl.trim() || undefined) : isZhihu ? (zhihuPublicUrl.trim() || undefined) : isCsdn ? (csdnPublicUrl.trim() || csdnUsername.trim() || undefined) : isGithub ? (githubUsername.trim() || undefined) : undefined,
        tagSpecs: [],
        itemTagAssignments: assignments
      };
      let next: ImportResult;
      if (isZhihu) {
        next = await api.executeZhihuImport(input);
      } else if (isCsdn) {
        next = await api.executeCsdnImport(input);
      } else if (isGithub) {
        next = await api.executeGithubImport(input);
      } else {
        next = await api.executeImport(input);
      }
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
          <p>从多个平台导入收藏内容，统一管理标签。</p>
        </div>
      </header>

      {error && <div className="alert">{error}</div>}

      {step === "source" && (
        <div className="import-source-grid">
          <div className="import-modes">
            <button
              type="button"
              className={`import-mode-card ${(mode === "login" || mode === "public") ? "is-active" : ""}`}
              onClick={() => setMode(mode === "login" ? "public" : "login")}
            >
              <LogIn size={20} />
              <strong>B 站收藏</strong>
              <span>扫码登录读取收藏夹，或粘贴公开链接导入。</span>
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
              className={`import-mode-card ${(mode === "zhihu" || mode === "zhihu_public") ? "is-active" : ""}`}
              onClick={() => setMode(mode === "zhihu" ? "zhihu_public" : "zhihu")}
            >
              <LogIn size={20} />
              <strong>知乎收藏</strong>
              <span>登录知乎读取收藏夹，或粘贴链接导入。</span>
            </button>
            <button
              type="button"
              className={`import-mode-card ${(mode === "csdn" || mode === "csdn_public") ? "is-active" : ""}`}
              onClick={() => setMode(mode === "csdn" ? "csdn_public" : "csdn")}
            >
              <Code2 size={20} />
              <strong>CSDN 收藏</strong>
              <span>输入用户名读取收藏夹，或粘贴链接导入。</span>
            </button>
            <button
              type="button"
              className={`import-mode-card ${mode === "github" ? "is-active" : ""}`}
              onClick={() => setMode("github")}
            >
              <Github size={20} />
              <strong>GitHub Stars</strong>
              <span>输入 GitHub 用户名即可导入 Star 仓库列表。</span>
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
                <div className="import-section-divider" />
                <label className="field-label">或者粘贴公开收藏夹链接</label>
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
            ) : mode === "zhihu" || mode === "zhihu_public" ? (
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
                        点击下方按钮在浏览器中打开知乎并登录。登录后按 F12 →
                        Application → Cookies → zhihu.com →
                        分别复制 <strong>z_c0</strong> 和 <strong>d_c0</strong> 的值，
                        用分号拼接粘贴到下方，格式：<code>z_c0=xxx; d_c0=xxx</code>
                      </p>
                      <button
                        type="button"
                        className="secondary-button"
                        style={{ justifySelf: "start" }}
                        onClick={() => api.openUrl("https://www.zhihu.com/signin")}
                      >
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
                                setZhihuProfile(p);
                                if (p.isLogin) {
                                  setZhihuCollections(await api.listZhihuCollections());
                                }
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
                <div className="import-section-divider" />
                <label className="field-label">或者粘贴收藏夹链接</label>
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
            ) : mode === "csdn" || mode === "csdn_public" ? (
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
                      <input
                        value={csdnUsername}
                        onChange={(event) => setCsdnUsername(event.target.value)}
                        placeholder="CSDN 英文用户名，如 LOVEmy134611"
                        onKeyDown={(event) => {
                          if (event.key === "Enter" && csdnUsername.trim()) {
                            void loadCsdnCollections(csdnUsername.trim());
                          }
                        }}
                      />
                      <button
                        className="secondary-button"
                        type="button"
                        onClick={() => csdnUsername.trim() && loadCsdnCollections(csdnUsername.trim())}
                        disabled={busy || !csdnUsername.trim()}
                      >
                        {busy ? <LoaderCircle className="spin" size={16} /> : <RefreshCcw size={16} />}
                        获取收藏夹
                      </button>
                    </div>
                  </div>
                </div>
                <label className="field-label">选择收藏夹</label>
                <select
                  className="select-control full"
                  value={csdnSelectedCollectionId}
                  onChange={(event) => setCsdnSelectedCollectionId(event.target.value)}
                  disabled={csdnCollections.length === 0}
                >
                  <option value="">请选择收藏夹</option>
                  {csdnCollections.map((collection) => (
                    <option key={collection.id} value={collection.id}>
                      {collection.title}（{collection.count}）
                    </option>
                  ))}
                </select>
                {csdnCollections.length > 0 && (
                  <button
                    className="ghost-button"
                    type="button"
                    onClick={() => csdnUsername.trim() && loadCsdnCollections(csdnUsername.trim())}
                  >
                    <RefreshCcw size={15} />
                    刷新收藏夹
                  </button>
                )}
                <div className="import-section-divider" />
                <label className="field-label">或者粘贴收藏夹链接</label>
                <div className="input-with-button">
                  <input
                    value={csdnPublicUrl}
                    onChange={(event) => setCsdnPublicUrl(event.target.value)}
                    placeholder="https://blog.csdn.net/用户名/favorites?folderId=123"
                  />
                  <button className="secondary-button" type="button" onClick={parseCsdnUrl} disabled={busy}>
                    <ClipboardPaste size={16} />
                    解析
                  </button>
                </div>
                {csdnParsedCollection && (
                  <div className="parsed-card">
                    <strong>{csdnParsedCollection.title}</strong>
                    <span>{csdnParsedCollection.owner || "公开用户"}</span>
                    <span>{csdnParsedCollection.count} 条</span>
                  </div>
                )}
              </div>
            ) : mode === "github" ? (
              <div className="login-block">
                <div className="account-line">
                  <div style={{ display: "grid", gap: "8px", width: "100%" }}>
                    <p style={{ margin: 0, color: "var(--muted)", fontSize: "13px" }}>
                      GitHub Stars 列表是公开的，无需登录。输入你的 GitHub <strong>用户名</strong> 即可获取。
                    </p>
                    <div className="input-with-button">
                      <input
                        value={githubUsername}
                        onChange={(event) => setGithubUsername(event.target.value)}
                        placeholder="GitHub 用户名，如 OwOlioh"
                        onKeyDown={(event) => {
                          if (event.key === "Enter" && githubUsername.trim()) {
                            void loadGithubStars(githubUsername.trim());
                          }
                        }}
                      />
                      <button
                        className="secondary-button"
                        type="button"
                        onClick={() => githubUsername.trim() && loadGithubStars(githubUsername.trim())}
                        disabled={busy || !githubUsername.trim()}
                      >
                        {busy ? <LoaderCircle className="spin" size={16} /> : <RefreshCcw size={16} />}
                        获取 Stars
                      </button>
                    </div>
                  </div>
                </div>
                {githubCollections.length > 0 && (
                  <div className="parsed-card">
                    <strong>{githubCollections[0].title}</strong>
                    <span>{githubCollections[0].owner}</span>
                  </div>
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
