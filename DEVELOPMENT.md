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

### 3.14 数据库迁移的行尾漂移（会导致启动panic）

#### 现象

```
Failed to setup app: error encountered during setup hook:
migration N was previously applied but has been modified
```

`N` 是**第一个校验失败**的版本号（sqlx 从小到大校验，遇到不匹配即停）——**它通常不是"坏掉"的那个文件，只是排在队首**。

#### 原理

`sqlx::migrate!` 在**编译期**把 `.sql` 按**字节** embed 进 exe，运行时对数据库 `_sqlx_migrations.checksum`（sha384）逐一比对。CRLF 与 LF 在 SQL 语义上完全等价，但字节不同 → sha384 不同 → 判定"迁移被篡改" → panic。

三个来源的字节只要有一个不一致就炸：

| 来源 | 字节由谁决定 |
|---|---|
| 数据库记录的 checksum | **当初建库那台机器**上的文件 |
| CI 构建的 exe | CI runner checkout 出来的文件 |
| 本地 `cargo run` | 本地磁盘上的文件 |

因此**只发生在"老数据库 + 新 exe"的升级路径**；全新安装不会遇到（建库即用当前字节写入，天然一致）。

#### 三层防护（均已在位，勿拆）

1. `.gitattributes`：`*.sql text eol=lf`（规范化策略）。**注意它只对新的 checkout/add 生效，不会改写磁盘上已存在的老文件。**
2. 历史数据库一次性修复：把 `_sqlx_migrations` 的旧 checksum 重算为 LF 版（务必先备份 db + wal + shm）。
3. **运行时自愈**：`db::heal_migration_line_endings(&pool, &migrator)`，在 `migrator.run()` **之前**调用（`db::connect` 已接好）。

   判定逻辑：把当前 SQL 分别归一化成**全 LF / 全 CRLF** 各算一次 sha384，**只有命中其中之一**才认定是行尾漂移并 UPDATE checksum；两者都不匹配说明内容真被改了，**不做任何修改**，交由 sqlx 照常 panic。防篡改语义完整保留。

#### ⚠️ 操作陷阱：刷新磁盘行尾的正确姿势

加完 `.gitattributes` 后，要刷新磁盘上已有的老文件：

```bash
# ❌ 无效：git 的 clean filter 双向转换会判定「工作区 CRLF」与「索引 LF」内容相同，
#         checkout 直接跳过重写，行尾纹丝不动且 git status 依然干净
git add --renormalize src-tauri/migrations/
git checkout -- src-tauri/migrations/

# ✅ 有效：先删掉再检出，强制 git 走 smudge 按 eol=lf 重写
rm src-tauri/migrations/*.sql
git checkout -- src-tauri/migrations/
```

改完 `git status` 应仍然**干净**——因为仓库里存的本来就是 LF，磁盘只是残留了 CRLF。所以这类修复**通常不需要提交**。

#### 纪律

- **迁移文件一旦合并进主分支即视为不可变。** 要改 schema 就加新版本，绝不回头编辑已发布的文件。
- 改动 `heal_migration_line_endings` 或任何 migrate 相关代码后，必须跑：
  ```bash
  cargo test db::tests
  ```
  其中 5 个迁移自愈测试覆盖了：LF↔CRLF 双向漂移能自愈、真篡改**不得**自愈、全新库（无表）不报错、checksum 已一致时不产生写入。
- 排查脚本思路：Python 对每个 `.sql` 算 sha384，与 DB 记录对比，并额外算"转 LF 后"的值，用于区分**行尾差异**与**真篡改**。

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

---

## 七、批注 × Obsidian 联动（方案已定稿，**尚未实施**）

> 状态：仅完成方案设计与决策，代码未动。按 3.13，实施后不得自动 commit。

### 7.1 背景与现状

批注目前是 **app 内的孤岛**：

- 存储：`items.notes TEXT`（迁移 `0006_video_notes.sql`），单字段，无标题、无 Markdown 渲染
- 命令：只有 `update_item_notes`（`commands.rs:645` → `db.rs:495`）
- 前端：`VideoCard` 的 `card-note-button` → `LibraryPage.noteVideo` → `VideoNoteEditorModal`（编辑 / 预览两态，预览态只做 `LinkifiedText` URL 转链）
- `notes` 已进 FTS 索引，app 内可搜

联动目标：让批注沉淀进 Obsidian vault，同时**不牺牲不用 Obsidian 的用户**。

### 7.2 设计决策（已拍板）

| 项 | 决定 | 理由 |
|---|---|---|
| 联动深度 | **L1 跳转 + L2 单向导出**，不做 L3 回读 | 回读会引入双向冲突 / 改名 / 删除等分布式同步问题，收益小。收藏是"输入流"，vault 是"知识库"，单向流最自然 |
| 笔记粒度 | 一篇收藏一个 md | 最利于 Obsidian 检索、双链、Dataview 查询 |
| 创建时机 | **保存批注时自动创建 / 更新** | 用户原话「只对做批注的页面进行笔记的创建」，无需惦记手动导出 |
| 创建范围 | 仅写了批注的收藏，**不做全量** | 全量一万条会淹没 `Ctrl+O` 切换器与全局搜索，信噪比才是真代价；且 app 内已有 FTS，重复建设 |
| 兼容要求 | 联动为**默认关闭的开关**，未启用则纯本地批注、零副作用 | 必须兼容不使用 Obsidian 的用户 |
| 封面 | **不复制进 vault** | 封面平均 ~278 KB/张，一万条约 2.7 GB，是唯一真正的空间炸弹 |
| vault 同步 | 用户 vault 纯本地、无任何同步 | 故文件数量对性能无硬约束（若有同步则必须严格控制写入量） |

**性能实测结论**：md 本身极小（frontmatter + 短批注约 300~500 字节，一万条约 4 MB 文件大小 / 40 MB 磁盘占用）。Obsidian 扛得住文件数，真正的成本是**搜索噪音**与**首次索引**，不是磁盘。

### 7.3 数据契约：一篇笔记长这样

```markdown
---
collector_id: bilibili:BV1xx411c7mD
title: "视频标题"
url: https://www.bilibili.com/video/BV1xx411c7mD
source: bilibili
author: UP主名
tags: [前端, 性能优化]
favorited_at: 2026-09-03
---

<!-- collector:notes:start -->
这里是你写的批注，app 每次保存只替换这一块
<!-- collector:notes:end -->

（以下区域 app 永不触碰，用户在 Obsidian 里自由扩充）
```

三个关键点：

1. **`collector_id` 复用现有 `(source, external_id)` 复合键**——天然唯一、天然幂等。**绝不用路径或标题做匹配**：用户在 Obsidian 里改标题或移动文件夹，映射就断了。将来若要回读，也是扫描 vault 内所有含 `collector_id` 的 md 重建映射。
2. **`tags` 映射成 Obsidian 原生标签**，标签面板与 Dataview 可直接用。
3. **分区托管**：HTML 注释在 Obsidian 阅读视图下不显示（无视觉污染），但圈定了 app 的责任边界。**这是防"用户在 Obsidian 扩充的内容被覆盖"的唯一保险**；检测到标记被手动删除则说明用户不愿被托管，跳过同步并提示。

**存放位置（已定）**：笔记写入**用户已有的 vault** 根目录下的一个子目录（默认 `收藏/`，设置页可改名），app 只在该子目录内读写，**绝不触及 vault 中其他任何位置**——实现上要在 Rust 端做路径前缀校验（与 `remove_cover_files` 的前缀校验同一思路）。

- **不新建独立 vault**：独立 vault 会切断双链与统一搜索，收藏卡片无法与既有笔记互相链接，联动价值减半。
- 若用户确实想要独立库：把 vault 路径指向一个空文件夹，再在 Obsidian 里「打开文件夹作为仓库」即可，方案天然支持，只是无法与主库双链。
- 目录选择复用已有的 `tauri-plugin-dialog`；选中后温和校验该目录下是否存在 `.obsidian/`（不存在只提示、不阻止，避免误伤用其他工具管理 md 的场景）。
- 子目录名留空则直接写到 vault 根目录——**不推荐**，会污染根目录，UI 上应给出提示。

文件名：默认 `{标题}.md`；重名且已有文件的 `collector_id` 不是自己时，追加 `[{source}-{id前6}]` 消歧。

### 7.4 实施清单

| 文件 | 改动 |
|---|---|
| `src-tauri/migrations/0008_obsidian_sync.sql` | 新建：`ALTER TABLE items ADD COLUMN obsidian_path TEXT` |
| `src-tauri/src/obsidian.rs` | **新建**：`ObsidianSettings` 读写、文件名 sanitize、frontmatter 生成（serde_yaml）、分区托管替换、写文件、构造并打开 `obsidian://` |
| `src-tauri/src/commands.rs` | 改 `update_item_notes`（写库成功后触发同步并回写 `obsidian_path`）；新增 `get/set_obsidian_settings`、`open_note_in_obsidian`、`export_items_to_obsidian`、`pick_obsidian_vault` |
| `src-tauri/src/lib.rs` | 注册 `obsidian` 模块与新命令（复用既有 `dialog` + `webbrowser`，**未引入新插件**） |
| `src-tauri/Cargo.toml` | 仅加 `serde_yaml`（生成 frontmatter）；打开 `obsidian://` 复用既有 `webbrowser`，不引入 `tauri-plugin-opener` |
| `src/components/SettingsPage.tsx` | 新增 Obsidian 分区：开关 + vault 目录选择（**复用既有 `tauri-plugin-dialog`**）+ 子目录名 |
| `src/components/VideoNoteEditorModal.tsx` | 加「在 Obsidian 中打开」（有 `obsidian_path` 时可用）；保存成功时提示已同步 |
| `src/components/VideoCard.tsx` | hover 菜单加「导出到 Obsidian」（开关开启时显示） |
| `src/components/LibraryPage.tsx` | 加载联动开关状态并透传给 `VideoCard`；批量工具栏加「导出到 Obsidian」 |

**依赖现状（实施修正）**：打开 `obsidian://` 深链直接复用既有 `webbrowser` 依赖（即 `open_url` 命令用的那个），**未引入 `tauri-plugin-opener`**；目录选择器用既有 `tauri-plugin-dialog` 的 Rust 端 `blocking::FileDialog`（新增 `pick_obsidian_vault` 命令），**未新增前端 npm 依赖**。因此 `Cargo.toml` 只新增了 `serde_yaml`。理由：深链 scheme 用 `webbrowser::open` 在 Windows 上经 ShellExecute 分发即可可靠唤起 Obsidian，无需额外插件；保持依赖面最小、编译更快。

**为什么同步逻辑放 Rust 端**：Tauri 前端 fs 插件有 scope 限制，写不了 app 数据目录外的路径；Rust 端 `std::fs` 无此限制。

### 7.5 核心流程（保存批注）

```
用户点保存
  → ① 写库 items.notes                ← 必须先成功
  → ② 检查：开关开启？vault 已配置？notes 非空？
        ├─ 任一不满足 → 静默返回，零副作用
        └─ 满足 → ③
  → ③ 生成 / 更新 md（只替换托管区）
  → ④ 落库 items.obsidian_path
  → ⑤ toast「已同步到 Obsidian」
```

**顺序是关键**：先写库再写文件。同步失败只 toast，**绝不能影响批注已保存**——这是整个功能的健壮性底线。

### 7.6 降级与兼容（不用 Obsidian 的用户）

1. 联动开关**默认关闭**
2. 未开启 / 未配置 vault → 完全不触发文件系统操作，行为与现状完全一致
3. vault 路径失效（目录被删、无写权限）→ toast 提示一次，批注照常保存
4. 开关关闭时，同步相关 UI（打开按钮、导出入口）**不显示**，避免干扰

### 7.7 ⚠️ 必须避开的坑

| 坑 | 后果 | 对策 |
|---|---|---|
| 标题含 `:` `#` `[` `"` | **YAML 直接崩掉**，Obsidian 解析不出 frontmatter | 用 `serde_yaml` 序列化，**禁止字符串拼接** |
| Windows 非法字符 `\ / : * ? " < > \|` | 写入失败或静默丢文件 | 白名单 sanitize + 255 长度截断（中文按字符算） |
| 写文件带 BOM | Obsidian 里中文乱码 | 强制 UTF-8 无 BOM |
| 标签含空格或 `#` | Obsidian tags 非法 | 空格转 `-`，剔除非法字符 |
| 用户在 Obsidian 里扩充后被覆盖 | **丢数据，且悄无声息** | 分区托管（见 7.3） |
| 批注被清空 | 文件要不要删？ | **只清托管区，不删文件**（可能已有用户笔记） |
| 收藏进回收站 | 笔记怎么办？ | **不动 md**，沿用 3.12 软删除哲学——只做加法 |
| `obsidian_path` 存成绝对路径 | 换机后 vault 路径一变，映射全部失效 | **必须存相对 vault 根目录的路径**（见 7.11） |

### 7.8 分阶段

- **P0（约 1 天）**：设置项 + `obsidian.rs` + 保存批注自动创建 / 更新笔记 + `obsidian_path` 落库 + 分区托管。核心闭环可用。
- **P1（半天）**：`obsidian://` 打开跳转 + 手动导出单条 / 批量。
- **P2（可选）**：批注编辑器支持 Markdown 渲染。

> P2 在分区托管方案下是**可选**的：app 批注保持纯文本也完全可用，用户在 Obsidian 里扩充时自己用 Markdown 即可，不影响主流程。

### 7.9 性能影响评估

前提：**仅批注触发生成**，量级是几百篇（不是一万条），vault 纯本地无同步。

**Obsidian 侧**

| 场景 | 影响 |
|---|---|
| 磁盘占用 | 约 400 B/篇，几百篇合计 < 200 KB |
| 启动 / 搜索索引 | 几百个小文件的增量索引，毫秒级 |
| 保存批注触发写入 | 单文件增量重索引，几毫秒，**不会全库重扫** |
| 批量导出几百条 | 连续写入时 Obsidian 会集中处理，可能短暂占 CPU（数秒）→ 分批写入 + 完成后统一 toast |

**app 侧**

| 场景 | 影响 |
|---|---|
| 单次保存批注 | 多一次 <1 KB 的本地文件写入，约 1~5 ms，UI 无感 |
| 数据库 | 新增 `obsidian_path` 一列，每行几十字节，万条约几百 KB |
| 启动开销 | **零**——不做回读（L3），启动时不扫描 vault |

**真正会拖慢的两条**（故方案明确回避）：

1. **全量同步一万条** → Obsidian 搜索噪音 + 首次索引变慢（见 7.2）
2. **做 L3 回读** → 每次启动都要遍历 vault 全文比对 mtime，那才是真正的性能负担

换言之，放弃回读不仅省掉了冲突处理，也顺带换来了**零启动开销**。

### 7.10 删除语义：笔记如何处置

沿用 3.12「只做加法」的软删除哲学——**app 在任何情况下都不删除 vault 里的文件**。

| 操作 | 笔记处置 | 理由 |
|---|---|---|
| 软删除（进回收站） | **不动** | 可能只是误删，保留期内会恢复；笔记里也可能已有用户内容 |
| 从回收站恢复 | 不动，映射原样生效 | `obsidian_path` 一直在，恢复后继续同步 |
| 永久删除 / 清空回收站 / 超期清理 | **默认不动**；可选移到 `收藏/已归档/`（**移动而非删除**） | 笔记可能已成长为用户自己的内容，删掉是灾难 |
| 批注被清空 | 只清托管区，**不删文件** | 同上 |

永久删除时的分级提示（不静默）：

- 检测到托管区之外还有用户内容 → **一定不动**，toast「该笔记含你自己的内容，已保留」
- 若纯粹由 app 生成（只有 frontmatter + 托管区）→ 提示「这篇笔记可以安全删除」，但**让用户在 Obsidian 里自己删**，app 不动手

**重新导入能自动接回原笔记**：锚点是 `collector_id`（`source:external_id`），**不是 `item.id`**。即使数据库行被永久删除、之后重新导入生成了新的 `item.id`，只要 `collector_id` 不变，写入时就能找到已有 md 并复用（只更新托管区），不会新建重复文件。查找顺序：

1. `obsidian_path` 非空且文件存在 → 直接用
2. 否则按预期文件名 `{标题}.md` 找，并校验 frontmatter 的 `collector_id` 是否匹配 → 匹配则复用
3. 都没有 → 新建

> 边界：若源站改了标题，第 2 步会落空，会新建一篇、旧的成为孤儿。可加「重新关联」扫描功能（P2，扫描目录按 `collector_id` 匹配）兜底。

### 7.11 换机迁移

迁移的是**两样彼此独立的东西**，互不影响：

**① 收藏数据（app 侧）** —— 现有 JSON 导出 / 导入链路可用，但**需要补一处**

- ✅ **批注本身能迁移**：`export_items` 的 SELECT 已含 `notes`（`db.rs:1098`），`ExportItem` 已带 `notes`（`db.rs:1155`），`import_collection` 会写回（`db.rs:1326`）
- ❌ **`obsidian_path` 不在导出链路里**：`export_items` 的 SELECT 与 `import_collection` 的 INSERT 都还没有这一列 → **实施时必须两处都补上**，否则新电脑上 app 不知道笔记在哪，映射全丢
- 导入是增量模式（`db.rs:1290`：已存在则跳过，绝不覆盖原库），空库导入不受影响
- 封面无需手动拷贝：`import_collection` 会调 `cache_imported_covers` 重新下载

**② 笔记（vault 侧）** —— 与 app 无关

- 就是一堆 md 文件，自己拷到新电脑（U 盘 / 移动硬盘 / 网盘），Obsidian 打开即可
- app 不参与，也不需要参与

**③ 恢复联动**

- **`obsidian_path` 必须存「相对 vault 根目录」的路径**（如 `收藏/视频标题.md`），**绝不能存绝对路径**——新电脑的 vault 路径大概率不同，存绝对路径会导致全部失效
- 新电脑上只需在设置里重新指定 vault 根目录，所有映射自动生效

**迁移清单**：

1. 旧电脑：导出 JSON（实施后含 `obsidian_path`）
2. 拷贝 vault 文件夹到新电脑
3. 新电脑：装 app → 导入 JSON → 设置里指定 vault 根目录
4. 封面自动重下，映射自动恢复

### 7.12 待定项（已拍板，2026-09-03 开工）

1. **配置存放位置**：✅ 采用 Rust 端 `obsidian_settings.json`（存于 app data 目录）。理由：同步是后端行为，配置就近；与 `retention.ts` 走前端 localStorage 的不一致可接受，因为两者的触发机制不同（retention 仅影响启动清理，本功能涉及文件系统写入）。
2. **分区托管**：✅ 保留。HTML 注释标记圈出 app 托管区，同步时只替换该区，用户在 Obsidian 里写的其余内容永不丢失；若标记被手动删除则跳过同步并提示。
3. **永久删除「移到归档目录」**：❌ 暂不做。默认永久删除时完全不动 vault 文件（Obsidian 是独立知识库）。如后续需要，再加一个开关把笔记移到 `收藏/已归档/`。

### 7.13 实施状态（2026-09-04 更新）

- **功能已完整落地并提交**（commit `32213c4` + `2257c0d`）：迁移 `0008`、`obsidian.rs`、`update_item_notes` 自动同步、5 个命令（`get/set_obsidian_settings`、`get_item_obsidian_path`、`open_note_in_obsidian`、`export_items_to_obsidian`、`pick_obsidian_vault`）、设置页联动卡、批注弹窗整合「导出到 Obsidian + 在 Obsidian 中打开」、批量工具栏导出、移除卡片重复导出按钮。
- 打开深链：`webbrowser` 在 Windows **只认默认浏览器**（硬编码查 `http` 关联并把任何 scheme 丢给浏览器）→ 改走 `ShellExecuteW`（`obsidian.rs::open_uri_system`，需 `windows-sys 0.59`），由系统按协议关联唤起 Obsidian.exe。
- 真机验证后修复的坑：
  - `ItemRow` 加 `obsidian_path` 后 `search_items` / `list_trash` 漏列 → `query_as` 运行时 `ColumnNotFound` 致收藏库空白；已统一为 `ITEM_ROW_COLUMNS` 常量 + 回归测试。
  - Obsidian 命令最初写成**同步命令**（主线程），`blocking_pick_folder()` 卡死 UI → 全部改 async，选目录放 `spawn_blocking`。
  - `ensure_within_vault` 用 `canonicalize()` 前缀比较，Windows 会给 vault 加 `\\?\` 前缀而目标文件未创建时无前缀 → 永远误拒；改纯词法规范化比较 + 回归测试。
  - 前端 `obsidianEnabled` 只挂载时加载一次 → 设置页开启后导出入口不出现；改为依赖 `refreshToken` + `onObsidianChanged` 回调刷新。
  - 批注弹窗「在 Obsidian 中打开」按 item 快照判灰 → 弹窗打开时用 `get_item_obsidian_path` 查库确认，导出成功后再点亮。
- P2（批注编辑器支持 Markdown 渲染）留作后续，非主流程阻塞项。