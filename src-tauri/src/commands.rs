use std::collections::HashMap;

use tauri::State;

use crate::db;
use crate::error::AppError;
use crate::models::{
    BilibiliProfile, CollectionInfo, ImportPreview, ImportRequest, ImportResult, ItemFilters,
    PartitionSuggestion, QrSession, QrStatus, Tag, TagCategory, TagInput, VideoItem,
};
use crate::source::SourceAdapter;
use crate::state::AppState;

fn to_video_item(item: &crate::models::ExternalItem, local_id: i64) -> VideoItem {
    VideoItem {
        id: local_id,
        source: item.source.clone(),
        external_id: item.external_id.clone(),
        source_url: item.source_url.clone(),
        title: item.title.clone(),
        description: item.description.clone(),
        notes: String::new(),
        cover_url: item.cover_url.clone(),
        cover_local_path: item.cover_local_path.clone(),
        author_name: item.author_name.clone(),
        author_id: item.author_id.clone(),
        partition_name: item.partition_name.clone(),
        published_at: item.published_at,
        duration: item.duration,
        favorite_time: item.favorite_time,
        tags: vec![],
    }
}

fn cover_cache_path(
    state: &AppState,
    source: &str,
    external_id: &str,
    extension: &str,
) -> Result<std::path::PathBuf, AppError> {
    let covers_dir = state.data_dir.join("covers");
    std::fs::create_dir_all(&covers_dir)?;
    let hash = md5::compute(format!("{source}:{external_id}"));
    Ok(covers_dir.join(format!("{hash:x}.{extension}")))
}

fn save_cover_file(
    state: &AppState,
    source: &str,
    external_id: &str,
    bytes: &[u8],
    extension: &str,
) -> Result<String, AppError> {
    let path = cover_cache_path(state, source, external_id, extension)?;
    if !path.exists() {
        std::fs::write(&path, bytes)?;
    }
    Ok(path.to_string_lossy().into_owned())
}

async fn cache_item_covers(
    state: &AppState,
    items: &[crate::models::ExternalItem],
) -> Vec<crate::models::ExternalItem> {
    let mut cached = Vec::with_capacity(items.len());
    for item in items {
        let mut next = item.clone();
        if let Some(url) = item.cover_url.as_deref().filter(|value| !value.is_empty()) {
            if let Ok((bytes, extension)) = state.bili.download_cover(url).await {
                if let Ok(path) =
                    save_cover_file(state, &item.source, &item.external_id, &bytes, &extension)
                {
                    next.cover_local_path = Some(path);
                }
            }
        }
        cached.push(next);
    }
    cached
}

async fn resolve_collection(
    state: &AppState,
    input: &ImportRequest,
) -> Result<CollectionInfo, AppError> {
    match input.kind {
        crate::models::ImportKind::Favorites => {
            let collections = state.bili.list_collections().await?;
            let media_id = input
                .media_id
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请选择要导入的收藏夹".into()))?;
            collections
                .into_iter()
                .find(|item| item.id == media_id)
                .ok_or_else(|| AppError::NotFound("没有找到指定的收藏夹".into()))
        }
        crate::models::ImportKind::PublicUrl => {
            let url = input
                .url
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请提供公开收藏夹链接".into()))?;
            state.bili.resolve_collection(url).await
        }
    }
}

#[tauri::command]
pub async fn bilibili_start_qr_login(state: State<'_, AppState>) -> Result<QrSession, String> {
    state
        .bili
        .start_qr_login()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn bilibili_poll_qr_login(
    state: State<'_, AppState>,
    qrcode_key: String,
) -> Result<QrStatus, String> {
    let status = state
        .bili
        .poll_qr_login(&qrcode_key)
        .await
        .map_err(|error| error.to_string())?;
    if status.code == 0 {
        let cookie = state.bili.cookie_value();
        state
            .save_bili_cookie(cookie)
            .map_err(|error| error.to_string())?;
    }
    Ok(status)
}

#[tauri::command]
pub async fn bilibili_profile(state: State<'_, AppState>) -> Result<BilibiliProfile, String> {
    state
        .bili
        .profile()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<(), String> {
    state.bili.set_cookie(None);
    state
        .save_bili_cookie(None)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn list_bilibili_favorites(
    state: State<'_, AppState>,
) -> Result<Vec<CollectionInfo>, String> {
    state
        .bili
        .list_collections()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn parse_public_favorite_url(
    state: State<'_, AppState>,
    url: String,
) -> Result<CollectionInfo, String> {
    state
        .bili
        .resolve_collection(&url)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn preview_import(
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportPreview, String> {
    let collection = resolve_collection(&state, &input)
        .await
        .map_err(|error| error.to_string())?;
    let items = state
        .bili
        .fetch_collection(&collection)
        .await
        .map_err(|error| error.to_string())?;
    let enriched = state
        .bili
        .enrich_items(&items)
        .await
        .map_err(|error| error.to_string())?;
    let preview_items = enriched
        .iter()
        .enumerate()
        .map(|(index, item)| to_video_item(item, -(index as i64 + 1)))
        .collect::<Vec<_>>();
    let mut partition_counts = HashMap::<String, i64>::new();
    for item in &enriched {
        if let Some(partition) = &item.partition_name {
            *partition_counts.entry(partition.clone()).or_default() += 1;
        }
    }
    let mut suggestions = partition_counts
        .into_iter()
        .map(|(name, count)| PartitionSuggestion {
            name,
            count,
            selected: true,
        })
        .collect::<Vec<_>>();
    suggestions.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    Ok(ImportPreview {
        collection,
        items: preview_items,
        partition_suggestions: suggestions,
    })
}

#[tauri::command]
pub async fn execute_import(
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportResult, String> {
    let collection = resolve_collection(&state, &input)
        .await
        .map_err(|error| error.to_string())?;
    let items = state
        .bili
        .fetch_collection(&collection)
        .await
        .map_err(|error| error.to_string())?;
    let enriched = state
        .bili
        .enrich_items(&items)
        .await
        .map_err(|error| error.to_string())?;
    let enriched = cache_item_covers(&state, &enriched).await;
    let assignments = input
        .item_tag_assignments
        .iter()
        .map(|assignment| (assignment.external_id.as_str(), &assignment.tag_specs))
        .collect::<HashMap<_, _>>();
    let total = enriched.len() as i64;
    let run_id = db::create_import_run(&state.pool, &collection, total, false)
        .await
        .map_err(|error| error.to_string())?;

    let mut imported = 0i64;
    let mut skipped = 0i64;
    let mut failed = 0i64;
    let mut errors = Vec::new();
    for item in &enriched {
        let result = async {
            let (item_id, inserted) = db::upsert_item(&state.pool, item).await?;
            let tag_specs = assignments
                .get(item.external_id.as_str())
                .copied()
                .unwrap_or(&input.tag_specs);
            for tag_spec in tag_specs {
                let tag_id = db::get_or_create_tag(&state.pool, tag_spec).await?;
                db::attach_tag(&state.pool, item_id, tag_id).await?;
            }
            db::rebuild_item_fts(&state.pool, item_id).await?;
            db::link_import_item(&state.pool, run_id, item_id).await?;
            Ok::<bool, AppError>(inserted)
        }
        .await;
        match result {
            Ok(true) => imported += 1,
            Ok(false) => skipped += 1,
            Err(error) => {
                failed += 1;
                if errors.len() < 20 {
                    errors.push(error.to_string());
                }
            }
        }
    }

    db::finish_import_run(&state.pool, run_id, imported, skipped, failed, &errors)
        .await
        .map_err(|error| error.to_string())?;
    db::build_import_result(&state.pool, run_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn search_items(
    state: State<'_, AppState>,
    filters: ItemFilters,
) -> Result<Vec<VideoItem>, String> {
    db::search_items(&state.pool, &filters)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_item(state: State<'_, AppState>, item_id: i64) -> Result<(), String> {
    let cover_path = db::delete_item(&state.pool, item_id)
        .await
        .map_err(|error| error.to_string())?;
    remove_cover_files(&state, cover_path.into_iter().collect::<Vec<_>>());
    Ok(())
}

#[tauri::command]
pub async fn delete_items(state: State<'_, AppState>, item_ids: Vec<i64>) -> Result<usize, String> {
    let count = item_ids.len();
    let cover_paths = db::delete_items(&state.pool, &item_ids)
        .await
        .map_err(|error| error.to_string())?;
    remove_cover_files(&state, cover_paths);
    Ok(count)
}

#[tauri::command]
pub async fn delete_items_by_tag(state: State<'_, AppState>, tag_id: i64) -> Result<usize, String> {
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM item_tags WHERE tag_id = ?")
        .bind(tag_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|error| error.to_string())?;
    let cover_paths = db::delete_items_by_tag(&state.pool, tag_id)
        .await
        .map_err(|error| error.to_string())?;
    remove_cover_files(&state, cover_paths);
    Ok(count as usize)
}

fn remove_cover_files(state: &AppState, cover_paths: Vec<String>) {
    let covers_dir = state.data_dir.join("covers");
    for cover_path in cover_paths {
        let path = std::path::Path::new(&cover_path);
        if path.starts_with(&covers_dir) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[tauri::command]
pub async fn list_tags(state: State<'_, AppState>) -> Result<Vec<Tag>, String> {
    db::list_tags(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn upsert_tag(state: State<'_, AppState>, tag: TagInput) -> Result<Tag, String> {
    db::upsert_tag(&state.pool, &tag)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn merge_tags(
    state: State<'_, AppState>,
    source_tag_id: i64,
    target_tag_id: i64,
) -> Result<(), String> {
    db::merge_tags(&state.pool, source_tag_id, target_tag_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_tag(state: State<'_, AppState>, tag_id: i64) -> Result<(), String> {
    db::delete_tag(&state.pool, tag_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_tag_categories(state: State<'_, AppState>) -> Result<Vec<TagCategory>, String> {
    db::list_tag_categories(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn create_tag_category(
    state: State<'_, AppState>,
    name: String,
    color: Option<String>,
) -> Result<TagCategory, String> {
    db::create_tag_category(&state.pool, &name, color)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn rename_tag_category(
    state: State<'_, AppState>,
    category_id: i64,
    name: String,
    color: Option<String>,
) -> Result<TagCategory, String> {
    db::rename_tag_category(&state.pool, category_id, &name, color)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_tag_category(
    state: State<'_, AppState>,
    category_id: i64,
) -> Result<(), String> {
    db::delete_tag_category(&state.pool, category_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn assign_tag_category(
    state: State<'_, AppState>,
    tag_id: i64,
    category_id: Option<i64>,
) -> Result<Tag, String> {
    db::assign_tag_category(&state.pool, tag_id, category_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn update_item_tags(
    state: State<'_, AppState>,
    item_id: i64,
    tag_specs: Vec<TagInput>,
) -> Result<VideoItem, String> {
    db::replace_item_tags(&state.pool, item_id, &tag_specs)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn update_item_notes(
    state: State<'_, AppState>,
    item_id: i64,
    notes: String,
) -> Result<VideoItem, String> {
    db::update_item_notes(&state.pool, item_id, &notes)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    webbrowser::open(&url).map_err(|error| error.to_string())
}
