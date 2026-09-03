// Edge 侧边栏：读取当前标签页 → 填标签与批注 → 通过本地桥写入「收藏管理器」。
//
// 桥只监听 127.0.0.1，端口在 17820–17829 之间顺延，扩展按顺序探测
// （扩展读不到本地文件，所以不能靠读端口文件）。

const PORT_START = 17820;
const PORT_END = 17829;

const state = {
  token: '',
  port: null,
  pool: [],
  selected: [],
  page: null,
  tabId: null,
  exists: false,
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

  els.tagInput.addEventListener('keydown', onTagInput);
  els.save.addEventListener('click', onSave);
  els.openOptions.addEventListener('click', () => chrome.runtime.openOptionsPage());

  init();
});

async function init() {
  const stored = await chrome.storage.local.get(['bridgeToken', 'bridgePort']);
  state.token = (stored.bridgeToken || '').trim();
  state.port = stored.bridgePort || null;

  if (!state.token) {
    setStatus('请先在扩展选项页填写本机令牌', 'error');
    disableForm(true);
    return;
  }

  try {
    await bridgeFetch('/ping');
  } catch (error) {
    setStatus('连不上收藏管理器，请先启动应用', 'error');
    disableForm(true);
    return;
  }

  await loadPage();
  // 页面切换 / 刷新时同步热更新当前页内容（仅对当前侧边栏对应的那个标签页响应）。
  chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
    if (changeInfo.status === 'complete' && tabId === state.tabId) {
      loadPage();
    }
  });
  chrome.tabs.onActivated.addListener((activeInfo) => {
    if (activeInfo.tabId !== state.tabId) {
      loadPage();
    }
  });
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

  let lastError = new Error('无法连接到收藏管理器');
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

// ── 读取当前页面 ──

async function readCurrentTab() {
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  if (!tab?.id) {
    throw new Error('找不到当前标签页');
  }
  // 浏览器内部页面（chrome://、edge://、about:、扩展自身页）扩展永远无法注入脚本，
  // 直接抛带标记的错，让上层显示友好提示而不是原始英文报错。
  if (tab.url && /^chrome:|^edge:|^about:|^chrome-extension:|^moz-extension:/i.test(tab.url)) {
    const error = new Error('这个页面无法收藏（浏览器内部页面）');
    error.code = 'PROTECTED';
    throw error;
  }
  // 有 <all_urls> host 权限，注入脚本可读取标题 / 选中文字 / 描述。
  // 极个别注入失败时，再用 tabs API 的 url/title 兜底，保证至少能入库。
  try {
    const [injected] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => {
        const meta = (selector) =>
          document.querySelector(selector)?.content || '';
        const ogTitle = meta('meta[property="og:title"]');
        const ogImage = meta('meta[property="og:image"]');
        const ogDesc = meta('meta[property="og:description"]');
        const metaDesc = meta('meta[name="description"]');
        // 知乎登录后 document.title 会带未读提示，如「(8封私信/10条消息) 标题 - 知乎」。
        // 优先用干净的 og:title；清洗是幂等的，对干净标题无害，所以无条件执行。
        const title = (ogTitle || document.title || '')
          // 去掉开头的未读提示括号（含「私信」或「消息」），如 (8封私信/10条消息)
          .replace(/^[（(][^)）]*(?:私信|消息)[^)）]*[)）]\s*/, '')
          // 去掉结尾的「 - 知乎」
          .replace(/\s*[-–—]\s*知乎\s*$/, '')
          .trim();
        return {
          url: location.href,
          title,
          ogImage,
          description: ogDesc || metaDesc,
          selection: (window.getSelection()?.toString() || '').trim(),
        };
      },
    });
    if (injected?.result) {
      return { id: tab.id, ...injected.result };
    }
  } catch (error) {
    // 落到下面的 tabs 兜底。
  }
  if (tab.url) {
    return { id: tab.id, url: tab.url, title: tab.title || tab.url, description: '', selection: '' };
  }
  throw new Error('读取当前页面失败：拿不到页面地址');
}

async function loadPage() {
  let page;
  try {
    page = await readCurrentTab();
  } catch (error) {
    if (error?.code === 'PROTECTED') {
      setStatus('这个页面无法收藏（浏览器内部页面）', 'error');
    } else {
      setStatus(`读取当前页面失败：${error.message}`, 'error');
    }
    disableForm(true);
    return;
  }
  if (!page?.url) {
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
    tags = (await tagsResponse.json()).tags || [];
    const looked = await itemResponse.json();
    if (looked.exists && looked.item) {
      item = looked.item;
    }
  } catch (error) {
    setStatus(`读取标签池失败：${error.message}`, 'error');
    disableForm(true);
    return;
  }

  state.pool = tags.map((tag) => tag.name);
  if (item) {
    // 已收藏过：回填标签与批注，按钮变「更新」。
    state.exists = true;
    state.selected = item.tags || [];
    els.title.value = item.title || page.title || page.url;
    els.note.value = item.notes || '';
    els.save.textContent = '更新';
  } else {
    state.exists = false;
    state.selected = [];
    els.title.value = page.title || page.url;
    els.note.value = page.selection || '';
    els.save.textContent = '收藏';
  }

  render();
  disableForm(false);
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
      })
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
      })
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

// ── 交互 ──

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
  if (!state.page?.url) return;
  els.save.disabled = true;
  try {
    const response = await bridgeFetch('/capture', {
      method: 'POST',
      body: JSON.stringify({
        url: state.page.url,
        title: els.title.value.trim(),
        note: els.note.value,
        tags: state.selected,
        description: state.page.description || '',
        ogImage: state.page.ogImage || '',
      }),
    });
    const data = await response.json();
    if (!data.ok) {
      throw new Error(data.error || '保存失败');
    }
    state.exists = true;
    els.save.textContent = '更新';
    setStatus('已保存', 'success');
    // 保留几秒让用户确认，然后清空，方便接着收藏下一页。
    setTimeout(resetForm, 3000);
  } catch (error) {
    setStatus(`保存失败：${error.message}`, 'error');
  } finally {
    els.save.disabled = false;
  }
}

function resetForm() {
  els.title.value = state.page?.title || state.page?.url || '';
  els.note.value = '';
  state.selected = [];
  clearStatus();
  render();
}

function setStatus(message, kind = '') {
  els.status.textContent = message;
  els.status.className = kind ? `status ${kind}` : 'status';
  els.status.hidden = false;
}

function clearStatus() {
  els.status.hidden = true;
  els.status.textContent = '';
}

function disableForm(disabled) {
  els.title.disabled = disabled;
  els.tagInput.disabled = disabled;
  els.note.disabled = disabled;
  els.save.disabled = disabled;
}
