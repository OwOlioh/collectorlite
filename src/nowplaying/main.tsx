import React from "react";
import ReactDOM from "react-dom/client";
import { NowPlayingApp } from "./NowPlayingApp";
// 复用主窗口的设计变量与基础样式，保证深浅色主题和收藏库一致
import "../styles.css";
import "./nowplaying.css";
import { applyTheme, getStoredTheme } from "../lib/theme";

applyTheme(getStoredTheme());

/**
 * 面板是独立窗口，出问题时常表现为「一片空白」—— 没有 devtools 可开，
 * 光看空白分不清是 HTML 没加载、JS 没跑、还是跑了一半抛异常。
 * 所以这里兜一层：把异常直接画进页面，配合 nowplaying.html 里的占位文本形成三级诊断阶梯。
 */
function showFatal(message: string) {
  const root = document.getElementById("root");
  if (!root) return;
  root.innerHTML = "";
  const pre = document.createElement("pre");
  pre.style.cssText =
    "margin:0;padding:12px;font:12px/1.6 ui-monospace,Consolas,monospace;color:var(--danger,#e05260);white-space:pre-wrap;word-break:break-all";
  pre.textContent = `速记面板启动失败：\n${message}`;
  root.appendChild(pre);
}

window.addEventListener("error", (event) => showFatal(event.message));
window.addEventListener("unhandledrejection", (event) =>
  showFatal(String(event.reason))
);

try {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <NowPlayingApp />
    </React.StrictMode>
  );
} catch (error) {
  showFatal(String(error));
}
