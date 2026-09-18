// Edge 侧边栏：读当前页 → 收藏 / 笔记两个视图 → 通过本地桥写入「collectorlite」。
//
// 桥只监听 127.0.0.1，端口在 17820–17829 之间顺延，扩展按顺序探测
// （扩展读不到本地文件，所以不能靠读端口文件）。
//
// 两个视图共享同一条收藏：
//   「收藏」= 标题 + 标签（提交到 /capture）
//   「批注」= 轻量批注（提交到 /annotation，独立于 items.notes，绝不进 Obsidian）
//   「笔记」= items.notes（提交到 /note，带乐观锁，可选择性地联动到 Obsidian）
// 批注与笔记是两个**互相独立**的字段，切页不会互相搬运内容：
// 侧边栏批注 ↔ 应用内每条收藏下方的「批注按钮」共用同一份，天然同步。

const PORT_START = 17820;
const PORT_END = 17829;
const AUTOSAVE_MS = 1200;
// 轮询间隔（第 4 步）：桥是纯请求式的，没有 SSE / WebSocket / 文件监听，
// 应用侧改了批注或笔记，侧边栏只能靠定时回拉 /item 才能发现。
// 8 s 是「手感够及时」与「别拿本地请求刷屏」之间的折中。
const POLL_MS = 8000;
// 轮询定时器句柄。放在顶部声明，避免模块底部 `let` 的 TDZ 风险
// （visibilitychange 可能在脚本后半段执行前就被触发）。
let pollTimer = null;

const state = {
  token: '',
  port: null,
  pool: [],
  selected: [],
  page: null,
  tabId: null,
  exists: false,
  item: null, // { id, source, title, notes, annotation, tags, obsidianPath, updatedAt }
  view: 'collect',
  mode: 'notebook', // 'annotation' | 'notebook'
  noteMode: 'edit', // 'edit' | 'preview'
  conflict: null, // { remoteNotes, remoteUpdatedAt }
  obsidian: null, // { enabled, vaultPath }
  savedAt: null,
  dirty: false, // 笔记（items.notes）是否有未落盘改动
  anDirty: false, // 批注（annotations）是否有未落盘改动 —— 与笔记互相独立
  // 离线缓存（C 方案）：桥不可达时把收藏攒在本地，恢复后自动补录。
  online: false, // 最近一次桥请求是否成功
  flushing: false, // 正在回放离线队列，避免重入
  flushTimer: null, // 后台定期尝试回放的定时器
  // 自定义关联笔记（双向同步）
  linkTimer: null, // 关联搜索框的输入防抖
  linkLoading: false,
};

const els = {};

document.addEventListener('DOMContentLoaded', () => {
  els.status = document.getElementById('status');
  els.title = document.getElementById('title');
  els.tagInput = document.getElementById('tag-input');
  els.selected = document.getElementById('selected');
  els.pool = document.getElementById('pool');
  els.note = document.getElementById('note');
  els.save = document.getElementById('save');
  els.url = document.getElementById('url');
  els.openOptions = document.getElementById('open-options');
  els.tabCollect = document.getElementById('tab-collect');
  els.tabNote = document.getElementById('tab-note');
  els.viewCollect = document.getElementById('view-collect');
  els.viewNote = document.getElementById('view-note');
  els.nbLocked = document.getElementById('nb-locked');
  els.nbOpen = document.getElementById('nb-open');
  els.conflict = document.getElementById('conflict');
  els.goCollect = document.getElementById('go-collect');
  els.mkTime = document.getElementById('mk-time');
  els.mkPos = document.getElementById('mk-pos');
  els.mkList = document.getElementById('mk-list');
  els.mkCount = document.getElementById('mk-count');
  els.mkPreview = document.getElementById('mk-preview');
  els.notePreview = document.getElementById('note-preview');
  els.openObsidian = document.getElementById('open-obsidian');
  els.linkObsidian = document.getElementById('link-obsidian');
  els.syncObsidian = document.getElementById('sync-obsidian');
  els.unlinkObsidian = document.getElementById('unlink-obsidian');
  els.linkedNote = document.getElementById('linked-note');
  els.linkedPath = document.getElementById('linked-path');
  els.linkPanel = document.getElementById('link-panel');
  els.linkSearch = document.getElementById('link-search');
  els.linkList = document.getElementById('link-list');
  els.linkHint = document.getElementById('link-hint');
  els.linkClose = document.getElementById('link-close');
  els.saveDot = document.getElementById('save-dot');
  els.saveText = document.getElementById('save-text');
  els.cfOverwrite = document.getElementById('cf-overwrite');
  els.cfReload = document.getElementById('cf-reload');
  els.tabAnnotate = document.getElementById('tab-annotate');
  els.tabNotebook = document.getElementById('tab-notebook');
  els.annotatePane = document.getElementById('annotate-pane');
  els.notebookPane = document.getElementById('notebook-pane');
  els.annotation = document.getElementById('annotation');
  els.anTime = document.getElementById('an-time');
  els.anPos = document.getElementById('an-pos');
  els.anList = document.getElementById('an-list');
  els.anCount = document.getElementById('an-count');
  els.anDot = document.getElementById('an-dot');
  els.anText = document.getElementById('an-text');
  els.noteSetup = document.getElementById('note-setup');
  els.noteEditor = document.getElementById('note-editor');
  els.nbCreate = document.getElementById('nb-create');
  els.nbLink = document.getElementById('nb-link');

  bind();
  init();
});

function bind() {
  els.tagInput.addEventListener('keydown', onTagInput);
  els.save.addEventListener('click', onSave);
  els.openOptions.addEventListener('click', () => chrome.runtime.openOptionsPage());
  els.tabCollect.addEventListener('click', () => setView('collect'));
  els.tabNote.addEventListener('click', () => setView('note'));
  els.goCollect.addEventListener('click', () => {
    setView('collect');
    els.title.focus();
    els.title.select();
  });
  els.mkTime.addEventListener('click', () => insertTimestampInto(els.note));
  els.mkPos.addEventListener('click', () => insertPositionInto(els.note));
  els.anTime.addEventListener('click', () => insertTimestampInto(els.annotation));
  els.anPos.addEventListener('click', () => insertPositionInto(els.annotation));
  els.tabAnnotate.addEventListener('click', () => setMode('annotation'));
  els.tabNotebook.addEventListener('click', () => setMode('notebook'));
  els.nbCreate.addEventListener('click', createNote);
  els.nbLink.addEventListener('click', openLinkPanel);
  els.mkPreview.addEventListener('click', () =>
    setNoteMode(state.noteMode === 'edit' ? 'preview' : 'edit'),
  );
  els.openObsidian.addEventListener('click', openInObsidian);
  els.linkObsidian.addEventListener('click', openLinkPanel);
  els.linkClose.addEventListener('click', closeLinkPanel);
  els.syncObsidian.addEventListener('click', syncNow);
  els.unlinkObsidian.addEventListener('click', unlinkNote);
  els.linkSearch.addEventListener('input', () => {
    clearTimeout(state.linkTimer);
    state.linkTimer = setTimeout(() => loadLinkCandidates(els.linkSearch.value), 250);
  });
  els.cfOverwrite.addEventListener('click', () => {
    // 用服务端当前值当基准再发一次，等于「我确认覆盖」
    if (state.conflict) {
      state.item = { ...state.item, updatedAt: state.conflict.remoteUpdatedAt };
    }
    hideConflict();
    saveNote();
  });
  els.cfReload.addEventListener('click', () => {
    if (!state.conflict) return;
    els.note.value = state.conflict.remoteNotes || '';
    state.item = {
      ...state.item,
      notes: els.note.value,
      updatedAt: state.conflict.remoteUpdatedAt,
    };
    hideConflict();
    state.dirty = false;
    renderMarkers();
    if (state.noteMode === 'preview') renderPreview();
    setSaved('已保存');
  });

  els.note.addEventListener('input', () => {
    renderMarkers();
    markDirty();
  });
  els.annotation.addEventListener('input', () => {
    renderAnnotationMarkers();
    markAnDirty();
  });

  // 侧边栏随时会被卸载（切站点、关面板）—— 走之前把没保存的内容推出去。
  // MV3 下普通 fetch 会被杀掉，keepalive 才能发出去；token 走查询参数，
  // 这样将来即使降级到 sendBeacon 也不会丢鉴权。
  // 离线模式下没有条目可定位，把未保存的笔记直接攒进离线队列。
  const flush = () => {
    if (!state.online && state.dirty && state.page) {
      enqueueOffline({
        type: 'note',
        payload: {
          url: state.page.url,
          note: els.note.value,
          baseUpdatedAt: state.item ? state.item.updatedAt : null,
          title: state.page.title || state.page.url,
        },
      });
      state.dirty = false;
    }
    flushSave();
  };
  window.addEventListener('pagehide', flush);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') {
      flush();
      // 不可见时停掉轮询：用户看不到，请求纯属白费。
      stopPollTimer();
    } else {
      // 转回可见：立刻补一次，让「别处的改动」不用干等下一个 8 s。
      void pollSync();
      startPollTimer();
    }
  });
}

async function init() {
  const stored = await chrome.storage.local.get(['bridgeToken', 'bridgePort', 'sidebarView']);
  state.token = (stored.bridgeToken || '').trim();
  state.port = stored.bridgePort || null;
  state.view = stored.sidebarView === 'note' ? 'note' : 'collect';
  state.noteMode = stored.sidebarNoteMode === 'preview' ? 'preview' : 'edit';
  state.mode = stored.sidebarMode === 'annotation' ? 'annotation' : 'notebook';

  if (!state.token) {
    setStatus('请先在扩展选项页填写本机令牌', 'error');
    setView(state.view);
    disableForm(true);
    return;
  }

  // 试探桥：成功 = 在线；失败 = 进入离线缓存模式（收藏先攒本地，应用启动后自动补录）。
  let reachable = true;
  try {
    await bridgeFetch('/ping');
  } catch (error) {
    reachable = false;
  }
  state.online = reachable;

  setView(state.view);
  await loadPage();
  startFlushTimer();
  // 轮询同步（第 4 步）：让应用侧的改动能流回侧边栏。
  startPollTimer();

  if (reachable) {
    // 桥刚上线：把之前攒在本地、还没补录的收藏回放一遍，再刷新页面状态。
    const flushed = await flushOffline();
    if (flushed > 0) await loadPage();
  } else {
    setStatus(
      '应用未启动：已进入离线缓存模式，收藏会先存在本地，应用启动后自动补录',
      'info',
    );
  }

  // 页面切换 / 刷新时热更新（仅对侧边栏当前对应的那个标签页响应）。
  chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
    if (changeInfo.status === 'complete' && tabId === state.tabId) {
      reload();
    }
  });
  chrome.tabs.onActivated.addListener((activeInfo) => {
    if (activeInfo.tabId !== state.tabId) {
      reload();
    }
  });
}

async function reload() {
  flushSave();
  await loadPage();
}

// ── 与本地桥通信 ──

async function bridgeFetch(path, options = {}) {
  const headers = new Headers(options.headers || {});
  headers.set('X-Bridge-Token', state.token);
  if (options.body) {
    headers.set('Content-Type', 'application/json');
  }

  // 先试上次成功的端口，再扫全区间，避免每次都探测 10 个端口。
  const ports = state.port
    ? [state.port, ...portRange().filter((port) => port !== state.port)]
    : portRange();

  let lastError = new Error('无法连接到collectorlite');
  for (const port of ports) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}${path}`, {
        ...options,
        headers,
      });
      if (response.status === 200) {
        state.online = true;
        if (state.port !== port) {
          state.port = port;
          chrome.storage.local.set({ bridgePort: port });
        }
        return response;
      }
      lastError = new Error(`桥返回 HTTP ${response.status}`);
    } catch (error) {
      lastError = error;
    }
  }
  // 所有端口都连不上 = 桥进程没在跑（应用没开 / 后台桥没自启）。标记离线，
  // 上层据此把收藏攒进本地队列而非报错丢弃。
  state.online = false;
  throw lastError;
}

function portRange() {
  const ports = [];
  for (let port = PORT_START; port <= PORT_END; port += 1) {
    ports.push(port);
  }
  return ports;
}

// ── 离线缓存队列（C 方案） ──
// 桥不可达时把收藏攒进 chrome.storage.local，桥恢复后回放。键值：pendingCaptures。

const OFFLINE_QUEUE_KEY = 'pendingCaptures';
// Obsidian 设置缓存：离线时 /obsidian/status 拿不到，得用上次在线缓存的
// vaultName / subdir 才拼得出 obsidian://new 深链。
const OBSIDIAN_SETTINGS_KEY = 'obsidianSettings';

// 网络失败时用它替代 bridgeFetch：桥挂了返回 null 而不是抛错，调用方据此降级而非把表单锁死。
async function bridgeFetchOrNull(path, options = {}) {
  try {
    return await bridgeFetch(path, options);
  } catch (error) {
    return null;
  }
}

async function readQueue() {
  const stored = await chrome.storage.local.get([OFFLINE_QUEUE_KEY]);
  return stored[OFFLINE_QUEUE_KEY] || [];
}

// 写入一条离线操作。同 (type + url) 只保留最后一条，避免重复攒一堆。
async function enqueueOffline(op) {
  const queue = await readQueue();
  const url = op.payload && op.payload.url;
  const filtered = queue.filter(
    (existing) => !(existing.type === op.type && existing.payload && existing.payload.url === url),
  );
  filtered.push(op);
  await chrome.storage.local.set({ [OFFLINE_QUEUE_KEY]: filtered });
}

// 回放离线队列：先补录 capture（建出条目），再补 note（此时条目已存在）。
// 返回本次成功补录的条数；note 若因条目仍未存在而 404，则留到下次再试。
async function flushOffline() {
  if (state.flushing) return 0;
  const queue = await readQueue();
  if (!queue.length) return 0;
  state.flushing = true;
  let flushed = 0;
  try {
    // 先确认桥回来了，否则直接放弃本轮（保留队列）。
    await bridgeFetch('/ping');
    const remaining = [];
    const captures = queue.filter((op) => op.type === 'capture');
    const notes = queue.filter((op) => op.type === 'note');
    const claims = queue.filter((op) => op.type === 'claim');
    for (const op of captures) {
      try {
        await bridgeFetch('/capture', {
          method: 'POST',
          body: JSON.stringify(op.payload),
        });
        flushed += 1;
      } catch (error) {
        remaining.push(op);
      }
    }
    for (const op of notes) {
      try {
        let response = await bridgeFetch('/note', {
          method: 'POST',
          body: JSON.stringify(op.payload),
        });
        // 条目还没建出来（离线下只写了笔记、没点收藏）→ 先用标题建条目，再回放笔记。
        if (response.status === 404) {
          if (!(await captureFromNote(op.payload))) {
            remaining.push(op);
            continue;
          }
          flushed += 1;
          response = await bridgeFetch('/note', {
            method: 'POST',
            body: JSON.stringify(op.payload),
          });
          if (response.status === 404) {
            remaining.push(op);
            continue;
          }
        }
        flushed += 1;
      } catch (error) {
        remaining.push(op);
      }
    }
    // 认领离线下已建好的 Obsidian 文件。必须排在 capture/note 之后：条目得先存在，
    // 否则 claim 会 404，留到下一轮（那时条目已被上面的流程建出来了）。
    for (const op of claims) {
      try {
        const response = await bridgeFetch('/obsidian/claim', {
          method: 'POST',
          body: JSON.stringify(op.payload),
        });
        if (response.status === 404) {
          remaining.push(op);
          continue;
        }
        flushed += 1;
      } catch (error) {
        remaining.push(op);
      }
    }
    await chrome.storage.local.set({ [OFFLINE_QUEUE_KEY]: remaining });
  } catch (error) {
    // 桥又掉了，队列原样保留。
  } finally {
    state.flushing = false;
  }
  return flushed;
}

// 离线下「只写了笔记、没点收藏」时，补录阶段先按 title 把条目建出来。
// 不带 note：CaptureRequest.note 为 None 表示「不要动库里的 notes」，
// 否则会把刚建出来的条目笔记写空、随后又被 note 回放覆盖，白折腾一轮。
async function captureFromNote(notePayload) {
  if (!notePayload || !notePayload.url) return false;
  try {
    await bridgeFetch('/capture', {
      method: 'POST',
      body: JSON.stringify({
        url: notePayload.url,
        title: notePayload.title || notePayload.url,
      }),
    });
    return true;
  } catch (error) {
    return false;
  }
}

// 后台定时器：在线时定期尝试回放队列（侧边栏开着期间生效）。
function startFlushTimer() {
  if (state.flushTimer) return;
  state.flushTimer = setInterval(async () => {
    if (state.online && !state.flushing) {
      const flushed = await flushOffline();
      if (flushed > 0) await loadPage();
    }
  }, 30000);
}

// ── 注入脚本 ──
// 下面几个函数会被序列化后注入页面，**不能引用外部作用域的任何变量**。

// eslint-disable-next-line no-unused-vars
function pageVideoState() {
  const videos = Array.from(document.querySelectorAll('video')).filter(
    (v) => Number.isFinite(v.duration) && v.duration > 0,
  );
  if (!videos.length) return null;
  const pick = videos.sort(
    (a, b) => b.offsetWidth * b.offsetHeight - a.offsetWidth * a.offsetHeight,
  )[0];
  return { currentTime: pick.currentTime, duration: pick.duration };
}

// eslint-disable-next-line no-unused-vars
function pageSeek(seconds) {
  const videos = Array.from(document.querySelectorAll('video')).filter((v) =>
    Number.isFinite(v.duration),
  );
  if (!videos.length) return false;
  const pick = videos.sort(
    (a, b) => b.offsetWidth * b.offsetHeight - a.offsetWidth * a.offsetHeight,
  )[0];
  pick.currentTime = Math.max(0, Math.min(pick.duration, seconds));
  const played = pick.play();
  if (played && typeof played.catch === 'function') played.catch(() => {});
  return true;
}

// eslint-disable-next-line no-unused-vars
function pageSelection() {
  const selection = window.getSelection();
  const text = (selection && selection.toString() ? selection.toString() : '').trim();
  if (!text) return null;
  let node = selection.anchorNode;
  if (node && node.nodeType === 3) node = node.parentElement;
  const withId = node && node.closest ? node.closest('[id]') : null;
  return { text: text.slice(0, 60), anchorId: withId && withId.id ? withId.id : null };
}

// eslint-disable-next-line no-unused-vars
function pageScrollToText(quote, anchorId) {
  let target = anchorId ? document.getElementById(anchorId) : null;
  if (!target) {
    const nodes = document.querySelectorAll('p, li, h1, h2, h3, h4, blockquote, td');
    const needle = quote.slice(0, 12);
    for (const node of nodes) {
      if (node.textContent.includes(needle)) {
        target = node;
        break;
      }
    }
  }
  if (!target) return false;
  target.scrollIntoView({ block: 'center', behavior: 'smooth' });
  return true;
}

async function callPage(fn, args = []) {
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  if (!tab || !tab.id) throw new Error('找不到当前标签页');
  const [injected] = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: fn,
    args,
  });
  return injected ? injected.result : null;
}

// ── 读取当前页面 ──

async function readCurrentTab() {
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  if (!tab || !tab.id) {
    throw new Error('找不到当前标签页');
  }
  // 浏览器内部页面（chrome://、edge://、about:、扩展自身页）扩展永远无法注入脚本，
  // 直接抛带标记的错，让上层显示友好提示而不是原始英文报错。
  if (tab.url && /^chrome:|^edge:|^about:|^chrome-extension:|^moz-extension:/i.test(tab.url)) {
    const error = new Error('这个页面无法收藏（浏览器内部页面）');
    error.code = 'PROTECTED';
    throw error;
  }
  try {
    const [injected] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => {
        const pick = (selector) => {
          const node = document.querySelector(selector);
          return node ? node.content : '';
        };
        const ogTitle = pick('meta[property="og:title"]');
        const ogImage = pick('meta[property="og:image"]');
        const ogDesc = pick('meta[property="og:description"]');
        const metaDesc = pick('meta[name="description"]');
        // 知乎登录后 document.title 会带未读提示，如「(8封私信/10条消息) 标题 - 知乎」。
        const title = (ogTitle || document.title || '')
          .replace(/^[（(][^)）]*(?:私信|消息)[^)）]*[)）]\s*/, '')
          .replace(/\s*[-–—]\s*知乎\s*$/, '')
          .trim();
        const videos = Array.from(document.querySelectorAll('video')).filter(
          (v) => Number.isFinite(v.duration) && v.duration > 0,
        );
        return {
          url: location.href,
          title,
          ogImage,
          description: ogDesc || metaDesc,
          selection: (window.getSelection() ? window.getSelection().toString() : '').trim(),
          hasVideo: videos.length > 0,
        };
      },
    });
    if (injected && injected.result) {
      return { id: tab.id, ...injected.result };
    }
  } catch (error) {
    // 落到下面的 tabs 兜底。
  }
  if (tab.url) {
    return {
      id: tab.id,
      url: tab.url,
      title: tab.title || tab.url,
      description: '',
      selection: '',
      hasVideo: false,
    };
  }
  throw new Error('读取当前页面失败：拿不到页面地址');
}

async function loadPage() {
  let page;
  try {
    page = await readCurrentTab();
  } catch (error) {
    if (error && error.code === 'PROTECTED') {
      setStatus('这个页面无法收藏（浏览器内部页面）', 'error');
    } else {
      setStatus(`读取当前页面失败：${error.message}`, 'error');
    }
    disableForm(true);
    return;
  }
  if (!page.url) {
    setStatus('这个页面无法收藏（浏览器内部页面）', 'error');
    disableForm(true);
    return;
  }

  state.page = page;
  state.tabId = page.id;
  els.url.textContent = page.url;

  let tags = [];
  let item = null;
  // 离线模式下桥不可达：tags/item 拿不到就当空，表单照常可用（收藏会进本地队列）。
  // 用 bridgeFetchOrNull 而非 bridgeFetch，避免网络失败时把整个面板锁死。
  const [tagsResponse, itemResponse] = await Promise.all([
    bridgeFetchOrNull('/tags'),
    bridgeFetchOrNull(`/item?url=${encodeURIComponent(page.url)}`),
  ]);
  if (tagsResponse) {
    tags = ((await tagsResponse.json()).tags || []).map((tag) => tag.name);
  }
  if (itemResponse) {
    const looked = await itemResponse.json();
    if (looked.exists && looked.item) {
      item = looked.item;
    }
  }

  // Obsidian 联动状态：决定要不要显示「在 Obsidian 中打开」。
  // 离线拿不到就退回上次缓存的 vaultName/subdir —— 否则按钮会被隐藏、离线深链也拼不出来。
  try {
    const response = await bridgeFetchOrNull('/obsidian/status');
    if (response) {
      const status = await response.json();
      state.obsidian = status;
      await chrome.storage.local.set({ [OBSIDIAN_SETTINGS_KEY]: status });
    } else {
      const cached = await chrome.storage.local.get([OBSIDIAN_SETTINGS_KEY]);
      state.obsidian = cached[OBSIDIAN_SETTINGS_KEY] || null;
    }
  } catch (error) {
    state.obsidian = null; // 老版本 app 没有这个端点，静默降级
  }

  state.pool = tags;
  state.exists = Boolean(item);
  state.item = item;
  hideConflict();

  if (item) {
    state.selected = item.tags || [];
    els.title.value = item.title || page.title || page.url;
    els.note.value = item.notes || '';
    els.annotation.value = item.annotation || '';
    els.save.textContent = '更新';
  } else {
    state.selected = [];
    els.title.value = page.title || page.url;
    els.note.value = '';
    els.save.textContent = '收藏';
  }

  // 只在「服务端为空且有本地草稿」时恢复，避免把用户刚清空的内容又捞回来。
  const draft = await readDraft(page.url);
  if (draft && !els.note.value) {
    els.note.value = draft;
    markDirty();
  }

  if (state.noteMode === 'preview') renderPreview();

  state.dirty = false;
  state.anDirty = false;
  render();
  renderNoteChrome();
  setSaved(item ? '已保存' : '');
  disableForm(false);
}

function setMode(mode) {
  if (mode === state.mode) return;
  // 批注与笔记是两个**独立字段**，切换时绝不搬运内容，否则一边会把另一边覆盖掉。
  // 各自未落盘的改动由各自的自动保存定时器负责（定时器是模块级的，切页照样会触发），
  // 所以这里不需要手动 flush。
  state.mode = mode;
  chrome.storage.local.set({ sidebarMode: mode });
  renderModeChrome();
}

let anSaveTimer = null;
function markAnDirty() {
  // 批注有自己的保存指示与脏标记，**不碰 state.dirty** ——
  // 那是笔记一套的脏标记，复用会让「切到笔记模式」被误判成笔记有改动而存一次。
  state.anDirty = true;
  els.anDot.className = 'dot pending';
  els.anText.textContent = '未保存…';
  clearTimeout(anSaveTimer);
  anSaveTimer = setTimeout(saveAnnotation, AUTOSAVE_MS);
}
async function saveAnnotation() {
  if (!state.page || !state.item) return;
  // 批注走独立端点 /annotation：只落本地 annotations 表，
  // 不动 items.notes（那是笔记模式 + Obsidian 的地盘），因此也没有乐观锁冲突一说。
  const body = {
    url: state.page.url,
    annotation: els.annotation.value,
  };
  try {
    const response = await bridgeFetch('/annotation', { method: 'POST', body: JSON.stringify(body) });
    const data = await response.json();
    if (!data.ok) throw new Error(data.error || '保存失败');
    state.item = { ...state.item, annotation: data.annotation };
    state.anDirty = false;
    els.anDot.className = 'dot';
    els.anText.textContent = '已保存';
  } catch (error) {
    els.anText.textContent = '保存失败';
  }
}
function flushAnnotationSave() {
  clearTimeout(anSaveTimer);
  if (state.page && state.item) saveAnnotation();
}

async function createNote() {
  if (!state.page) return;
  try {
    await bridgeFetch('/obsidian/create', {
      method: 'POST',
      body: JSON.stringify({ url: state.page.url }),
    });
    setStatus('已在 vault 新建笔记，刷新中…', 'success');
    await loadPage();
  } catch (error) {
    setStatus(`新建笔记失败：${error.message}`, 'error');
  }
}

// ── 视图切换 ──

function setView(view) {
  state.view = view;
  els.tabCollect.classList.toggle('on', view === 'collect');
  els.tabNote.classList.toggle('on', view === 'note');
  els.viewCollect.classList.toggle('on', view === 'collect');
  els.viewNote.classList.toggle('on', view === 'note');
  chrome.storage.local.set({ sidebarView: view });
  if (view === 'note') {
    renderNoteChrome();
    // 进入笔记视图时同步预览/编辑显隐（init 时 noteMode 可能已是 preview）
    els.note.hidden = state.noteMode !== 'edit';
    els.notePreview.hidden = state.noteMode === 'edit';
    els.mkPreview.textContent = state.noteMode === 'edit' ? '预览' : '编辑';
    if (state.noteMode === 'preview') renderPreview();
  }
}

function renderNoteChrome() {
  // 离线时视为「已开放」：笔记先攒本地，应用启动后自动补录（含自动建条目），
  // 所以不再显示「这一页还没收藏」的锁定提示。
  const open = Boolean(state.item) || !state.online;
  els.nbLocked.hidden = open;
  els.nbOpen.classList.toggle('on', open);
  // 没有视频就不显示「+ 时间戳」——一个常年灰着的按钮只是噪音。
  els.mkTime.hidden = !state.page || !state.page.hasVideo;
  renderModeChrome();
  renderObsidianButton();
  renderMarkers();
}

function renderModeChrome() {
  const annotate = state.mode === 'annotation';
  els.tabAnnotate.classList.toggle('on', annotate);
  els.tabNotebook.classList.toggle('on', !annotate);
  els.annotatePane.hidden = !annotate;
  els.notebookPane.hidden = annotate;
  // 笔记模式：没关联到 Obsidian 笔记前，只给「新建 / 关联」二选一，**不开放编辑器**。
  //
  // ⚠️ 这里**不能**加 `state.item` 守卫（以前就踩过）：加了之后「未收藏」或「离线」时
  // 整个分支被跳过，note-editor 停留在 HTML 默认的可见状态 —— 可编辑框就这么漏了出来，
  // 用户还没做选择就能往里打字。有没有笔记只取决于有没有关联文件。
  if (!annotate) {
    const hasNote = Boolean(state.item && state.item.obsidianPath);
    // 离线时 Obsidian 根本够不着，「新建 / 关联」点了必然失败，摆两个死按钮不如退回本地编辑：
    // 笔记照写，攒进离线队列，联网后自动补录。
    // 在线时严格遵守「先选创建 / 关联，再开放编辑器」。
    const editable = hasNote || !state.online;
    els.noteSetup.hidden = editable;
    els.noteEditor.hidden = !editable;
  }
  if (annotate) renderAnnotationMarkers();
}

function renderObsidianButton() {
  const configured = state.obsidian && state.obsidian.enabled;
  els.openObsidian.hidden = !configured;
  els.linkObsidian.hidden = !configured;
  els.syncObsidian.hidden = !configured;
  if (!configured) {
    els.linkedNote.hidden = true;
    return;
  }
  // 在线：必须有已同步的 obsidianPath 才允许点（避免还没建文件就打开）。
  // 离线：直接本地拼 obsidian://new 唤起，路径稍后由 flush 认领。
  const hasNote = Boolean(state.item && state.item.obsidianPath);
  const offlineMode = !state.online;
  els.openObsidian.disabled = !hasNote && !offlineMode;
  els.openObsidian.title = offlineMode
    ? '离线打开：会在 Obsidian 里新建笔记，应用启动后自动同步'
    : hasNote
      ? ''
      : '这条还没同步到 Obsidian，先写点笔记保存一次';

  // 已关联到某个笔记文件时，把文件名亮出来并提供「取消关联」出口。
  els.linkedNote.hidden = !hasNote;
  if (hasNote) {
    els.linkedPath.textContent = state.item.obsidianPath;
    els.linkedPath.title = state.item.obsidianPath;
  }
}

// ── 渲染 ──

function render() {
  renderSelected();
  renderPool();
}

function renderSelected() {
  els.selected.textContent = '';
  for (const name of state.selected) {
    els.selected.appendChild(
      chip(name, true, () => {
        state.selected = state.selected.filter((item) => item !== name);
        render();
      }),
    );
  }
}

function renderPool() {
  const keyword = els.tagInput.value.trim().toLowerCase();
  els.pool.textContent = '';
  for (const name of state.pool) {
    if (state.selected.includes(name)) continue;
    if (keyword && !name.toLowerCase().includes(keyword)) continue;
    els.pool.appendChild(
      chip(name, false, () => {
        state.selected.push(name);
        els.tagInput.value = '';
        render();
      }),
    );
  }
}

function chip(name, selected, onClick) {
  const node = document.createElement('span');
  node.className = selected ? 'chip on' : 'chip';
  node.textContent = name;
  if (selected) {
    const remove = document.createElement('span');
    remove.className = 'remove';
    remove.textContent = '×';
    remove.addEventListener('click', (event) => {
      event.stopPropagation();
      onClick();
    });
    node.appendChild(remove);
  } else {
    node.addEventListener('click', onClick);
  }
  return node;
}

// ── 标记 ──

const MARKER_RE = /^[ \t]*[-*][ \t]*\[([^\]]+)\]\(([^)\s]+)\)[ \t]*(.*)$/;

function parseMarkersFrom(text) {
  const out = [];
  (text || '').split('\n').forEach((line) => {
    const m = line.match(MARKER_RE);
    if (!m) return;
    const url = m[2];
    let kind = null;
    if (/[:~]text=/.test(url)) kind = 'pos';
    else if (/[?&]t=/.test(url)) kind = 'time';
    if (!kind) return;
    out.push({
      label: m[1],
      url,
      desc: m[3] || '',
      kind,
      seconds: kind === 'time' ? parseInt((url.match(/[?&]t=(\d+)/) || [0, 0])[1], 10) || 0 : 0,
      anchor: kind === 'pos' ? (url.split('#')[1] || '').split(':~:')[0] || null : null,
      quote: kind === 'pos' ? decodeURIComponent((url.split('text=')[1] || '').split('&')[0]) : '',
    });
  });
  return out;
}

function parseMarkers() {
  return parseMarkersFrom(els.note.value || '');
}

function renderMarkersInto(listEl, countEl, text) {
  const list = parseMarkersFrom(text);
  countEl.textContent = list.length ? `${list.length} 条` : '';
  listEl.textContent = '';
  if (!list.length) {
    const empty = document.createElement('div');
    empty.className = 'mk-empty';
    empty.textContent = '还没有标记。';
    listEl.appendChild(empty);
    return;
  }
  for (const marker of list) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'mk';

    const badge = document.createElement('span');
    badge.className = 'badge';
    badge.textContent = marker.kind === 'time' ? formatTime(marker.seconds) : '位置';

    const text = document.createElement('span');
    text.className = 'txt';
    text.textContent = marker.desc || (marker.kind === 'pos' ? marker.quote : '') || '（无说明）';

    const go = document.createElement('span');
    go.className = 'go';
    go.textContent = '跳转 ›';

    button.appendChild(badge);
    button.appendChild(text);
    button.appendChild(go);
    button.addEventListener('click', () => jump(marker));
    listEl.appendChild(button);
  }
}

function renderMarkers() {
  renderMarkersInto(els.mkList, els.mkCount, els.note.value || '');
}

function renderAnnotationMarkers() {
  renderMarkersInto(els.anList, els.anCount, els.annotation.value || '');
}

async function jump(marker) {
  try {
    if (marker.kind === 'time') {
      const ok = await callPage(pageSeek, [marker.seconds]);
      if (!ok) {
        // 页面里找不到 video（比如播放器被卸载了）→ 退化成带 t= 参数重新打开
        await chrome.tabs.update(state.tabId, { url: marker.url });
        setStatus(`已跳到 ${formatTime(marker.seconds)}（重新载入）`);
        return;
      }
      setStatus(`已跳到 ${formatTime(marker.seconds)}`, 'success');
    } else {
      const ok = await callPage(pageScrollToText, [marker.quote, marker.anchor]);
      setStatus(ok ? '已定位到标记位置' : '没找到这段内容', ok ? 'success' : 'error');
    }
  } catch (error) {
    setStatus(`跳转失败：${error.message}`, 'error');
  }
}

// ── 插入标记 ──

function formatTime(seconds) {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n) => (n < 10 ? `0${n}` : `${n}`);
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

// 镜像 Rust 端 normalize_url：剥掉追踪参数、保留 p（B站分P）、去掉 fragment。
// 否则写进笔记的标记 URL 会带上整串 spm_id_from/vd_source，而且页面自带的 ?t=
// 会和插入的跳秒参数撞车导致定位错乱。
const STRIP_PARAMS = [
  'spm_id_from',
  'from_spm_id',
  'vd_source',
  'share_source',
  'share_tag',
  'share_from',
  'unique_k',
  't',
  'timestamp',
  'bsource',
  'bsource_type',
  'utm_source',
  'utm_medium',
  'utm_campaign',
  'utm_term',
  'utm_content',
  'utm_id',
];

function cleanUrl(url) {
  const h = url.indexOf('#');
  const base = h >= 0 ? url.slice(0, h) : url;
  const q = base.indexOf('?');
  if (q < 0) return base; // 无 query：直接返回干净路径
  const path = base.slice(0, q);
  const params = new URLSearchParams(base.slice(q + 1));
  for (const key of STRIP_PARAMS) params.delete(key);
  const s = params.toString();
  return s ? `${path}?${s}` : path;
}

function withTimeParam(url, seconds) {
  const base = cleanUrl(url);
  return `${base}${base.includes('?') ? '&' : '?'}t=${seconds}`;
}

function withTextFragment(url, quote, anchorId) {
  const base = cleanUrl(url);
  const fragment = `${anchorId ? `#${anchorId}` : ''}:~:text=${encodeURIComponent(quote)}`;
  return base + fragment;
}

// 在当前光标处插入一行，并把光标停在这一行末尾——
// 连续打点时不需要每次重新定位光标。
function insertLineInto(textarea, text) {
  const start = textarea.selectionStart;
  const end = textarea.selectionEnd;
  const before = textarea.value.slice(0, start);
  const after = textarea.value.slice(end);
  const prefix = before.length && !/\n$/.test(before) ? '\n' : '';
  textarea.value = before + prefix + text + after;
  const caret = before.length + prefix.length + text.length;
  textarea.focus();
  textarea.setSelectionRange(caret, caret);

  // ⚠️ 程序化改 `textarea.value` **不会触发 input 事件**，所以这里必须手动标脏，
  // 否则插入的时间戳只是显示在框里、永远不会落盘。
  //
  // 而且必须按目标输入框**分派**：批注与笔记是两个独立字段、两套脏标记与自动保存。
  // 以前这里无条件走笔记那套（renderMarkers + markDirty），导致「在批注里插时间戳」
  // 触发的是 saveNote（存笔记），批注的 saveAnnotation 根本没被排上 —— 表现就是
  // 「时间戳插进去了，刷新后没了」。
  if (textarea === els.annotation) {
    renderAnnotationMarkers();
    markAnDirty();
  } else {
    renderMarkers();
    markDirty();
  }
}

// ── 笔记：编辑 / 预览切换 ──
// 编辑态看到原始 Markdown（方便手写）；预览态把时间戳 / 位置标记渲染成可点的
// 干净按钮，把长 URL 完全藏起来，笔记读起来清爽。两种形态存的是同一份文本。

function setNoteMode(mode) {
  state.noteMode = mode;
  if (mode === 'preview') flushSave(); // 进预览前先落盘，免得看到旧内容
  const edit = mode === 'edit';
  els.note.hidden = !edit;
  els.notePreview.hidden = edit;
  els.mkPreview.textContent = edit ? '预览' : '编辑';
  if (!edit) renderPreview();
  chrome.storage.local.set({ sidebarNoteMode: mode });
}

// 把笔记文本渲染成预览：标记行 → 可点按钮（URL 隐藏），其余行原样展示。
// 整篇接管后，关联的笔记可能自带 YAML frontmatter。Obsidian 的阅读视图是不显示它的，
// 预览跟着隐藏，否则用户会以为「我的笔记前面怎么多出来这些东西」。
//
// ⚠️ 只作用于**预览**：编辑框必须保持完整原文，用户要编辑的就是整篇。
// 没有闭合的 `---` 就不算 frontmatter（用户正文里可能真有分隔线），原样返回。
function stripFrontmatter(text) {
  if (!text.startsWith('---')) return text;
  const lines = text.split('\n');
  for (let i = 1; i < lines.length; i += 1) {
    const t = lines[i].trim();
    if (t === '---' || t === '...') return lines.slice(i + 1).join('\n');
  }
  return text;
}

function renderPreview() {
  const box = els.notePreview;
  box.textContent = '';
  const lines = stripFrontmatter(els.note.value || '').split('\n');
  if (lines.length === 1 && lines[0].trim() === '') {
    const ph = document.createElement('p');
    ph.className = 'ph';
    ph.textContent = '预览：时间戳与位置标记会显示为可点击的干净按钮，链接本身被隐藏。';
    box.appendChild(ph);
    return;
  }
  lines.forEach((line) => {
    const div = document.createElement('div');
    div.className = 'ln';
    const m = line.match(MARKER_RE);
    let marker = null;
    if (m) {
      const url = m[2];
      let kind = null;
      if (/[:~]text=/.test(url)) kind = 'pos';
      else if (/[?&]t=/.test(url)) kind = 'time';
      if (kind) {
        marker = {
          kind,
          seconds: kind === 'time' ? parseInt((url.match(/[?&]t=(\d+)/) || [0, 0])[1], 10) || 0 : 0,
          anchor: kind === 'pos' ? (url.split('#')[1] || '').split(':~:')[0] || null : null,
          quote: kind === 'pos' ? decodeURIComponent((url.split('text=')[1] || '').split('&')[0]) : '',
          url,
          label: m[1],
          desc: m[3] || '',
        };
      }
    }
    if (marker) {
      const chip = document.createElement('button');
      chip.type = 'button';
      chip.className = `mk-chip ${marker.kind}`;
      chip.textContent = marker.kind === 'time' ? formatTime(marker.seconds) : '位置';
      chip.title = marker.kind === 'time' ? `跳到 ${formatTime(marker.seconds)}` : '跳到该位置';
      chip.addEventListener('click', () => jump(marker));
      div.appendChild(chip);
      if (marker.desc) {
        const span = document.createElement('span');
        span.className = 'desc';
        span.textContent = ` ${marker.desc}`;
        div.appendChild(span);
      }
    } else {
      div.textContent = line;
    }
    box.appendChild(div);
  });
}

async function insertTimestampInto(textarea) {
  if (!state.page) return;
  let seconds = 0;
  try {
    const video = await callPage(pageVideoState);
    if (!video) {
      setStatus('这一页没检测到视频', 'error');
      return;
    }
    // 回退 3 秒：听到重点再点按钮已经有延迟了，跳回去正好落在重点前。
    seconds = Math.max(0, Math.floor(video.currentTime) - 3);
  } catch (error) {
    setStatus(`读取播放进度失败：${error.message}`, 'error');
    return;
  }
  const url = withTimeParam(state.page.url, seconds);
  insertLineInto(textarea, `- [${formatTime(seconds)}](${url}) `);
  if (textarea === els.note && state.noteMode === 'preview') setNoteMode('edit'); // 插入后回到编辑
  setStatus(`已插入 ${formatTime(seconds)}，直接接着打字`, 'success');
}

async function insertPositionInto(textarea) {
  if (!state.page) return;
  let selection = null;
  try {
    selection = await callPage(pageSelection);
  } catch (error) {
    setStatus(`读取选区失败：${error.message}`, 'error');
    return;
  }
  if (!selection || !selection.text) {
    setStatus('先在页面里选中一段文字', 'error');
    return;
  }
  const url = withTextFragment(state.page.url, selection.text, selection.anchorId);
  insertLineInto(textarea, `- [位置](${url}) `);
  if (textarea === els.note && state.noteMode === 'preview') setNoteMode('edit');
  setStatus(
    selection.anchorId ? `已用锚点 #${selection.anchorId} + 文本片段` : '没有可用锚点，已用文本片段',
    'success',
  );
}

// ── 保存笔记 ──

let saveTimer = null;

function markDirty() {
  // 离线模式：没有 item 是常态（/item 返 null），但笔记仍要落本地草稿 + 触发入队
  if (!state.item && state.online) return;
  state.dirty = true;
  setSaved('编辑中…', true);
  writeDraft(state.page.url, els.note.value);
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => saveNote(), AUTOSAVE_MS);
}

function setSaved(text, pending) {
  els.saveText.textContent = text || '';
  els.saveDot.className = pending ? 'dot pending' : 'dot';
}

async function saveNote() {
  if (!state.page) return;
  // 离线模式：直接把笔记攒本地，不带条目校验（条目恢复后由 flush 补录）。
  if (!state.online) {
    const body = {
      url: state.page.url,
      note: els.note.value,
      baseUpdatedAt: state.item ? state.item.updatedAt : null,
      // 带 title：flush 遇 404 时用来先建条目，再回放笔记。
      title: state.page.title || state.page.url,
    };
    await enqueueOffline({ type: 'note', payload: body });
    state.dirty = false;
    setSaved('已离线缓存', true);
    setStatus('已离线缓存笔记，应用启动后自动补录', 'success');
    return;
  }
  if (!state.item) return;
  clearTimeout(saveTimer);
  const body = {
    url: state.page.url,
    note: els.note.value,
    baseUpdatedAt: state.item.updatedAt === undefined ? null : state.item.updatedAt,
  };
  try {
    const response = await bridgeFetch('/note', { method: 'POST', body: JSON.stringify(body) });
    const data = await response.json();
    if (data.conflict) {
      state.conflict = {
        remoteNotes: data.remoteNotes || '',
        remoteUpdatedAt: data.remoteUpdatedAt,
      };
      els.conflict.hidden = false;
      setSaved('未保存 · 有冲突', true);
      return;
    }
    if (!data.ok) {
      throw new Error(data.error || '保存失败');
    }
    state.item = {
      ...state.item,
      notes: data.notes,
      updatedAt: data.updatedAt,
      obsidianPath: data.obsidianPath,
    };
    state.dirty = false;
    hideConflict();
    clearDraft(state.page.url);
    renderObsidianButton();
    state.savedAt = Date.now();
    setSaved('已保存 · 刚刚');
    // 顺手把排队的离线收藏补录掉。
    const flushed = await flushOffline();
    if (flushed > 0) setStatus(`已保存 · 另补录 ${flushed} 条离线收藏`, 'success');
  } catch (error) {
    setSaved('保存失败', true);
    setStatus(`笔记保存失败：${error.message}`, 'error');
  }
}

// 页面即将卸载时把未保存内容推出去。普通 fetch 会被杀掉，
// keepalive 能让请求活过页面销毁；token 走查询参数，桥本身就支持这种鉴权。
function flushSave() {
  if (!state.item || !state.page || !state.port) return;
  const base = `http://127.0.0.1:${state.port}`;
  const token = encodeURIComponent(state.token);
  const post = (path, payload) => {
    try {
      fetch(`${base}${path}?token=${token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
        keepalive: true,
      });
    } catch (error) {
      // 尽力而为：实在发不出去，草稿还在 storage 里兜底
    }
  };
  // 批注与笔记是两个**独立字段**，各自有脏标记，卸载时分别把各自那份推出去。
  // 不能「按当前激活 pane 二选一」——那样另一边的改动就丢了。
  if (state.anDirty) {
    post('/annotation', { url: state.page.url, annotation: els.annotation.value });
    state.anDirty = false;
  }
  if (state.dirty) {
    post('/note', {
      url: state.page.url,
      note: els.note.value,
      baseUpdatedAt: state.item.updatedAt === undefined ? null : state.item.updatedAt,
    });
    state.dirty = false;
  }
}

function hideConflict() {
  state.conflict = null;
  els.conflict.hidden = true;
}

// ── 草稿（最后一道兜底） ──

function draftKey(url) {
  return `draft:${url}`;
}

async function readDraft(url) {
  const stored = await chrome.storage.local.get([draftKey(url)]);
  return stored[draftKey(url)] || '';
}

function writeDraft(url, value) {
  chrome.storage.local.set({ [draftKey(url)]: value });
}

function clearDraft(url) {
  chrome.storage.local.remove([draftKey(url)]);
}

// ── Obsidian ──

// 与 Rust `obsidian::sanitize_filename` 对齐：过滤 Windows 非法字符、换行/tab 转空格、
// 截断 200 字符、空则兜底「未命名收藏」。两边算法必须一致，否则认领会落空。
function sanitizeFilename(name) {
  let out = String(name || '')
    .replace(/[\\/:*?"<>|]/g, '')
    .replace(/[\n\r\t]/g, ' ')
    .trim();
  out = Array.from(out).slice(0, 200).join('');
  return out || '未命名收藏';
}

// 与 Rust `obsidian::urlencode` 对齐：只放行 A-Za-z0-9 与 - _ . ~，其余按 UTF-8 字节 %XX。
function obsidianUrlencode(input) {
  const bytes = new TextEncoder().encode(String(input));
  let out = '';
  for (const byte of bytes) {
    const ch = String.fromCharCode(byte);
    if (/^[A-Za-z0-9\-_.~]$/.test(ch)) {
      out += ch;
    } else {
      out += `%${byte.toString(16).toUpperCase().padStart(2, '0')}`;
    }
  }
  return out;
}

// 与 Rust `obsidian::rel_for_new` 对齐：`<subdir>/<标题>.md`（subdir 为空则不带前缀）。
function obsidianRelPath(title) {
  const sub = String((state.obsidian && state.obsidian.subdir) || '').trim().replace(/\/+$/, '');
  const file = `${sanitizeFilename(title)}.md`;
  return sub ? `${sub}/${file}` : file;
}

// 离线打开：桥不可达时本地拼 obsidian://new 直接唤起客户端。
// 实际路径记进离线队列，等 app 起来后由 /obsidian/claim 认领 —— 否则 app 同步时
// 会认为「没同步过」而再建一个文件（重复）。
async function openInObsidianOffline() {
  const settings = state.obsidian;
  if (!settings || !settings.enabled || !settings.vaultName) {
    setStatus('离线打开需要先用应用同步一次 Obsidian 设置', 'error');
    return;
  }
  const title = (state.page && (state.page.title || state.page.url)) || '未命名收藏';
  const rel = obsidianRelPath(title);
  const uri =
    `obsidian://new?vault=${obsidianUrlencode(settings.vaultName)}` +
    `&file=${obsidianUrlencode(rel)}&content=${obsidianUrlencode(els.note.value || '')}`;
  let tab = null;
  try {
    tab = await chrome.tabs.create({ url: uri });
  } catch (error) {
    setStatus(`离线打开失败：${error.message}`, 'error');
    return;
  }
  // 外部协议唤起后往往留下一个空白标签页，稍后清掉（清不掉也无所谓）。
  if (tab && tab.id) {
    setTimeout(() => {
      chrome.tabs.remove(tab.id).catch(() => {});
    }, 1500);
  }
  await enqueueOffline({ type: 'claim', payload: { url: state.page.url, path: rel } });
  setStatus('已在 Obsidian 中打开（离线新建，应用启动后自动关联）', 'success');
}

async function openInObsidian() {
  if (!state.page) return;
  if (!state.online) {
    await openInObsidianOffline();
    return;
  }
  try {
    const response = await bridgeFetch('/obsidian/open', {
      method: 'POST',
      body: JSON.stringify({ url: state.page.url }),
    });
    const data = await response.json();
    if (!data.ok) throw new Error(data.error || '打开失败');
    setStatus('已在 Obsidian 中打开', 'success');
  } catch (error) {
    setStatus(`打开失败：${error.message}`, 'error');
  }
}

// ── 自定义关联笔记 ──
//
// 浏览器扩展碰不到本地文件系统、也弹不了系统文件对话框，所以「让用户挑一篇笔记」
// 只能由 Rust 侧扫描 vault 后把候选回传，再在这里以搜索框的形式选择。

function openLinkPanel() {
  if (!state.page) return;
  els.linkPanel.hidden = false;
  els.linkSearch.value = '';
  els.linkSearch.focus();
  loadLinkCandidates('');
}

function closeLinkPanel() {
  els.linkPanel.hidden = true;
  els.linkList.innerHTML = '';
  els.linkHint.textContent = '';
}

async function loadLinkCandidates(query) {
  if (state.linkLoading) return;
  state.linkLoading = true;
  els.linkHint.textContent = '正在扫描 vault…';
  try {
    const response = await bridgeFetchOrNull(
      `/obsidian/files?q=${encodeURIComponent(query || '')}`,
    );
    if (!response) {
      els.linkList.innerHTML = '';
      els.linkHint.textContent = '桥不可达：关联需要应用在线运行';
      return;
    }
    const data = await response.json();
    renderLinkList(data.files || [], data.truncated);
  } catch (error) {
    els.linkHint.textContent = `加载失败：${error.message}`;
  } finally {
    state.linkLoading = false;
  }
}

function renderLinkList(files, truncated) {
  els.linkList.innerHTML = '';
  if (files.length === 0) {
    els.linkHint.textContent = '没有匹配的笔记';
    return;
  }
  els.linkHint.textContent = truncated ? '结果较多，只显示了最近修改的一部分' : '';
  for (const file of files) {
    const row = document.createElement('button');
    row.type = 'button';
    row.className = 'link-item';
    row.innerHTML = '';

    const name = document.createElement('span');
    name.className = 'link-name';
    name.textContent = file.name;
    row.appendChild(name);

    const path = document.createElement('span');
    path.className = 'link-file';
    path.textContent = file.path;
    row.appendChild(path);

    if (file.managed) {
      const tag = document.createElement('span');
      tag.className = 'link-tag';
      tag.textContent = '已托管';
      row.appendChild(tag);
    }

    row.addEventListener('click', () => linkToNote(file.path));
    els.linkList.appendChild(row);
  }
}

async function linkToNote(path) {
  if (!state.page) return;
  els.linkHint.textContent = '正在关联…';
  try {
    const response = await bridgeFetch('/obsidian/link', {
      method: 'POST',
      body: JSON.stringify({ url: state.page.url, path }),
    });
    const data = await response.json();
    if (!data.ok) throw new Error(data.error || '关联失败');
    if (state.item) {
      state.item.obsidianPath = data.path || path;
    }
    closeLinkPanel();
    // 关联时后端会把文件里已有的托管区内容读回 items.notes，
    // 这里必须再拉一次把它灌进编辑器 —— 否则编辑器虽然出现了，
    // 框里还是关联前的旧内容（用户要的是「选完之后显示对应笔记的内容」）。
    await reloadLinkedNotes();
    renderObsidianButton();
    renderModeChrome();
    setStatus('已关联笔记，之后内容与这个文件双向同步', 'success');
  } catch (error) {
    els.linkHint.textContent = `关联失败：${error.message}`;
  }
}

async function unlinkNote() {
  if (!state.page) return;
  try {
    const response = await bridgeFetch('/obsidian/unlink', {
      method: 'POST',
      body: JSON.stringify({ url: state.page.url }),
    });
    const data = await response.json();
    if (!data.ok) throw new Error(data.error || '取消关联失败');
    if (state.item) state.item.obsidianPath = null;
    renderObsidianButton();
    renderModeChrome();
    setStatus('已取消关联（笔记文件本身没有删除）', 'success');
  } catch (error) {
    setStatus(`取消关联失败：${error.message}`, 'error');
  }
}

/// 立即把改动同步一遍，不用等下一轮轮询（默认 30 秒一次）。
async function syncNow() {
  setStatus('正在同步…');
  try {
    const response = await bridgeFetch('/obsidian/sync', { method: 'POST' });
    const data = await response.json();
    if (!data.ok) throw new Error(data.error || '同步失败');
    const report = data.report || {};
    const parts = [];
    if (report.pulled) parts.push(`从 Obsidian 读回 ${report.pulled} 条`);
    if (report.pushed) parts.push(`推送到 Obsidian ${report.pushed} 条`);
    if (report.conflicts) parts.push(`${report.conflicts} 条冲突（已以 Obsidian 为准并留痕）`);
    if (report.unlinked) parts.push(`${report.unlinked} 条因笔记被删已取消关联`);

    // 有内容被读回来时，若用户没在编辑就把笔记框刷新成最新 —— 直接覆盖正在编辑的内容太危险。
    if (report.pulled && !state.dirty) {
      await reloadLinkedNotes();
    }
    setStatus(parts.length ? `同步完成：${parts.join('，')}` : '同步完成：没有变化', 'success');
  } catch (error) {
    setStatus(`同步失败：${error.message}`, 'error');
  }
}

/// 重新拉一次当前页的条目，把笔记框刷成库里的值。
async function reloadLinkedNotes() {
  if (!state.page) return;
  try {
    const response = await bridgeFetchOrNull(
      `/item?url=${encodeURIComponent(state.page.url)}`,
    );
    if (!response) return;
    const data = await response.json();
    if (!data.exists || !data.item) return;
    els.note.value = data.item.notes || '';
    if (state.item) {
      state.item.notes = data.item.notes || '';
      state.item.updatedAt = data.item.updatedAt;
    }
    state.dirty = false;
    renderMarkers();
    if (state.noteMode === 'preview') renderPreview();
    setSaved('已保存');
  } catch (error) {
    // 刷新笔记失败不影响同步本身已完成
    console.warn('reloadLinkedNotes failed', error);
  }
}

// ── 轮询同步（第 4 步）──
//
// 桥只能被请求、不能推送，所以在应用里改的批注 / 笔记，侧边栏无从知晓。
// 这里定时回拉一次 /item，把「别处」的改动同步过来。
//
// 两条铁律（否则会把用户正在敲的内容冲掉）：
//   1. 只在**对应字段没有未保存改动**时才覆盖 —— 笔记看 state.dirty，批注看 state.anDirty，两者独立。
//   2. 面板不可见时**不轮询**（用户看不到，纯属白费请求）；转回可见时立刻补一次。

async function pollSync() {
  if (!state.page || state.flushing) return;
  if (document.visibilityState === 'hidden') return;

  const wasOnline = state.online;
  let data = null;
  try {
    const response = await bridgeFetchOrNull(
      `/item?url=${encodeURIComponent(state.page.url)}`,
    );
    // 桥不可达：静默跳过。轮询失败不该打扰用户。
    if (!response) return;
    data = await response.json();
  } catch (error) {
    return;
  }

  // 桥刚恢复（应用启动 / 后台桥拉起）：状态可能整片变了（离线收藏已补录、条目已建出），
  // 走一次完整加载最稳，顺便把攒在本地的离线队列回放掉。
  if (!wasOnline && state.online) {
    await flushOffline();
    await loadPage();
    return;
  }
  if (!data || !data.ok) return;

  const item = data.item || null;
  // 「还没收藏」↔「已收藏」是整体状态翻转（比如在应用里删了、或在别处收了进来），
  // 增量合并不够表达，直接整页重载。
  if (Boolean(item) !== Boolean(state.item)) {
    await loadPage();
    return;
  }
  // 两边都还没收藏：无事可做。注意别写成 `|| !item`，否则未收藏页会每 8 s 整页重载一次。
  if (!item) return;

  let changed = false;

  // 笔记：独立脏标记 dirty。本地有未保存改动时绝不覆盖，
  // 等它自己存完（那时远程值就追平了）再跟。
  const remoteNotes = item.notes || '';
  if (!state.dirty && els.note.value !== remoteNotes) {
    els.note.value = remoteNotes;
    changed = true;
  }

  // 批注：独立字段 + 独立脏标记 anDirty，与笔记互不干扰。
  const remoteAnnotation = item.annotation || '';
  if (!state.anDirty && els.annotation.value !== remoteAnnotation) {
    els.annotation.value = remoteAnnotation;
    changed = true;
  }

  // 合并远程快照。注意：这里**不**碰 els.* 的编辑值（上面已按脏标记分别处理），
  // 只更新用于乐观锁与按钮可用性的元数据。
  const prevPath = state.item ? state.item.obsidianPath : null;
  state.item = state.item ? { ...state.item, ...item } : item;
  if (prevPath !== item.obsidianPath) changed = true;

  if (changed) {
    // renderNoteChrome 内部会连带刷新 mode chrome / Obsidian 按钮 / 时间戳列表。
    renderNoteChrome();
    // 预览态显示的是渲染结果，不是 textarea —— 文本内容变了必须重渲染，
    // 否则用户会盯着一份已经过时的预览（renderNoteChrome 不会做这件事）。
    if (state.noteMode === 'preview') renderPreview();
  }
}

function startPollTimer() {
  if (pollTimer) return;
  pollTimer = setInterval(() => {
    void pollSync();
  }, POLL_MS);
}

function stopPollTimer() {
  if (!pollTimer) return;
  clearInterval(pollTimer);
  pollTimer = null;
}

// ── 收藏 ──

function onTagInput(event) {
  if (event.key === 'Enter' || event.key === ',') {
    event.preventDefault();
    const name = els.tagInput.value.trim();
    if (!name) return;
    if (!state.selected.includes(name)) {
      state.selected.push(name);
    }
    els.tagInput.value = '';
    render();
    return;
  }
  if (event.key === 'Backspace' && els.tagInput.value === '' && state.selected.length > 0) {
    state.selected.pop();
    render();
    return;
  }
  renderPool();
}

async function onSave() {
  if (!state.page || !state.page.url) return;
  const wasNew = !state.exists;
  els.save.disabled = true;
  const payload = {
    url: state.page.url,
    title: els.title.value.trim(),
    // 不带 note：笔记归「笔记」视图管，这里绝不动它
    tags: state.selected,
    description: state.page.description || '',
    ogImage: state.page.ogImage || '',
  };
  try {
    const response = await bridgeFetch('/capture', {
      method: 'POST',
      body: JSON.stringify(payload),
    });
    const data = await response.json();
    if (!data.ok) {
      throw new Error(data.error || '保存失败');
    }
    setStatus(wasNew ? '已收藏 · 笔记已开放' : '已更新', 'success');
    // 刚上线时把之前攒的离线收藏一起补录。
    const flushed = await flushOffline();
    await loadPage();
    if (wasNew) setView('note');
    if (flushed > 0) setStatus(`已收藏 · 另补录 ${flushed} 条离线收藏`, 'success');
  } catch (error) {
    if (!state.online) {
      // 桥挂了：本地缓存，不丢这次收藏。下次桥恢复会自动补录。
      await enqueueOffline({ type: 'capture', payload });
      state.dirty = false;
      setStatus('已离线缓存，应用启动后自动补录', 'success');
    } else {
      setStatus(`保存失败：${error.message}`, 'error');
    }
  } finally {
    els.save.disabled = false;
  }
}

// ── 小工具 ──

function setStatus(message, kind = '') {
  els.status.textContent = message;
  els.status.className = kind ? `status ${kind}` : 'status';
  els.status.hidden = false;
}

function disableForm(disabled) {
  els.title.disabled = disabled;
  els.tagInput.disabled = disabled;
  els.note.disabled = disabled;
  els.save.disabled = disabled;
}
