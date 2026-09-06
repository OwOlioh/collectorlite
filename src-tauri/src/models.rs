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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrStatus {
    pub code: i64,
    pub message: String,
    pub profile: Option<BilibiliProfile>,
}
