# collectorlite（bilibili_collector）· 项目答辩文档

> 面向答辩的功能全景、架构设计与工程实践说明
> 版本：v0.1.0　　文档日期：2026-09-05

---

## 一、定位

**一款运行在 Windows 桌面的本地优先（Local-First）多平台收藏聚合工具**——把散落在 B 站、知乎、CSDN、GitHub、浏览器书签里的收藏，统一汇聚到本地 SQLite 库中，提供跨源检索、标签分类、批注与 Obsidian 联动。

### 核心取舍：只存元数据与链接，不下载内容本体

理由：单条收藏仅需约 124 字节，283 条数据总库仅 ~7 MB（含封面）；若下载视频/正文，体积会爆炸式增长且涉及版权与隐私。

**本项目定位是「索引与入口」，而不是「离线归档」**——解决的是"找到它"，而不是"存下它"。用户点开卡片仍会跳回原站观看。

### 项目名片

- 项目名：collectorlite（原名「收藏管理器」；工程目录与仓库仍沿用历史名 `bilibili_collector`）
- 技术栈：Tauri 2（Rust 后端）+ React 18 + TypeScript + Vite + SQLite
- 仓库：https://github.com/OwOlioh/billcollector2.0
- 分发：GitHub Release（NSIS 安装器 + 免安装 zip + Edge 扩展 zip）

---

## 二、设计动机

| 痛点 | 现状 | 本项目的做法 |
|---|---|---|
| **收藏散落** | 视频在 B 站、文章在知乎、代码在 GitHub，各平台各自为政 | 统一数据模型，一个库管所有来源 |
| **平台会失效** | 内容被删、账号被封、平台改版，收藏就没了 | 元数据永久留在本地，源链接失效也保留记录 |
| **无法统一检索** | 想找"之前存过的那篇 Rust 文章"要挨个平台翻 | 本地全文检索 + 标签 + 来源 + 排序组合筛选 |
| **不敢整理** | 一删就永久丢失，误删成本高 | 回收站软删除 + 可配置保留期 + 一键恢复 |
| **存了≠消化** | 收藏夹变成"数字墓地" | 批注 + Obsidian 笔记联动，把收藏接进知识工作流 |

---

## 三、架构

### 3.1 分层架构

```
┌──────────────────────────────────────────────────────────────┐
│  前端（WebView）React 18 + TypeScript + Vite                 │
│  LibraryPage / ImportPage / SettingsPage / TrashPage         │
│  TagManagerPanel · VideoCard · VideoNoteEditorModal ...      │
└─────────────── invoke（IPC，约 60 个命令）───────────────────┘
                          │
┌──────────────────────────────────────────────────────────────┐
│  Rust 后端（Tauri 2 进程）                                    │
│  ┌────────────┬─────────────┬───────────┬─────────────────┐  │
│  │ commands.rs│ source/     │ db.rs     │ obsidian.rs     │  │
│  │ 命令层     │ SourceAdapter│ 数据访问  │ 笔记单向同步     │  │
│  ├────────────┼─────────────┼───────────┼─────────────────┤  │
│  │ capture.rs │ proxy.rs    │ wbi.rs    │ models/error    │  │
│  │ 本地 HTTP 桥│ 系统代理解析 │ B站签名   │ 数据模型/错误    │  │
│  └────────────┴─────────────┴───────────┴─────────────────┘  │
└──────────────────────────────────────────────────────────────┘
        │                    │                    │
   SQLite(sqlx 0.8)    reqwest(经代理)      tiny_http 本地桥
   8 个迁移版本          各平台 API         127.0.0.1:17820-17829
```



### 3.3 分层职责

| 层 | 文件 | 职责 |
|---|---|---|
| 命令层 | `commands.rs`（~1500 行） | 约 60 个 Tauri 命令，前端唯一入口 |
| 数据源层 | `source/{bilibili,zhihu,csdn,github,browser,proxy}.rs` | 统一 `SourceAdapter` trait，各平台协议适配 |
| 数据层 | `db.rs`（~1400 行） | SQLite 读写、FTS 维护、软删除语义 |
| 笔记层 | `obsidian.rs`（~380 行） | Markdown 生成、分区托管、URI 唤起 |
| 桥接层 | `capture.rs`（~900 行） | 本地 HTTP 服务，供浏览器扩展调用 |

### 3.4 数据模型

**主要表结构**

| 表 | 说明 |
|---|---|
| `items` | 收藏本体，20 个字段，`(source, external_id)` 唯一约束 |
| `tags` | 标签（含 `namespace`、`category_id`、`color`） |
| `tag_categories` | 标签分类（颜色 + 排序位置） |
| `item_tags` | 收藏与标签的多对多绑定 |
| `import_runs` / `import_run_items` | 导入批次审计，可追溯每次导入的来龙去脉 |
| `items_fts` | FTS5 虚拟表（contentless），支撑全文检索 |
| `_sqlx_migrations` | 迁移版本与校验记录 |

**关键字段演进（8 个迁移版本）**

```
0001 初始建表（items / tags / item_tags）
0002 标签分类 tag_categories
0003 清理 UP 主噪声标签
0004 重建 FTS5 索引
0005 cover_local_path（封面本地化）
0006 notes（批注）
0007 deleted_at（软删除 / 回收站）
0008 obsidian_path（笔记联动）
```

> 全部 8 个迁移都是 `ALTER TABLE ADD COLUMN` 或纯数据操作，**没有一次破坏性重建**，老用户的库可以在不丢数据的前提下平滑升级。


### 3.5 工程实践

| 维度 | 做法 |
|---|---|
| **测试** | 55 个 Rust 单元测试（含 URL 解析、WBI 签名、路径边界、列完整性回归），`cargo test` 全绿 |
| **类型安全** | 前后端均严格类型化，`tsc --noEmit` 零错误；Rust 侧统一 `AppError` 错误模型 |
| **迁移管理** | sqlx migrate，8 个版本化迁移，全部向后兼容 |
| **CI/CD** | GitHub Actions + `tauri-action`：push `v*` tag 自动构建 Windows 安装包并上传 Release（含权限声明、Rust 缓存） |
| **分发** | NSIS 安装器 + 免安装 zip + Edge 扩展 zip，三件套 |
| **代码规模** | Rust 17 文件 / 约 8.6k 行；前端 TS+TSX 约 5.6k 行 |
| **安全** | 本地桥 token 鉴权 + Host 回环校验 + CORS 预检；写文件路径防越界；凭据仅存本地 `%APPDATA%`，从不进入仓库与安装包 |

### 3.6 技术栈一览

**后端**：Rust · Tauri 2 · sqlx 0.8 (SQLite) · reqwest · tokio · serde · serde_yaml · scraper · tiny_http · webbrowser · windows-sys (ShellExecuteW)

**前端**：React 18 · TypeScript · Vite · lucide-react · react-virtuoso · qrcode.react

**扩展**：Edge Extension Manifest V3（sidePanel + service worker，零依赖原生 JS）

**工程**：GitHub Actions · tauri-action · sqlx migrate · cargo test（55 个）

---

## 四、功能

### 4.1 多源导入（5 个平台适配器 + 1 个兜底）

所有适配器实现同一个 trait，新增一个平台只需实现 3 个方法：


| 来源 | 支持采集的内容 | 登录方式 |
|---|---|---|
| **B 站** | 自己的全部收藏夹、公开收藏夹（分享链接）、合集 / 系列（4 类链接）、**图文动态收藏**（独立接口） | 扫码登录（二维码轮询），凭据加密存本地 |
| **知乎** | 收藏夹（回答 / 文章 / 想法 / 视频）、单条回答·文章·想法 | 手动粘贴 Cookie 或浏览器登录辅助 |
| **CSDN** | 收藏夹、单篇文章 | Cookie |
| **GitHub** | Star 列表（GraphQL/REST）、单个仓库 | 公开读取（无需登录） |
| **浏览器书签** | 解析 Chrome / Edge 导出的 bookmarks HTML（保留文件夹层级） | 无需登录 |
| **任意网页（扩展）** | 通过 Edge 扩展侧边栏一键入库，按域名智能路由解析元数据 | 无（本地桥 token 鉴权） |

**导入流程统一为三步**：选来源 → 预览并选择标签 → 执行导入。
导入采用**白名单机制**：只有用户在预览页勾选/打过标签的条目才会真正入库，避免"一键把 3000 条收藏全灌进来"。

### 4.2 统一收藏库

- **双视图**：网格（封面墙）/ 列表（紧凑信息流），可切换
- **检索**：标题 / 简介 / UP 主 / 分区 / **标签名** 全字段模糊匹配，背后是 SQLite FTS5 虚拟表
- **筛选**：来源多选（B 站 / 知乎 / CSDN / GitHub / 书签）+ 标签多选（支持 AND / OR 两种标签匹配模式）
- **排序**：按收藏时间 / 发布时间倒序
- **批量操作**：多选后批量打标签、批量导出到 Obsidian、批量删除（进回收站）
- **虚拟滚动**：`react-virtuoso`，上千条收藏滚动无卡顿
- **封面本地化**：封面图下载到 `%APPDATA%/covers/` 并用 `convertFileSrc` 本地加载（原因见 §5.2）

### 4.3 标签体系（三层结构）

```
分类（tag_categories，可定义颜色与排序）
  └── 标签（tags，带 namespace 命名空间）
        └── 绑定（item_tags，多对多）
```

- **命名空间**：标签带 `namespace` 字段，区分「用户自建」与「来源自动带入」（如 B 站原收藏夹名），互不污染
- **分类管理**：独立的标签管理面板，支持新建 / 重命名 / 删除分类、**拖拽把标签归入分类**、按分类着色
- **标签合并**：`merge_tags`——把多个相似标签合并为一个（如 `Rust` / `rust-lang` → `Rust`），自动迁移全部绑定关系
- **批量打标签**：收藏库里多选 → 一次性挂标签；导入预览阶段也可预挂标签

### 4.4 批注 + Obsidian 单向联动

**批注**：每条收藏可写 Markdown 批注（`notes` 字段），弹窗内支持编辑 / 预览双模式。

**Obsidian 联动**（本项目最有特色的能力）：

```
SQLite 批注  ──单向推送──▶  Obsidian Markdown 笔记
   权威源                      （vault 内 收藏/xxx.md）
```

- 生成的 md 带 YAML frontmatter（标题、来源、作者、链接、收藏时间、标签），正文用 `serde_yaml` 安全序列化——标题里有 `:` `#` `[` 也不会破坏格式
- **分区托管**：正文用 `<!-- collector:notes:start/end -->` 圈出 app 的责任区，**只覆盖这一段**，你在 Obsidian 里额外写的内容永不被冲掉
- **单向**：只从 app 推向 Obsidian，不回读。理由：批注是轻量内容，双向自动同步的冲突处理成本远大于收益，且存在静默覆盖风险（用户已确认不做双向）
- **路径安全**：`subdir` 即使填绝对路径或 `..`，也会在写文件前被拦截（有回归测试）
- **删除不联动**：app 里删收藏、清回收站，**不会删除** vault 里的 md

### 4.5 回收站（软删除）

| 能力 | 说明 |
|---|---|
| 软删除 | `delete_item` 只置 `deleted_at` 时间戳，行仍在库里 |
| 恢复 | 单条 / 批量恢复，`deleted_at` 置回 NULL |
| 保留期 | 7 / 15 / 30 天可选（默认 7 天，存 localStorage） |
| 自动清理 | app 启动时调 `auto_purge_trash`，超期条目**硬删除**并同步删封面文件 |
| 彻底删除 | 回收站页可手动 `purge` / `empty_trash` |

> 设计要点：删除的"不可逆"被推迟了一个可配置的窗口期，误删可救；同时保留期机制保证磁盘不会无限增长。

### 4.6 备份与迁移

- **JSON 备份导出**：`export_collection` 导出含 `format_version` 的版本化 JSON，每条 item 携带全部标签（含分类名）
- **导入恢复**：`import_collection` 按 `(source, external_id)` 判重，重复条目跳过；**并按分类名自动重建分类**
- **已知边界**：分类的**自定义颜色**不随 JSON 导出（重建时用默认色）——这是明确的取舍，已记录为后续可选改进项

### 4.7 浏览器扩展（Edge MV3）

- 侧边栏形态（`chrome.sidePanel`），浏览网页时随时呼出
- 本地 HTTP 桥：`127.0.0.1:17820~17829`（端口占用自动顺延），四个端点 `/ping` `/tags` `/item` `/capture`
- **安全设计**：token 鉴权 + Host 头回环校验 + CORS 预检处理，防止恶意网页往你的库里写东西
- **智能路由**：按域名分派到对应适配器解析丰富元数据（B 站单视频/合集、知乎回答/文章/想法、CSDN、GitHub 仓库），解析失败安全回退为通用网页存档（`bk_<sha256(url)>`）
- 扩展会检测该 URL 是否已收藏，避免重复入库

### 4.8 设置与个性化

- 主题：浅色 / 深色 / 跟随系统（CSS 变量驱动）
- 回收站保留期配置
- 账号状态查看与登出（B 站 / 知乎）
- 封面重新缓存（修复失效封面）
- 浏览器扩展 token 显示 / 复制 / 重新生成
- Obsidian 仓库选择与联动开关

---

## 五、开发过程中遇到的技术难点

### 5.1 跨源统一与去重

**问题**：B 站的 BV 号、知乎的回答 ID、GitHub 的仓库全名，ID 体系完全不同，如何统一？

**方案**：
- 统一 `ExternalItem` 中间模型，各适配器负责把平台字段翻译成统一字段
- 用 **`(source, external_id)` 复合唯一键**去重——不同平台的 ID 天然隔离，同一平台重复导入自动命中
- 写入走 `upsert_item`：`INSERT ... ON CONFLICT DO UPDATE`，重复导入只更新时间戳、不产生重复行

### 5.2 封面必须本地化（Webview 的隐藏坑）

**现象**：直接从 CDN 加载封面，界面全是空白图。

**根因**：Tauri 的 WebView **不继承宿主进程的代理设置**。中国大陆网络环境下，WebView 直连 B 站 / 知乎 CDN 必然失败。

**方案**：导入流程里的 `cache_item_covers` 用 Rust 侧 reqwest（走系统代理）把封面下载到 `%APPDATA%/covers/`，数据库记 `cover_local_path`；前端 `resolveCoverUrl` 优先用本地文件（`convertFileSrc` 转换），远程 URL 仅作兜底。

### 5.3 统一代理解析

`source/proxy.rs::resolve_system_proxy()` 按 **环境变量 → Windows 注册表 → 本机常见端口兜底** 三级探测，保证无论用户是直接双击 exe、还是从 IDE 终端启动，都能正确出网。这是国内网络环境下的必备工程细节。

### 5.4 Obsidian 路径检查的 Windows 陷阱

**现象**：配置好仓库后点导出，永远提示"没有批注"，实际一条都没写出去。

**根因**：`ensure_within_vault` 用 `canonicalize()` 做路径前缀比较。Windows 上 `canonicalize()` 会给**已存在**的目录加上 `\\?\` 前缀（`\\?\C:\Users\...`），而目标 md 首次写入前父目录还不存在、`canonicalize` 失败退回**无前缀**原始路径——两边形式不一致，`starts_with` 恒为 false，所有写入都被误判为"越界"而拒绝。

**方案**：改为**纯词法规范化**（处理 `.` 与 `..`，不碰磁盘）后比较，彻底绕开前缀问题；并补 4 个回归测试（子路径放行 / 仓库外拒绝 / `..` 逃逸拒绝 / 绝对路径逃逸拒绝）。同时让导出命令**不再静默吞错误**，真实失败原因能 toast 到前端。


### 5.5 sqlx 运行时缺列导致整页空白

**现象**：收藏库与回收站完全空白，但数据库里数据完好。

**根因**：`sqlx::query_as::<_, ItemRow>()` 是**运行时按列名解码**，不是编译期检查。给 `ItemRow` 加了 `obsidian_path` 字段后，`search_items` / `list_trash` 的 SELECT 列清单漏改，`cargo check` 照样通过，运行时抛 `ColumnNotFound` → 前端拿到错误 → 整页空白。

**方案**：
- 抽出 `ITEM_ROW_COLUMNS` 常量，**所有 ItemRow 查询统一引用**，杜绝再漏列
- 加回归测试 `item_row_queries_return_every_column`，把列完整性钉死在 CI 里

### 5.6 启动闪退：迁移校验的行尾漂移（跨环境字节一致性）

**现象**：GitHub Actions 构建的安装包一启动就闪退，本地开发版却完全正常；修完之后本地 `cargo run` 又开始报同样的错——**同一类问题在两个环境轮流爆发**。

```
migration N was previously applied but has been modified
```

**根因**：`sqlx::migrate!` 在**编译期**把 `.sql` 按**字节**嵌入 exe，运行时对数据库 `_sqlx_migrations` 里记录的 checksum（sha384）逐一比对，从版本号最小的开始。CRLF 与 LF 在 SQL 语义上完全等价，但**字节不同 → sha384 不同 → 判定"迁移被篡改" → panic**。

三个来源的字节只要有一个不一致就炸：

| 来源 | 字节由谁决定 |
|---|---|
| 数据库记录的 checksum | **当初建库那台机器**上的文件 |
| CI 构建的 exe | CI runner checkout 出来的文件 |
| 本地 `cargo run` | 本地磁盘上的文件 |

因此它**只发生在"老数据库 + 新 exe"的升级路径**上——全新安装的用户不会遇到（建库即用当前字节写入，天然一致）。

**方案**（三层，前两层治标，第三层治本）：
1. 仓库加 `.gitattributes`：`*.sql text eol=lf`，统一规范化策略
2. 对既有数据库：把历史 checksum 重算为 LF 版（备份后一次性修复）
3. **运行时自愈**：`db::heal_migration_line_endings` 在 `migrator.run()` 之前执行——把当前 SQL 分别归一化成**全 LF / 全 CRLF** 再各算一次 sha384，**只有命中其中之一才认定是行尾漂移**并更新 checksum；两者都不匹配说明 SQL 内容真被改了，**不做任何修改**，交由 sqlx 照常 panic

第 3 层让"老库 + 新 exe"无论哪一侧是 CRLF 都能自愈，同时**完整保留 sqlx 的防篡改语义**——只放行确认过的纯行尾差异。

**工程教训**：
- **CI 构建成功 ≠ 能运行**，发布产物必须实际下载运行验证一遍
- `.gitattributes` 的 `eol=lf` **只对新的 checkout/add 生效**，不会改写磁盘上已存在的老文件。加完配置必须 `rm` 后 `git checkout --` 强制重写；`git add --renormalize` + `git checkout --` 在这里**无效**——git 的双向 filter 会判定「工作区 CRLF」与「索引 LF」内容相同而直接跳过
- **自愈类逻辑属于"静默修复"，必须配反向测试**（真篡改不得放过），否则安全边界形同虚设。本节对应 5 个单元测试 + 真实数据库上的注入/篡改双向验证

---

## 六、已知的局限

### 6.1 当前局限

| 项 | 说明 |
|---|---|
| 只做索引，不做归档 | 原站删稿 / 链接失效后，本地只剩元数据与失效链接，无法找回内容本体（见 §1 核心取舍） |
| 平台依赖 | B 站 WBI 签名、知乎 Cookie、GitHub Token 均可能过期或接口变更，需持续适配 |
| 平台覆盖 | 目前仅 Windows x64 有构建产物（macOS / Linux 需补打包与签名配置） |
| 代码签名 | 未签名，SmartScreen 会提示"未知发布者"；消除需购买签名证书 |
| 扩展分发 | 采用开发者模式加载；上架 Edge 外接程序商店是长期方案 |
| 批注编辑器 | 支持 Markdown 但**未做渲染预览美化**（先前记录的 P2 项） |
| 备份粒度 | JSON 备份不含分类颜色 / 排序位置 |



### 6.3 后续可做

- macOS / Linux 构建与打包配置
- 批注编辑器的 Markdown 渲染预览
- 扩展上架 Edge 外接程序商店
- 接入AI辅助收藏库管理与知识检索

---


