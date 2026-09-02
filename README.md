# 收藏管理器

一个运行在本地的多平台收藏整理工具。将 B站、浏览器书签、知乎等平台的收藏内容导入本地 SQLite 数据库，使用标签和分类统一管理。

## 功能

- **多平台导入**：B站（扫码登录/公开链接）、浏览器书签（HTML 文件）、知乎（登录/链接）
- 分区标签、自定义标签和标签分类
- 标签分类拖拽
- 内容标签编辑
- 内容批注
- 本地删除、多选批量删除、按标签删除（删除先进入回收站，默认 7 天保留期内可恢复）
- 来源筛选（B站/浏览器/知乎）
- 内容标题、简介、作者和标签检索
- 浏览器书签 favicon 封面

## 隐私与数据

- 不下载视频或文件内容。
- 只保存元数据和封面图片。
- 登录 Cookie 保存在 Windows 凭据管理器以及应用数据目录中，不会写入仓库。
- SQLite 数据库和封面保存在 Tauri 应用数据目录：

```text
%APPDATA%\com.local.bili-collector\
```

## 开发环境

- **项目路径**：`C:\Users\lioh\Documents\GitHub\bilibili_collector`
- Node.js 20+
- Rust stable
- Windows 上的 Microsoft Visual Studio Build Tools

安装依赖：

```powershell
npm.cmd install
```

启动开发环境（先启动 Vite，再启动 Tauri）：

```powershell
# 终端 1
npm.cmd run dev -- --host 127.0.0.1

# 终端 2
cd src-tauri
cargo run
```

## 检查

```powershell
npm.cmd run test
npm.cmd run build
cargo fmt -- --check
cargo check
cargo test --lib
```

## 新增来源

参见 [DEVELOPMENT.md](DEVELOPMENT.md) 获取完整的开发指南，包括架构说明、踩坑记录和检查清单。

## 打包

```powershell
npm.cmd run tauri build
```

Windows 安装包会生成在：

```text
src-tauri\target\release\bundle\nsis\
```

## 许可

[MIT](LICENSE)