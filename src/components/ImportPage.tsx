import { useCallback, useEffect, useMemo, useState } from "react";
import { Code2, FolderDown, Github, Globe, LoaderCircle, LogIn, Upload } from "lucide-react";
import { api } from "../lib/api";
import type {
  BilibiliProfile, BrowserImportRequest, CollectionInfo, ImportPreview,
  ImportResult, ItemTagAssignment, QrSession, Tag as AppTag, VideoItem
} from "../types";
import { BilibiliForm } from "./import/BilibiliForm";
import { ZhihuForm } from "./import/ZhihuForm";
import { CsdnForm } from "./import/CsdnForm";
import { GithubForm } from "./import/GithubForm";
import { BrowserForm } from "./import/BrowserForm";
import { TagEditor } from "./import/TagEditor";
import { ResultCard } from "./import/ResultCard";
import { useToast } from "./Toast";

type ImportMode = "login" | "public" | "browser" | "zhihu" | "zhihu_public" | "csdn" | "csdn_public" | "github" | "file";
type ImportStep = "source" | "tags" | "done";

interface ImportPageProps {
  tagPool: AppTag[];
  onTagsChanged: () => void;
}

export function ImportPage({ tagPool, onTagsChanged }: ImportPageProps) {
  const [mode, setMode] = useState<ImportMode>("login");
  const [step, setStep] = useState<ImportStep>("source");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [result, setResult] = useState<ImportResult | null>(null);
  const { toast } = useToast();

  // B站
  const [profile, setProfile] = useState<BilibiliProfile | null>(null);
  const [qr, setQr] = useState<QrSession | null>(null);
  const [loginBusy, setLoginBusy] = useState(false);
  const [collections, setCollections] = useState<CollectionInfo[]>([]);
  const [selectedCollectionId, setSelectedCollectionId] = useState("");
  const [publicUrl, setPublicUrl] = useState("");
  const [parsedCollection, setParsedCollection] = useState<CollectionInfo | null>(null);

  // 知乎
  const [zhihuProfile, setZhihuProfile] = useState<BilibiliProfile | null>(null);
  const [zhihuCollections, setZhihuCollections] = useState<CollectionInfo[]>([]);
  const [zhihuSelectedCollectionId, setZhihuSelectedCollectionId] = useState("");
  const [zhihuPublicUrl, setZhihuPublicUrl] = useState("");
  const [zhihuParsedCollection, setZhihuParsedCollection] = useState<CollectionInfo | null>(null);

  // CSDN
  const [csdnUsername, setCsdnUsername] = useState("");
  const [csdnCollections, setCsdnCollections] = useState<CollectionInfo[]>([]);
  const [csdnSelectedCollectionId, setCsdnSelectedCollectionId] = useState("");
  const [csdnPublicUrl, setCsdnPublicUrl] = useState("");
  const [csdnParsedCollection, setCsdnParsedCollection] = useState<CollectionInfo | null>(null);

  // GitHub
  const [githubUsername, setGithubUsername] = useState("");
  const [githubCollections, setGithubCollections] = useState<CollectionInfo[]>([]);

  // 浏览器
  const [browserHtmlContent, setBrowserHtmlContent] = useState("");
  const [browserFileName, setBrowserFileName] = useState("");
  const [browserItems, setBrowserItems] = useState<VideoItem[]>([]);

  // ── B站 ──
  const loadCollections = useCallback(async () => {
    setBusy(true); setError("");
    try { setCollections(await api.listBilibiliFavorites()); }
    catch (err) { setError(String(err)); }
    finally { setBusy(false); }
  }, []);

  const refreshProfile = useCallback(async () => {
    try {
      const next = await api.getProfile();
      setProfile(next);
      if (next.isLogin) await loadCollections();
    } catch (err) { const msg = String(err); setError(msg); toast("error", msg); }
  }, [loadCollections]);

  const startQr = async () => {
    setLoginBusy(true); setError("");
    try { setQr(await api.startQrLogin()); }
    catch (err) { setError(String(err)); }
    finally { setLoginBusy(false); }
  };

  const parsePublic = async () => {
    if (!publicUrl.trim()) { setError("请先粘贴公开收藏夹链接。"); return; }
    setBusy(true); setError("");
    try { setParsedCollection(await api.parsePublicFavoriteUrl(publicUrl.trim())); }
    catch (err) { setError(String(err)); }
    finally { setBusy(false); }
  };

  useEffect(() => { void refreshProfile(); }, [refreshProfile]);

  useEffect(() => {
    if (!qr) return;
    let stopped = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const status = await api.pollQrLogin(qr.qrcodeKey);
        if (status.code === 0) { setQr(null); setProfile(status.profile || null); await loadCollections(); return; }
        if (status.code === 86038) { setQr(null); setError("二维码已失效，请重新生成。"); return; }
      } catch (err) { setError(String(err)); setQr(null); return; }
      if (!stopped) timer = window.setTimeout(poll, 1800);
    };
    timer = window.setTimeout(poll, 1200);
    return () => { stopped = true; if (timer) window.clearTimeout(timer); };
  }, [qr, loadCollections]);

  // ── 知乎 ──
  const refreshZhihuProfile = useCallback(async () => {
    try {
      const next = await api.zhihuProfile();
      setZhihuProfile(next);
      if (next.isLogin) setZhihuCollections(await api.listZhihuCollections());
    } catch (err) { const msg = String(err); setError(msg); toast("error", msg); }
  }, []);

  const loadZhihuCollections = async () => {
    setBusy(true); setError("");
    try { setZhihuCollections(await api.listZhihuCollections()); }
    catch (err) { setError(String(err)); }
    finally { setBusy(false); }
  };

  const parseZhihu = async () => {
    if (!zhihuPublicUrl.trim()) { setError("请先粘贴知乎收藏夹链接。"); return; }
    setBusy(true); setError("");
    try { setZhihuParsedCollection(await api.parseZhihuCollectionUrl(zhihuPublicUrl.trim())); }
    catch (err) { setError(String(err)); }
    finally { setBusy(false); }
  };

  useEffect(() => { void refreshZhihuProfile(); }, [refreshZhihuProfile]);

  // ── CSDN ──
  const loadCsdnCollections = async () => {
    setBusy(true); setError(""); setCsdnSelectedCollectionId("");
    try { setCsdnCollections(await api.listCsdnCollections(csdnUsername.trim())); }
    catch (err) { setError(String(err)); setCsdnCollections([]); }
    finally { setBusy(false); }
  };

  const parseCsdnUrl = async () => {
    if (!csdnPublicUrl.trim()) { setError("请先粘贴 CSDN 收藏夹链接。"); return; }
    setBusy(true); setError("");
    try { setCsdnParsedCollection(await api.parseCsdnCollectionUrl(csdnPublicUrl.trim())); }
    catch (err) { setError(String(err)); }
    finally { setBusy(false); }
  };

  // ── GitHub ──
  const loadGithubStars = async () => {
    setBusy(true); setError("");
    try { setGithubCollections(await api.listGithubStars(githubUsername.trim())); }
    catch (err) { setError(String(err)); setGithubCollections([]); }
    finally { setBusy(false); }
  };

  // ── 浏览器 ──
  const parseBrowserHtmlLocally = async (html: string): Promise<VideoItem[]> => {
    const parser = new DOMParser();
    const doc = parser.parseFromString(html, "text/html");
    const items: VideoItem[] = [];
    let index = 0;
    const hashUrl = async (url: string): Promise<string> => {
      const data = new TextEncoder().encode(url);
      const hashBuffer = await crypto.subtle.digest("SHA-256", data);
      return "bk_" + Array.from(new Uint8Array(hashBuffer)).slice(0, 8).map(b => b.toString(16).padStart(2, "0")).join("");
    };
    const walk = async (node: Element, folderPath: string) => {
      for (const child of Array.from(node.children)) {
        if (child.tagName === "DT") {
          const h3 = child.querySelector(":scope > H3");
          const a = child.querySelector(":scope > A");
          if (h3) {
            const folderName = h3.textContent?.trim() || "";
            const dl = child.querySelector(":scope > DL");
            if (dl) await walk(dl, folderPath ? `${folderPath} / ${folderName}` : folderName);
          } else if (a) {
            const href = a.getAttribute("href") || "";
            const title = a.textContent?.trim() || "";
            const addDate = a.getAttribute("add_date");
            const icon = a.getAttribute("icon");
            index += 1;
            items.push({
              id: -index, source: "browser", externalId: await hashUrl(href),
              sourceUrl: href, title: title || href, description: "",
              coverUrl: icon || undefined, authorName: undefined, partitionName: undefined,
              favoriteTime: addDate ? parseInt(addDate, 10) : undefined,
              tags: [], duration: undefined, publishedAt: undefined, authorId: undefined,
              coverLocalPath: undefined, notes: undefined
            });
          }
        }
        if (child.tagName === "DL") await walk(child, folderPath);
      }
    };
    const body = doc.querySelector("body");
    if (body) { const dl = body.querySelector("DL"); if (dl) await walk(dl, ""); }
    return items;
  };

  const handleBrowserFile = (file: File) => {
    setBrowserFileName(file.name);
    const reader = new FileReader();
    reader.onload = async () => {
      setBrowserHtmlContent(reader.result as string);
      setBrowserItems(await parseBrowserHtmlLocally(reader.result as string));
      setError("");
    };
    reader.onerror = () => setError("读取文件失败。");
    reader.readAsText(file);
  };

  // ── currentCollection ──
  const currentCollection = useMemo(() => {
    if (mode === "public" || mode === "login")
      return parsedCollection || collections.find((c) => c.id === selectedCollectionId) || null;
    if (mode === "zhihu_public" || mode === "zhihu")
      return zhihuParsedCollection || zhihuCollections.find((c) => c.id === zhihuSelectedCollectionId) || null;
    if (mode === "csdn_public" || mode === "csdn")
      return csdnParsedCollection || csdnCollections.find((c) => c.id === csdnSelectedCollectionId) || null;
    if (mode === "github") return githubCollections[0] || null;
    if (mode === "browser" && browserItems.length > 0)
      return { source: "browser", id: "browser-bookmarks", title: browserFileName || "浏览器书签", owner: undefined, count: browserItems.length, url: undefined } as CollectionInfo;
    return null;
  }, [mode, parsedCollection, zhihuParsedCollection, csdnParsedCollection, collections, zhihuCollections, csdnCollections, githubCollections, selectedCollectionId, zhihuSelectedCollectionId, csdnSelectedCollectionId, browserItems, browserFileName]);

  // ── 预览 ──
  const startPreview = async () => {
    if (!currentCollection) { setError("请先选择或解析一个收藏夹。"); return; }
    setBusy(true); setError("");

    if (mode === "browser") {
      setPreview({ collection: currentCollection, items: browserItems, partitionSuggestions: [] });
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
        tagSpecs: [], itemTagAssignments: []
      };
      let next: ImportPreview;
      if (isZhihu) next = await api.previewZhihuImport(input);
      else if (isCsdn) next = await api.previewCsdnImport(input);
      else if (isGithub) next = await api.previewGithubImport(input);
      else next = await api.previewImport(input);
      setPreview(next);
      setStep("tags");
    } catch (err) { const msg = String(err); setError(msg); toast("error", msg); }
    finally { setBusy(false); }
  };

  // ── 执行 ──
  const buildImportInput = (assignments: ItemTagAssignment[]) => {
    const isZhihu = mode === "zhihu" || mode === "zhihu_public";
    const isCsdn = mode === "csdn" || mode === "csdn_public";
    const isGithub = mode === "github";
    const isBili = mode === "login" || mode === "public";
    const input = {
      kind: (isBili || isZhihu || isCsdn) && currentCollection?.id && !parsedCollection && !zhihuParsedCollection && !csdnParsedCollection ? ("favorites" as const) : ("public_url" as const),
      mediaId: (isBili || isZhihu || isCsdn) && currentCollection?.id && !parsedCollection && !zhihuParsedCollection && !csdnParsedCollection ? currentCollection.id : undefined,
      url: isBili ? (publicUrl.trim() || undefined) : isZhihu ? (zhihuPublicUrl.trim() || undefined) : isCsdn ? (csdnPublicUrl.trim() || csdnUsername.trim() || undefined) : isGithub ? (githubUsername.trim() || undefined) : undefined,
      tagSpecs: [], itemTagAssignments: assignments
    };
    if (isZhihu) return { apiCall: () => api.executeZhihuImport(input) };
    if (isCsdn) return { apiCall: () => api.executeCsdnImport(input) };
    if (isGithub) return { apiCall: () => api.executeGithubImport(input) };
    return { apiCall: () => api.executeImport(input) };
  };

  const executeBrowser = async (assignments: ItemTagAssignment[]) => {
    const request: BrowserImportRequest = { htmlContent: browserHtmlContent, tagSpecs: [], itemTagAssignments: assignments };
    return api.importBrowserBookmarks(request);
  };

  const handleExecute = async (assignments: ItemTagAssignment[]) => {
    if (mode === "browser") {
      return executeBrowser(assignments);
    }
    const input = buildImportInput(assignments);
    if (!input) throw new Error("无法构建导入请求");
    return input.apiCall();
  };

  const handleResult = (r: ImportResult) => {
    setResult(r);
    setStep("done");
    if (r.failed > 0) {
      toast("error", `导入完成：${r.imported} 条新增，${r.failed} 条失败`);
    } else {
      toast("success", `导入成功：${r.imported} 条已加入收藏库`);
    }
    window.setTimeout(() => {
      setStep("source"); setPreview(null); setResult(null);
      if (mode === "browser") { setBrowserHtmlContent(""); setBrowserFileName(""); setBrowserItems([]); }
    }, 1200);
  };

  // ── 从文件导入 ──
  const importFromFile = async (file: File) => {
    setBusy(true); setError("");
    try {
      const text = await file.text();
      const result = await api.importCollection(text);
      setResult(result);
      setStep("done");
      const parts = [`新增 ${result.imported} 条`, `跳过重复 ${result.skipped} 条`];
      if (result.failed > 0) parts.push(`失败 ${result.failed} 条`);
      const summary = `导入完成：${parts.join("，")}`;
      if (result.failed > 0) toast("error", summary);
      else toast("success", summary);

      // 导入后自动补一次封面缓存，覆盖新导入项以及库中历史遗留的缺封面项，
      // 保证封面都能正常显示（B站/CSDN 会下载到本地，其余来源走远程 https 封面）。
      void api
        .recacheCovers()
        .then((recache) => {
          if (recache.cached > 0) {
            toast("info", `已自动缓存 ${recache.cached} 张封面`);
          }
        })
        .catch(() => {
          // 封面缓存失败不阻断导入结果，静默忽略
        });

      onTagsChanged();
    } catch (err) {
      const msg = String(err);
      setError(msg);
      toast("error", msg);
    } finally {
      setBusy(false);
    }
  };

  // ── 渲染 ──
  return (
    <section className="page import-page">
      <header className="page-header">
        <div><h1>导入收藏内容</h1><p>从多个平台导入收藏内容，统一管理标签。</p></div>
      </header>

      {error && <div className="alert">{error}</div>}

      {step === "source" && (
        <div className="import-source-grid">
          <div className="import-modes">
            <SourceCard icon={<LogIn size={20} />} title="B 站收藏" desc="扫码登录读取收藏夹，或粘贴公开链接导入。"
              active={mode === "login" || mode === "public"} onClick={() => setMode(mode === "login" ? "public" : "login")} />
            <SourceCard icon={<Globe size={20} />} title="浏览器书签" desc="从浏览器导出的书签 HTML 文件中导入链接。"
              active={mode === "browser"} onClick={() => setMode("browser")} />
            <SourceCard icon={<LogIn size={20} />} title="知乎收藏" desc="登录知乎读取收藏夹，或粘贴链接导入。"
              active={mode === "zhihu" || mode === "zhihu_public"} onClick={() => setMode(mode === "zhihu" ? "zhihu_public" : "zhihu")} />
            <SourceCard icon={<Code2 size={20} />} title="CSDN 收藏" desc="输入用户名读取收藏夹，或粘贴链接导入。"
              active={mode === "csdn" || mode === "csdn_public"} onClick={() => setMode(mode === "csdn" ? "csdn_public" : "csdn")} />
            <SourceCard icon={<Github size={20} />} title="GitHub Stars" desc="输入 GitHub 用户名即可导入 Star 仓库列表。"
              active={mode === "github"} onClick={() => setMode("github")} />
            <SourceCard icon={<Upload size={20} />} title="从文件导入" desc="导入此前导出的收藏 JSON 文件，仅新增不覆盖。"
              active={mode === "file"} onClick={() => setMode("file")} />
          </div>

          <div className="import-form-panel">
            {mode === "login" ? (
              <BilibiliForm busy={busy} setBusy={setBusy} setError={setError}
                profile={profile} setProfile={setProfile} collections={collections} setCollections={setCollections}
                selectedCollectionId={selectedCollectionId} setSelectedCollectionId={setSelectedCollectionId}
                publicUrl={publicUrl} setPublicUrl={setPublicUrl} parsedCollection={parsedCollection}
                setParsedCollection={setParsedCollection} qr={qr} setQr={setQr}
                loginBusy={loginBusy} setLoginBusy={setLoginBusy}
                loadCollections={loadCollections} startQr={startQr} parsePublic={parsePublic} />
            ) : mode === "zhihu" || mode === "zhihu_public" ? (
              <ZhihuForm busy={busy} setBusy={setBusy} setError={setError} setLoginBusy={setLoginBusy}
                profile={zhihuProfile} setProfile={setZhihuProfile} collections={zhihuCollections}
                setCollections={setZhihuCollections} selectedCollectionId={zhihuSelectedCollectionId}
                setSelectedCollectionId={setZhihuSelectedCollectionId} publicUrl={zhihuPublicUrl}
                setPublicUrl={setZhihuPublicUrl} parsedCollection={zhihuParsedCollection}
                setParsedCollection={setZhihuParsedCollection} loadCollections={loadZhihuCollections}
                parseUrl={parseZhihu} />
            ) : mode === "csdn" || mode === "csdn_public" ? (
              <CsdnForm busy={busy} username={csdnUsername} setUsername={setCsdnUsername}
                collections={csdnCollections} selectedCollectionId={csdnSelectedCollectionId}
                setSelectedCollectionId={setCsdnSelectedCollectionId} publicUrl={csdnPublicUrl}
                setPublicUrl={setCsdnPublicUrl} parsedCollection={csdnParsedCollection}
                onLoadCollections={loadCsdnCollections} onParseUrl={parseCsdnUrl} />
            ) : mode === "github" ? (
              <GithubForm busy={busy} username={githubUsername} setUsername={setGithubUsername}
                collections={githubCollections} onLoadStars={loadGithubStars} />
            ) : mode === "file" ? (
              <FileImportForm onFile={importFromFile} busy={busy} />
            ) : (
              <BrowserForm browserFileName={browserFileName} browserItems={browserItems}
                onFileDrop={(e) => { e.preventDefault(); const f = e.dataTransfer.files[0]; if (f) handleBrowserFile(f); }}
                onFileInput={(e) => { const f = e.target.files?.[0]; if (f) handleBrowserFile(f); }} />
            )}

            {mode !== "file" && (
              <button className="primary-button wide" type="button" onClick={startPreview}
                disabled={!currentCollection || busy}>
                {busy ? <LoaderCircle className="spin" size={17} /> : <FolderDown size={17} />}
                预览并配置标签
              </button>
            )}
          </div>
        </div>
      )}

      {step === "tags" && preview && (
        <TagEditor preview={preview} tagPool={tagPool} onTagsChanged={onTagsChanged}
          onBack={() => setStep("source")}
          onExecute={handleResult}
          buildImportInput={(assignments: ItemTagAssignment[]) => {
            if (mode === "browser") return { apiCall: () => executeBrowser(assignments) };
            return buildImportInput(assignments);
          }} />
      )}

      {step === "done" && result && (
        <ResultCard result={result} onContinue={() => { setStep("source"); setPreview(null); setResult(null); }} />
      )}
    </section>
  );
}

function SourceCard({ icon, title, desc, active, onClick }: {
  icon: React.ReactNode; title: string; desc: string; active: boolean; onClick: () => void;
}) {
  return (
    <button type="button" className={`import-mode-card ${active ? "is-active" : ""}`} onClick={onClick}>
      {icon}
      <strong>{title}</strong>
      <span>{desc}</span>
    </button>
  );
}

function FileImportForm({ onFile, busy }: { onFile: (file: File) => void; busy: boolean }) {
  return (
    <div className="file-import-form">
      <p>
        选择一个此前从本应用导出的收藏 <code>.json</code> 文件。
        导入时只会<strong>新增不存在的收藏</strong>，已存在的项会被跳过、不会被覆盖；标签会自动合并。
      </p>
      <label className="file-drop">
        <input
          type="file"
          accept="application/json,.json"
          disabled={busy}
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) onFile(file);
          }}
        />
        <span>{busy ? "导入中..." : "点击选择 .json 文件"}</span>
      </label>
    </div>
  );
}