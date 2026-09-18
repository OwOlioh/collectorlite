use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: i64,
    pub namespace: String,
    pub name: String,
    pub normalized: String,
    pub color: Option<String>,
    pub description: Option<String>,
    pub count: i64,
    pub category_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagInput {
    pub id: Option<i64>,
    pub namespace: String,
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
    pub category_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCategory {
    pub id: i64,
    pub name: String,
    pub normalized: String,
    pub color: Option<String>,
    pub position: i64,
    /// 所属分类组：组内最靠前成员（leader）的 id；NULL = 未分组。
    #[serde(default)]
    pub group_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInfo {
    pub source: String,
    pub id: String,
    pub title: String,
    pub owner: Option<String>,
    pub count: i64,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalItem {
    pub source: String,
    pub external_id: String,
    pub source_url: String,
    pub title: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub cover_local_path: Option<String>,
    pub author_name: Option<String>,
    pub author_id: Option<String>,
    pub partition_name: Option<String>,
    pub published_at: Option<i64>,
    pub duration: Option<i64>,
    pub favorite_time: Option<i64>,
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoItem {
    pub id: i64,
    pub source: String,
    pub external_id: String,
    pub source_url: String,
    pub title: String,
    pub description: String,
    pub notes: String,
    pub cover_url: Option<String>,
    pub cover_local_path: Option<String>,
    pub author_name: Option<String>,
    pub author_id: Option<String>,
    pub partition_name: Option<String>,
    pub published_at: Option<i64>,
    pub duration: Option<i64>,
    pub favorite_time: Option<i64>,
    pub deleted_at: Option<i64>,
    /// 星标置顶：true = 内容置顶显示（starred_at 记录打星时间）。
    #[serde(default)]
    pub starred: bool,
    /// 打星时间（unix 秒）；未打星为 None。
    #[serde(default)]
    pub starred_at: Option<i64>,
    /// 同步到 Obsidian 后，vault 内相对路径（如 `收藏/标题.md`）；未同步为 None。
    pub obsidian_path: Option<String>,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionSuggestion {
    pub name: String,
    pub count: i64,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub collection: CollectionInfo,
    pub items: Vec<VideoItem>,
    pub partition_suggestions: Vec<PartitionSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    Favorites,
    PublicUrl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemTagAssignment {
    pub external_id: String,
    pub tag_specs: Vec<TagInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub kind: ImportKind,
    pub media_id: Option<String>,
    pub url: Option<String>,
    /// 前端已解析好的收藏夹信息（下拉选中项 / 公开链接首次解析结果）。
    /// 提供时 preview/execute 直接复用，跳过服务端的重复 resolve_collection 网络调用；
    /// 未提供时回退到 resolve_collection（深链接等场景）。
    #[serde(default)]
    pub collection: Option<CollectionInfo>,
    #[serde(default)]
    pub tag_specs: Vec<TagInput>,
    #[serde(default)]
    pub item_tag_assignments: Vec<ItemTagAssignment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub run_id: i64,
    pub total: i64,
    pub imported: i64,
    pub skipped: i64,
    pub failed: i64,
    pub cleanup_status: Option<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

/// 重新缓存封面的结果统计。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecacheResult {
    pub total: i64,
    pub cached: i64,
    pub failed: i64,
    #[serde(default)]
    pub errors: Vec<String>,
}

/// 封面缓存队列的当前状态：还有多少张没缓存、后台任务是否在跑。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverCacheStatus {
    pub pending: i64,
    pub running: bool,
}

/// 网易云增量同步结果。
///
/// `skippedReason` 为 `Some` 表示这一轮**根本没跑**（未登录 / 已关闭 / 太频繁 /
/// 还没登记过歌单），前端据此决定要不要提示用户；`None` 表示确实跑了，
/// `added` / `removed` 才是有效数字。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseSyncReport {
    /// 本次新入库的曲目数（已存在的算 skipped，不重复计数）
    pub added: i64,
    /// 因用户取消收藏而移入回收站的曲目数
    pub removed: i64,
    /// 本轮参与同步的歌单数
    pub playlists: i64,
    pub synced_at: Option<i64>,
    #[serde(default)]
    pub skipped_reason: Option<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

// ── 速记浮窗（P1） ──────────────────────────────────────────────────────────

/// 曲目反查结果（速记面板打开时用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackResolveResult {
    /// 是否拿到真实 song id。`false` 时 `songId` / `coverUrl` / `duration` 都可能为空，
    /// 但 `title` / `artist` 仍可信（来自窗口标题），面板照样能记批注。
    pub resolved: bool,
    pub song_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    pub duration: Option<i64>,
    /// 库里是否已存在（决定面板显示「更新」还是「新建」）
    pub in_library: bool,
    pub item_id: Option<i64>,
}

/// 速记面板的一键入库请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickCaptureRequest {
    pub title: String,
    pub artist: String,
    /// 反查到的真实 song id；`None` 表示反查失败 → 用合成 id 落库，**批注照样记**
    pub song_id: Option<String>,
    pub cover_url: Option<String>,
    pub duration: Option<i64>,
    /// 批注正文，可以为空
    pub note: String,
    /// 只传标签名，服务端按「空 namespace + 名称」归位
    pub tags: Vec<String>,
}

/// 速记面板的一键入库结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickCaptureResult {
    pub item_id: i64,
    /// `true` = 新建条目；`false` = 库里已有，这次只是补了批注 / 标签
    pub created: bool,
    /// `true` = 没反查到真实 id，用的是合成 external_id。
    /// 将来歌单导入同一首歌时会产生**两条**，前端如实提示、不擅自合并。
    pub unresolved: bool,
}

/// 导出文件中单个标签的精简表示（不含库内 id，靠 name+namespace 重新关联）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTag {
    pub namespace: String,
    pub name: String,
    pub color: Option<String>,
    pub category: Option<String>,
}

/// 导出文件中单条收藏的完整元数据（保留 extra_json 以不丢浏览器书签的 folder_tags）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportItem {
    pub source: String,
    pub external_id: String,
    pub source_url: String,
    pub title: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub author_name: Option<String>,
    pub author_id: Option<String>,
    pub partition_name: Option<String>,
    pub published_at: Option<i64>,
    pub duration: Option<i64>,
    pub favorite_time: Option<i64>,
    pub notes: String,
    /// vault 内相对路径，换机迁移时据此恢复联动；未同步为 None。
    pub obsidian_path: Option<String>,
    /// 星标状态随导出文件保存，换机/恢复时一并还原。老版本导出文件无此字段 → 默认未星标。
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub starred_at: Option<i64>,
    pub extra: Value,
    pub tags: Vec<ExportTag>,
}

/// 收藏库导出文件根结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionExport {
    pub format_version: u32,
    pub exported_at: i64,
    pub app: String,
    pub items: Vec<ExportItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemFilters {
    pub query: Option<String>,
    pub tag_ids: Vec<i64>,
    pub tag_mode: String,
    /// 严格匹配：item 的标签集合必须「恰好等于」输入的 tag_ids（既包含所有输入标签、又不含任何输入之外的标签）。
    /// 与 tag_mode 互斥——开启后忽略 and/or，仅按精确集合筛选。默认关闭。
    #[serde(default)]
    pub strict: bool,
    /// 无标签筛选：item 未挂任何标签（item_tags 无关联行）。与 tag_ids 互斥——开启时忽略 tag_ids。默认关闭。
    #[serde(default)]
    pub untagged: bool,
    pub sort: String,
    #[serde(default)]
    pub sources: Vec<String>,
    /// 回收站过滤：None/false = 仅正常在库；Some(true) = 仅回收站。
    #[serde(default)]
    pub trash: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliProfile {
    pub is_login: bool,
    pub mid: Option<i64>,
    pub name: Option<String>,
    pub face: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrSession {
    pub qrcode_key: String,
    pub qrcode_url: String,
}

/// 浏览器扩展「快速入库」本地桥的运行时状态，供设置页展示与排障。
/// `port` 为 0 表示桥未启动（端口全被占用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeInfo {
    pub port: u16,
    pub running: bool,
    pub token: String,
}

/// 收藏库统计聚合（前端「统计」页用）。字段命名与前端 `CollectionStats` 对应。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CollectionStats {
    pub total: i64,
    pub starred_count: i64,
    pub untagged_count: i64,
    pub by_source: Vec<SourceCount>,
    pub by_tag: Vec<TagCountStat>,
    pub by_month: Vec<MonthCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SourceCount {
    pub source: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TagCountStat {
    pub name: String,
    pub color: Option<String>,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MonthCount {
    pub month: String,
    pub count: i64,
}

/// 重复项视图里的单条预览（不暴露全部字段）。字段命名对应前端 `DuplicateItemPreview`。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateItemPreview {
    pub id: i64,
    pub source: String,
    pub external_id: String,
    pub source_url: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub favorite_time: Option<i64>,
}

/// 一组跨源重复项：同一归一化 `source_url`（match_type="url"）或标题高度相似（match_type="fuzzy"）的多条 item。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    /// 归一化后的 URL key（url 组）或 `fuzzy:{id_a}-{id_b}`（fuzzy 组），用于分组与调试。
    pub key: String,
    /// 匹配方式：`url` = 归一化链接相同；`fuzzy` = 标题相似度达到阈值（疑似，需用户手动确认）。
    pub match_type: String,
    pub items: Vec<DuplicateItemPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrStatus {
    pub code: i64,
    pub message: String,
    pub profile: Option<BilibiliProfile>,
}
