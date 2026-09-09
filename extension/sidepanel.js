// Edge 侧边栏：读当前页 → 收藏 / 笔记两个视图 → 通过本地桥写入「collectorlite」。
//
// 桥只监听 127.0.0.1，端口在 17820–17829 之间顺延，扩展按顺序探测
// （扩展读不到本地文件，所以不能靠读端口文件）。
//
// 两个视图共享同一条收藏：
//   「收藏」= 标题 + 标签（提交到 /capture）
//   「笔记」= items.notes（提交到 /note，带乐观锁）
// 笔记不再出现在收藏视图里 —— 同一个字段不该有两个编辑器，否则必然互相覆盖。

const PORT_START = 17820;
const PORT_END = 17829;
const AUTOSAVE_MS = 1200;

const state = {
  token: '',
  port: null,
  pool: [],
  selected: [],
  page: null,
  tabId: null,
  exists: false,
  item: null, // { id, source, title, notes, tags, obsidianPath, updatedAt }
  view: 'collect',
  noteMode: 'edit', // 'edit' | 'preview'
  conflict: null, // { remoteNotes, remoteUpdatedAt }
  obsidian: null, // { enabled, vaultPath }
  savedAt: null,
  dirty: false,
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
  els.saveDot = document.getElementById('save-dot');
  els.saveText = document.getElementById('save-text');
  els.cfOverwrite = document.getElementById('cf-overwrite');
  els.cfReload = document.getElementById('cf-reload');

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
  els.mkTime.addEventListener('click', insertTimestamp);
  els.mkPos.addEventListener('click', insertPosition);
  els.mkPreview.addEventListener('click', () =>
    setNoteMode(state.noteMode === 'edit' ? 'preview' : 'edit'),
  );
  els.openObsidian.addEventListener('click', openInObsidian);
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

  // 侧边栏随时会被卸载（切站点、关面板）—— 走之前把没保存的内容推出去。
  // MV3 下普通 fetch 会被杀掉，keepalive 才能发出去；token 走查询参数，
  // 这样将来即使降级到 sendBeacon 也不会丢鉴权。
  const flush = () => flushSave();
  window.addEventListener('pagehide', flush);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') flush();
  });
}

async function init() {
  const stored = await chrome.storage.local.get(['bridgeToken', 'bridgePort', 'sidebarView']);
  state.token = (stored.bridgeToken || '').trim();
  state.port = stored.bridgePort || null;
  state.view = stored.sidebarView === 'note' ? 'note' : 'collect';
  state.noteMode = stored.sidebarNoteMode === 'preview' ? 'preview' : 'edit';

  if (!state.token) {
    setStatus('请先在扩展选项页填写本机令牌', 'error');
    setView(state.view);
    disableForm(true);
    return;
  }

  try {
    await bridgeFetch('/ping');
  } catch (error) {
    setStatus('连不上collectorlite，请先启动应用', 'error');
    setView(state.view);
    disableForm(true);
    return;
  }

  setView(state.view);
  await loadPage();

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
  throw lastError;
}

function portRange() {
  const ports = [];
  for (let port = PORT_START; port <= PORT_END; port += 1) {
    ports.push(port);
  }
  return ports;
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
  try {
    const [tagsResponse, itemResponse] = await Promise.all([
      bridgeFetch('/tags'),
      bridgeFetch(`/item?url=${encodeURIComponent(page.url)}`),
    ]);
    tags = ((await tagsResponse.json()).tags || []).map((tag) => tag.name);
    const looked = await itemResponse.json();
    if (looked.exists && looked.item) {
      item = looked.item;
    }
  } catch (error) {
    setStatus(`读取收藏信息失败：${error.message}`, 'error');
    disableForm(true);
    return;
  }

  // Obsidian 联动状态：决定要不要显示「在 Obsidian 中打开」。
  try {
    const response = await bridgeFetch('/obsidian/status');
    state.obsidian = await response.json();
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
  render();
  renderNoteChrome();
  setSaved(item ? '已保存' : '');
  disableForm(false);
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
  const open = Boolean(state.item);
  els.nbLocked.hidden = open;
  els.nbOpen.classList.toggle('on', open);
  // 没有视频就不显示「+ 时间戳」——一个常年灰着的按钮只是噪音。
  els.mkTime.hidden = !state.page || !state.page.hasVideo;
  renderObsidianButton();
  renderMarkers();
}

function renderObsidianButton() {
  const configured = state.obsidian && state.obsidian.enabled;
  els.openObsidian.hidden = !configured;
  if (!configured) return;
  const hasNote = Boolean(state.item && state.item.obsidianPath);
  els.openObsidian.disabled = !hasNote;
  els.openObsidian.title = hasNote ? '' : '这条还没同步到 Obsidian，先写点笔记保存一次';
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

function parseMarkers() {
  const out = [];
  (els.note.value || '').split('\n').forEach((line) => {
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

function renderMarkers() {
  const list = parseMarkers();
  els.mkCount.textContent = list.length ? `${list.length} 条` : '';
  els.mkList.textContent = '';
  if (!list.length) {
    const empty = document.createElement('div');
    empty.className = 'mk-empty';
    empty.textContent = '还没有标记。';
    els.mkList.appendChild(empty);
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
    els.mkList.appendChild(button);
  }
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
function insertLine(text) {
  const textarea = els.note;
  const start = textarea.selectionStart;
  const end = textarea.selectionEnd;
  const before = textarea.value.slice(0, start);
  const after = textarea.value.slice(end);
  const prefix = before.length && !/\n$/.test(before) ? '\n' : '';
  textarea.value = before + prefix + text + after;
  const caret = before.length + prefix.length + text.length;
  textarea.focus();
  textarea.setSelectionRange(caret, caret);
  renderMarkers();
  markDirty();
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
function renderPreview() {
  const box = els.notePreview;
  box.textContent = '';
  const lines = (els.note.value || '').split('\n');
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

async function insertTimestamp() {
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
  insertLine(`- [${formatTime(seconds)}](${url}) `);
  if (state.noteMode === 'preview') setNoteMode('edit'); // 插入后回到编辑，方便接着写说明
  setStatus(`已插入 ${formatTime(seconds)}，直接接着打字`, 'success');
}

async function insertPosition() {
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
  insertLine(`- [位置](${url}) `);
  if (state.noteMode === 'preview') setNoteMode('edit');
  setStatus(
    selection.anchorId ? `已用锚点 #${selection.anchorId} + 文本片段` : '没有可用锚点，已用文本片段',
    'success',
  );
}

// ── 保存笔记 ──

let saveTimer = null;

function markDirty() {
  if (!state.item) return;
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
  if (!state.item || !state.page) return;
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
  } catch (error) {
    setSaved('保存失败', true);
    setStatus(`笔记保存失败：${error.message}`, 'error');
  }
}

// 页面即将卸载时把未保存内容推出去。普通 fetch 会被杀掉，
// keepalive 能让请求活过页面销毁；token 走查询参数，桥本身就支持这种鉴权。
function flushSave() {
  if (!state.dirty || !state.item || !state.page || !state.port) return;
  const body = JSON.stringify({
    url: state.page.url,
    note: els.note.value,
    baseUpdatedAt: state.item.updatedAt === undefined ? null : state.item.updatedAt,
  });
  try {
    fetch(`http://127.0.0.1:${state.port}/note?token=${encodeURIComponent(state.token)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body,
      keepalive: true,
    });
  } catch (error) {
    // 尽力而为：实在发不出去，草稿还在 storage 里兜底
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

async function openInObsidian() {
  if (!state.page) return;
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
  try {
    const response = await bridgeFetch('/capture', {
      method: 'POST',
      body: JSON.stringify({
        url: state.page.url,
        title: els.title.value.trim(),
        // 不带 note：笔记归「笔记」视图管，这里绝不动它
        tags: state.selected,
        description: state.page.description || '',
        ogImage: state.page.ogImage || '',
      }),
    });
    const data = await response.json();
    if (!data.ok) {
      throw new Error(data.error || '保存失败');
    }
    setStatus(wasNew ? '已收藏 · 笔记已开放' : '已更新', 'success');
    await loadPage();
    if (wasNew) setView('note');
  } catch (error) {
    setStatus(`保存失败：${error.message}`, 'error');
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
