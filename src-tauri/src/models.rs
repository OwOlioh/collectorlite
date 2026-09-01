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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrStatus {
    pub code: i64,
    pub message: String,
    pub profile: Option<BilibiliProfile>,
}
