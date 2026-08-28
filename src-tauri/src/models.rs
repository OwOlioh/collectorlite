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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemFilters {
    pub query: Option<String>,
    pub tag_ids: Vec<i64>,
    pub tag_mode: String,
    pub sort: String,
    #[serde(default)]
    pub sources: Vec<String>,
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
