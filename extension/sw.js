// 后台服务工作者：只负责让点击工具栏图标时打开侧边栏。
// 页面信息的读取、入库都由侧边栏自己做，这里保持无状态（MV3 的 SW 随时会被回收）。

chrome.runtime.onInstalled.addListener(() => {
  // 点图标直接展开侧边栏，而不是弹 popup。失败静默（老版本无此 API）。
  chrome.sidePanel
    .setPanelBehavior({ openPanelOnActionClick: true })
    .catch(() => {});
});

// 侧边栏每次打开时请求一次当前标签页授权（activeTab 需要用户手势激活）。
chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === 'ping') {
    sendResponse({ ok: true });
  }
  return false;
});
