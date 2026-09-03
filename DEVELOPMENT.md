# 收藏管理器 - 新增来源开发指南

本文档记录了在开发 B站 → 浏览器书签 → 知乎 → CSDN → GitHub Stars 五个来源过程中积累的经验和踩过的坑，供后续开发新来源时参考。其中 CSDN 收藏夹与 GitHub Stars 已严格按本文档的步骤成功落地，相关来源特有的经验见第六节。

---

## 一、架构概览

```
前端 (React/TS)                   后端 (Rust/Tauri)
┌─────────────────┐               ┌──────────────────────┐
│ ImportPage.tsx   │── invoke ──→│ commands.rs           │
│   mode="xxx"     │               │   preview_xxx_import  │
│   preview/execute│               │   execute_xxx_import  │
├─────────────────┤               ├──────────────────────┤
│ LibraryPage.tsx  │               │ source/xxx.rs         │
│   来源筛选按钮    │               │   impl SourceAdapter  │
├─────────────────┤               ├──────────────────────┤
│ api.ts           │               │ db.rs                 │
│   命令封装       │               │   upsert_item (通用)  │
├─────────────────┤               ├──────────────────────┤
│ types.ts         │               │ state.rs              │
│   类型定义       │               │   AppState 注册       │
└─────────────────┘               └──────────────────────┘
```

**核心设计原则**：所有来源都通过 `SourceAdapter` trait 统一接口，`ExternalItem` 是统一的数据中间层，`source` 字段区分来源。

---

## 二、开发新来源的完整步骤

### 第 1 步：后端实现 SourceAdapter

**文件**：`src-tauri/src/source/{new_source}.rs`

**模板**：
```rust
use async_trait::async_trait;
use crate::error::AppError;
use crate::models::{CollectionInfo, ExternalItem};
use crate::source::SourceAdapter;

pub struct NewSourceClient {
    client: reqwest::Client,
    cookie: RwLock<Option<String>>,
}

impl NewSourceClient {
    pub fn new() -> Result<Self, AppError> {
        // 创建 HTTP 客户端
        // 注意：如果需要 cookie 持久化，使用 cookie_store
        let client = reqwest::Client::builder()
            .cookie_store(true)  // 需要 features = ["cookies"]
            .build()?;
        Ok(Self { client, cookie: RwLock::new(None) })
    }

    pub fn set_cookie(&self, cookie: Option<String>) { ... }
    pub fn get_cookie(&self) -> Option<String> { ... }

    /// 构建请求头（必须模拟浏览器）
    fn build_headers(cookie: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, ...);
        headers.insert(REFERER, ...);
        // 重要：添加 accept, accept-language 等头防止风控
        if let Some(cookie_str) = cookie {
            headers.insert(COOKIE, ...);
        }
        headers
    }
}

#[async_trait]
impl SourceAdapter for NewSourceClient {
    // 四个必须实现的方法：
    async fn list_collections(&self) -> Result<Vec<CollectionInfo>, AppError>;
    async fn resolve_collection(&self, input: &str) -> Result<CollectionInfo, AppError>;
    async fn fetch_collection(&self, collection: &CollectionInfo) -> Result<Vec<ExternalItem>, AppError>;
    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError>;
}
```

### 第 2 步：注册模块

- `src-tauri/src/source/mod.rs`：添加 `pub mod {new_source};`
- `src-tauri/Cargo.toml`：添加需要的依赖（如 `regex`、`scraper` 等）
- `src-tauri/src/state.rs`：在 `AppState` 中注册客户端，添加 cookie 持久化
- `src-tauri/src/commands.rs`：添加 `preview_xxx_import`、`execute_xxx_import` 等命令
- `src-tauri/src/lib.rs`：在 `invoke_handler` 中注册命令

### 第 3 步：前端适配

- `src/types.ts`：添加 `ImportMode`（如 `"csdn"`）
- `src/lib/api.ts`：添加 API 调用方法
- `src/components/ImportPage.tsx`：添加来源卡片 + 表单 + 预览/执行逻辑
- `src/components/LibraryPage.tsx`：来源筛选按钮

---

## 三、⚠️ 关键踩坑经验

### 3.1 ExternalItem 的 external_id 设计

**这是最重要的设计决策，直接影响标签分配和去重逻辑。**

**规则**：
- `external_id` 必须在来源内**唯一**，因为 `upsert_item` 按 `(source, external_id)` 去重
- **不要用 URL 作为 external_id**——同一 URL 可能出现在多个收藏夹中，导致去重跳过
- 用平台 API 返回的**内容 ID**（如知乎的 `content.id`、B站的 BV号）
- **必须确保前端 preview 中的 `externalId` 和后端 fetch 中的 `external_id` 完全一致**，否则 `item_tag_assignments` 匹配失败

**踩坑记录**（知乎）：
- 知乎 API 返回的 item 顶层只有 `["content", "created"]` 两个字段，没有顶层 `id`
- 内容 ID 在 `content.id` 中，且可能是数字类型（非字符串）
- 需要用 `json_value_to_string` 同时处理数字和字符串

### 3.2 标签分配的正确逻辑

**前后端 external_id 必须一致**，否则标签匹配失败。

**B站模式**（preview → execute 走同一个 API）：
```
前端 preview: 后端返回 ExternalItem → 转 VideoItem → externalId 一致
前端 execute: 构建 ItemTagAssignment { externalId, tagSpecs }
后端 execute: assignments.get(item.external_id) → 匹配成功
```

**浏览器模式**（前端本地解析 → 后端独立命令）：
```
前端 parse: SHA256(URL) → externalId = "bk_" + hash
后端 parse: 同样的 SHA256(URL) → external_id = "bk_" + hash
→ 必须一致！
```

**注意**：执行导入时，如果 `assignments.get()` 找不到匹配，**不要 fallback 到全局 `tag_specs`**，否则所有 item 会共享标签。应该 fallback 到空数组。

### 3.3 API 认证与登录

**Cookie 保存**：
- 文件路径：`{data_dir}/{source}_cookie.txt`
- 凭据管理器：`keyring` crate（Windows Credential Manager）
- 登录状态检查：前端组件加载时必须调用 `xxx_profile` 命令

**Cookie 格式**：
- 不同平台需要不同的 cookie。知乎需要 `z_c0` + `d_c0`，仅 `z_c0` 不够
- 平台可能对 `HttpOnly` cookie 做限制，`document.cookie` 读不到
- 403 错误通常是 cookie 过期或风控，需要引导用户重新获取

**登录方式优先级**：
1. 平台有扫码登录 API → 实现扫码（最友好）
2. 平台没有 → 引导用户手动复制 cookie
3. WebView 内置登录 → 跨域限制导致无法读取 cookie，不可行

### 3.4 请求头与反爬

**必须模拟浏览器**：
```rust
headers.insert(USER_AGENT, "Mozilla/5.0 ... Chrome/120.0 ...");
headers.insert(REFERER, "https://平台域名/");
headers.insert("accept", "application/json, text/plain, */*");
headers.insert("accept-language", "zh-CN,zh;q=0.9");
```

**注意**：
- 知乎 403 常常是因为缺少 `accept` 或 `x-requested-with` 头
- 分页请求之间加 `sleep(200ms)` 防止触发频率限制

### 3.5 JSON 解析的健壮性

**API 返回的字段可能是数字或字符串**：
```rust
fn json_value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
```

**不要假设字段一定存在**，大量使用 `unwrap_or("")` 和 `as_str().unwrap_or("")`。

### 3.6 前端状态隔离

**不同来源必须使用独立的状态变量**：
```tsx
// B站
const [profile, setProfile] = useState(...);
const [collections, setCollections] = useState(...);
// 知乎
const [zhihuProfile, setZhihuProfile] = useState(...);
const [zhihuCollections, setZhihuCollections] = useState(...);
```

**踩坑记录**：共用状态变量会导致 B站链接出现在知乎输入框、B站登录状态覆盖知乎等问题。

### 3.7 前端 UI 合并

**登录和链接输入应该合并到同一个来源卡片中**，而不是分成两个卡片。通过分隔线 + 标签区分：

```tsx
{/* 登录区域 */}
<button>扫码登录</button>
<select>选择收藏夹</select>

<div className="import-section-divider" />

{/* 链接输入区域 */}
<label>或者粘贴收藏夹链接</label>
<input placeholder="https://..." />
<button>解析</button>
```

### 3.8 来源筛选按钮

在 `LibraryPage.tsx` 中添加来源筛选按钮，使用 `source_filter` 容器。按钮样式统一：

```tsx
<button className={filters.sources.includes("xxx") ? "is-active" : ""}
        onClick={() => /* toggle source in filters.sources array */}>
  <Icon size={15} />
</button>
```

- 不选 = 显示全部
- 可以多选
- 后端 `db.rs` 的 `search_items` 已支持 `sources` 过滤

### 3.9 数据库

**不需要新增迁移**。现有 `items` 表已通过 `source` 字段（如 `"bilibili"`、`"browser"`、`"zhihu"`）支持多来源，`external_id` 是通用字符串字段。

### 3.10 错误消息

`AppError::AuthRequired` 的 Display 消息已改为通用 `"需要登录后才能获取内容，请先登录"`，不再写死"B站"。

### 3.11 双入口来源（B站/知乎/CSDN）每个都要有两个独立按钮

**问题背景**：B站、知乎、CSDN 都有两种导入入口——「登录/用户名收藏夹」和「粘贴公开链接」。用户要求这三个平台**各自分别有**两个「预览并配置标签」按钮（登录收藏夹 / 收藏夹链接），且三平台按钮的**文案、位置、样式保持一致**（统一化）；不要合并成底部那一个按钮。「统一」指的是三平台之间一致，不是合并。

**正确做法**：每个来源在表单内放两个独立按钮，点击瞬间固化 `ImportChoice`，执行阶段只认固化值，不互相串味。

1. **共享类型**（放在 `ImportPage.tsx` 顶部）：
   ```ts
   type ImportChoice = {
     kind: "favorites" | "public_url";
     mediaId?: string;
     url?: string;
   };
   ```
2. **每个来源一个 stored-choice 状态**（B站/知乎/CSDN 各一个，互不复用）：
   ```ts
   const [biliImportInput, setBiliImportInput] = useState<ImportChoice | null>(null);
   const [zhihuImportInput, setZhihuImportInput] = useState<ImportChoice | null>(null);
   const [csdnImportInput, setCsdnImportInput] = useState<ImportChoice | null>(null);
   ```
3. **每个表单两个按钮**（文案统一为 `预览并配置标签（登录收藏夹）` / `预览并配置标签（收藏夹链接）`，都用 `primary-button wide`，置于各自小节末尾）：
   - `onPreviewFavorites` → `startXxxFavoritesPreview`：`kind: "favorites"`、`mediaId: selectedCollectionId`、`url: undefined`。
   - `onPreviewPublic` → `startXxxPublicPreview`：`kind: "public_url"`、`mediaId: undefined`、`url: xxxPublicUrl.trim()`。
   - 两个函数都先 `setXxxImportInput({...})` 再 `setPreview(next)`、`setStep("tags")`，并对空/未选输入做 `setError` 兜底。
4. **按钮可见性**：
   - **B站/知乎**：按钮始终渲染，未满足前置条件（未登录 / 未选收藏夹 / 未解析链接）时 `disabled`，而非隐藏。
   - **CSDN**：按钮**始终渲染**（即使还没输入用户名、还没拉到收藏夹），未选收藏夹 / 未解析链接时 `disabled`——这是修复「CSDN 缺失默认导入按钮」的关键：不要再让按钮依赖 `collections.length > 0` 才出现。
5. **B站图文收藏是第三条独立入口**（不走上面两个按钮）：`BilibiliForm` 内单独的「图文收藏 N 条」按钮 → `startBiliOpusPreview`，走哨兵 id `bili_opus_fav`。知乎/CSDN 没有这一条。
6. **底部统一按钮只留给单入口来源**：ImportPage 底部「预览并配置标签」按钮仅在 `mode === "github" | "browser"` 时显示（`bottomPreviewReady` 控制可点状态）；`mode` 用基础值（`"login"`=B站、`"zhihu"`、`"csdn"`、`"github"`、`"browser"`、`"file"`），点击来源卡片即选定。
7. **`buildImportInput` 优先用 stored choice**：对每个来源，先 `if (isXxx && xxxImportInput) { return { apiCall: () => api.executeXxxImport({kind, mediaId, url, itemTagAssignments: assignments}) }; }`；只在无 stored choice 时（GitHub、浏览器）才回退到推导逻辑。
8. **返回/完成时清空**：在 `TagEditor` 的 `onBack` 和导入完成 `handleResult` 的 `setTimeout` 里，把三个 `setXxxImportInput(null)`，防止上一次选择残留串到下一次导入。

**适用范围**：以后新增来源若同样有「登录 + 公开链接」双入口（例如网易云音乐收藏可能既有登录歌单也有公开歌单链接），务必沿用上述模式——表单内两个独立按钮 + 预览时固化 `ImportChoice`，并保持与 B站/知乎/CSDN 一致的按钮文案与样式。

**验证**：`tsc --noEmit` 通过、`vite build` 成功即可；真机 `cargo run` + `npm run dev`，分别确认 B站/知乎/CSDN 的「登录收藏夹」与「收藏夹链接」两个按钮都出现、置灰逻辑正确、点击后都走对 `kind`。

### 3.12 删除一律走软删除（回收站）

本应用没有"硬删除用户收藏"的概念——所有删除都先进入回收站，保留期内可恢复。**新增来源时不要新写硬删除逻辑**，复用现有机制即可。

- **软删除（进回收站）**：`delete_items`（批量）、`delete_items_by_tag`（按标签）在后端都是 `db::soft_delete_*`，仅置 `items.deleted_at` 时间戳 + 移除 FTS 行，保留 `item_tags` / `import_run_items` 关联与封面文件，恢复时零成本重建 FTS。
- **恢复**：`restore_item` / `restore_items` 把 `deleted_at` 置 `NULL` 并重建 FTS 行。
- **永久删除**：`purge_item` / `purge_items` / `empty_trash` 才真正删库行 + FTS 行，并通过 `remove_cover_files` 仅删 `covers/` 目录下的封面文件（带路径前缀校验，防误删）。
- **自动清理**：`auto_purge_trash(retention_days)` 删除超过保留期的回收站项；`App.tsx` 在应用启动时按当前保留期触发一次。
- **保留期**：`src/lib/retention.ts` 配置，`DEFAULT_RETENTION_DAYS = 7`，可选 7 / 15 / 30 天，设置页可改（已与用户确认默认 7 天）。
- **schema**：迁移 `0007_soft_delete.sql` 为 `items` 表加 `deleted_at INTEGER`（NULL = 在库，非 NULL = 在回收站）。
- **筛选**：`ItemFilters.trash: Option<bool>`（`None`/`false` = 仅正常项，`Some(true)` = 仅回收站），`search_items` 已支持。
- **前端**：侧边栏「回收站」入口带未读计数角标；`TrashPage` 提供单条/批量恢复、永久删除、清空，并显示保留期倒计时；`api.ts` 的 `listTrash` / `restoreItem` / `restoreItems` / `purgeItem` / `purgeItems` / `emptyTrash` / `getTrashCount` / `autoPurgeTrash` 均有 mock 兜底。

### 3.13 不要自动 commit / push（AI 协作约定）

**这是一条硬性协作纪律，优先级高于"实现→验证→提交"的默认节奏。**

- AI（WorkBuddy）完成代码 / 文档改动后，**不要自动执行 `git commit`**，也不要在"校验通过""验证全绿"之后自行提交。
- **正确流程**：改完 → 跑校验（`cargo check` / `tsc --noEmit` / `cargo test`）→ 把改动文件与 diff 摘要列给用户 review → **等用户明确说"提交"** 后再 commit。
- **push 同理**：必须用户明确同意才能 `git push`，且若用户中途叫停（例如"先不要 push"），立即停止，已 commit 但未 push 的内容保持本地。
- **例外**：仅当用户在本轮对话里已经明确授权（如"提交吧" / "push 吧"）时，才执行对应动作。
- **commit 聚焦**：提交前若 `cargo fmt` 误改了无关文件的纯格式差异，用 `git checkout -- <file>` 回退，保持 commit 只含本次相关改动。
- 这条约定是为了避免 AI 在未被确认的情况下就把半成品 / 阶段性改动固化进 git 历史；用户希望保留"先 review、后落盘"的掌控感。

---

## 四、新增来源检查清单

- [ ] 调研平台 API（收藏夹列表、收藏夹内容、认证方式）
- [ ] 创建 `source/{name}.rs`，实现 `SourceAdapter` trait
- [ ] 注册到 `source/mod.rs`、`state.rs`、`commands.rs`、`lib.rs`
- [ ] 添加 Cargo 依赖（如需要）
- [ ] 添加 Tauri 命令（登录、列表、预览、执行）
- [ ] 前端 `types.ts` 添加 `ImportMode`、`api.ts` 添加方法
- [ ] 前端 `ImportPage.tsx` 添加来源卡片 + 表单
- [ ] 若来源同时有「登录/用户名收藏夹」与「公开链接」两个入口，按 3.11 拆成两个独立「预览并配置标签」按钮（固化 `ImportChoice`，不要用单按钮 + 布尔推导）
- [ ] 前端 `LibraryPage.tsx` 添加来源筛选按钮
- [ ] `cargo check` + `cargo test` + `npm build` 全部通过
- [ ] 测试登录态持久化（重启应用后是否自动恢复）
- [ ] 测试标签分配（每个 item 只获得自己的标签）
- [ ] 测试去重（同一 URL 导入两次不会重复）
- [ ] 测试 source_url 链接正确性
- [ ] 若来源涉及删除/清理，复用 `db::soft_delete_*`（进回收站），不要引入硬删除逻辑（见 3.12）
- [ ] 新增 UI 颜色一律用 CSS 变量（`:root` 浅色 + `[data-theme="dark"]` 深色 + 侧边栏 `--side-*`），不要硬编码 hex（深浅色主题已支持）
- [ ] 跨源反馈用 `useToast()`，不要 `window.alert`
- [ ] **提交 + 打 tag（需用户明确同意后才做，见 3.13，禁止自动 commit/push）**

---

## 五、让 AI 阅读此文档

在 WorkBuddy 中开发新来源时，在对话开始时输入：

```
请先阅读 C:\Users\lioh\Documents\GitHub\bilibili_collector\DEVELOPMENT.md，
然后按照其中的步骤实现 {新平台名称} 收藏夹的导入功能。
```

WorkBuddy 会自动读取文档并按照其中的模板和检查清单进行开发。

---

## 六、已落地案例：CSDN 与 GitHub Stars

本文档的步骤已成功用于新增 CSDN 收藏夹与 GitHub Stars 两个来源，补充以下来源特有的经验（通用步骤见上文，此处只记差异与坑）。

### 6.1 CSDN 收藏夹
- **入口**：用户输入**英文用户名（handle）**，不是中文昵称。API 对未知/错误用户名返回空列表，前端必须给出明确提示（告诉用户去个人主页 URL 里找英文名）。
- **封面**：列表 API 不含封面。`enrich_items` 对每篇文章页面抓取 `og:image` 元信息，下载到本地 `cover_local_path`（复用 migration 0005 的字段），避免卡片封面空白。
- **无需登录**：公开收藏夹直接抓取。
- 注册位置：`src-tauri/src/source/csdn.rs` + 前端 `src/components/import/CsdnForm.tsx` + `LibraryPage` 筛选按钮。

### 6.2 GitHub Stars
- **入口**：个人访问令牌（PAT，建议 `public_repo`/`read:user` 范围）或仅用户名（只能取公开 stars）。
- **网络（关键坑）**：国内访问 `api.github.com` 常被墙。客户端使用 `native-tls`（**不是** `rustls-tls`），从而自动走系统/代理的 TLS 栈。若报错 `error sending request for url (https://api.github.com/...)`，基本是代理/TLS 问题——确认 `Cargo.toml` 中 `reqwest` 启用了 `native-tls`（feature `default-tls`），而非 `rustls-tls`。
- **封面**：用仓库 owner 的 `avatar_url`。
- 注册位置：`src-tauri/src/source/github.rs` + 前端 `src/components/import/GithubForm.tsx` + `LibraryPage` 筛选按钮。

### 6.3 与新增来源正交、但本仓库已采用的前端约定
新增来源后，以下优化会自动覆盖你的来源卡片，无需额外开发；但新增 UI 时请遵守：
- 收藏库长列表用 `VirtuosoGrid`（`react-virtuoso`）做虚拟滚动，数据多时不卡。
- 封面统一走 `CoverImage`（blur-up 懒加载 + shimmer 占位），卡片在 `VideoCard` 中接入即可。
- 跨源反馈统一用 `useToast()`（`src/components/Toast.tsx`），不要用 `window.alert`；导入执行阶段的错误原先被 `catch` 静默吞掉，现已改为 toast 提示。
- **深浅色主题**：所有颜色必须写成 CSS 变量（`:root` 浅色、`[data-theme="dark"]` 深色、侧边栏用 `--side-*`），新增 UI 禁止硬编码 hex，否则深色模式下会"开盲盒"。
- 微动效统一用 CSS 变量 + `transition`，并已纳入 `@media (prefers-reduced-motion: reduce)` 无障碍降级。