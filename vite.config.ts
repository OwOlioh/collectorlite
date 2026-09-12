import { resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  build: {
    rollupOptions: {
      // 两个入口：收藏库 + 速记面板。速记是独立窗口，不该共用主入口 ——
      // 否则打开它就会把整个收藏库的数据查询一起拉起来。
      input: {
        main: resolve(process.cwd(), "index.html"),
        nowplaying: resolve(process.cwd(), "nowplaying.html")
      }
    }
  },
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: {
      ignored: ["**/src-tauri/target/**"]
    }
  },
  envPrefix: ["VITE_", "TAURI_"]
});
