# B 站收藏管理器

一个运行在本地的 Bilibili 收藏整理工具。它把 B 站收藏夹中的视频元数据导入本地 SQLite 数据库，并允许你使用标签、分类和批注重新组织这些内容。

## 功能

- B 站扫码登录
- 公开收藏夹链接导入
- 登录账号收藏夹导入
- 分区标签、自定义标签和标签分类
- 标签分类拖拽
- 视频标签编辑
- 视频批注
- 本地视频删除、多选批量删除、按标签删除
- 本地封面缓存
- 视频标题、简介、UP 主和标签检索

## 隐私与数据

- 不下载视频内容。
- 只保存视频元数据和封面图片。
- 登录 Cookie 保存在 Windows 凭据管理器以及应用数据目录中，不会写入仓库。
- SQLite 数据库和封面保存在 Tauri 应用数据目录：

```text
%APPDATA%\com.local.bili-collector\
```

## 开发环境

- Node.js 20+
- Rust stable
- Windows 上的 Microsoft Visual Studio Build Tools

安装依赖：

```powershell
npm.cmd install
```

启动开发环境：

```powershell
npm.cmd run dev -- --host 127.0.0.1
```

然后在另一个终端启动 Tauri：

```powershell
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
