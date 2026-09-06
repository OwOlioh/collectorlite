use std::collections::HashMap;
use std::sync::atomic::Ordering;

use tauri::State;

use crate::capture;
use crate::db;
use crate::error::AppError;
use crate::models::{
    BilibiliProfile, BridgeInfo, CollectionInfo, ImportPreview, ImportRequest, ImportResult,
    ItemFilters, ItemTagAssignment, PartitionSuggestion, QrSession, QrStatus, RecacheResult, Tag,
    TagCategory, TagInput, VideoItem,
};
use crate::source::browser::BrowserBookmarkClient;
use crate::source::SourceAdapter;
use crate::state::AppState;
use crate::obsidian;

/// 封面下载并发度：导入时仍把远程封面存到本地 `covers/`（保留离线查看能力），
/// 但把原来逐条串行改为有界并发，缩短满收藏夹的导入等待时间。
const COVER_CONCURRENCY: usize = 8;

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
        deleted_at: None,
        obsidian_path: None,
        starred: false,
        starred_at: None,
        tags: vec![],
    }
}

/// 预览缓存的 key：来源 + 收藏夹 id，用于 execute 阶段判断是否可复用预览算好的 enriched items。
fn preview_cache_key(collection: &CollectionInfo) -> String {
    format!("{}:{}", collection.source, collection.id)
}

/// 若缓存命中当前收藏夹，取出并返回 enriched items（同时销毁缓存，避免误用过期/切换后的数据）。
/// 命中即可跳过 fetch_collection + enrich_items 二次开销（满收藏夹约省 5–7 分钟）。
fn take_cached_enriched(
    state: &AppState,
    collection: &CollectionInfo,
) -> Option<Vec<crate::models::ExternalItem>> {
    let mut guard = state.import_cache.lock().ok()?;
    let taken = guard.take()?;
    if taken.key == preview_cache_key(collection) {
        Some(taken.items)
    } else {
        *guard = Some(taken);
        None
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

pub(crate) fn save_cover_file(
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
    // 保留「导入时下载到本地 covers/ 以便离线查看」的行为，但把串行逐条改为有界并发。
    let mut out = items.to_vec();
    for chunk_start in (0..items.len()).step_by(COVER_CONCURRENCY) {
        let end = (chunk_start + COVER_CONCURRENCY).min(items.len());
        let futures: Vec<_> = (chunk_start..end)
            .map(|i| {
                let item = &items[i];
                async move {
                    let mut next = item.clone();
                    if let Some(url) = item.cover_url.as_deref().filter(|value| !value.is_empty()) {
                        if let Ok((bytes, extension)) = state.bili.download_cover(url).await {
                            if let Ok(path) = save_cover_file(
                                state,
                                &item.source,
                                &item.external_id,
                                &bytes,
                                &extension,
                            ) {
                                next.cover_local_path = Some(path);
                            }
                        }
                    }
                    (i, next)
                }
            })
            .collect();
        let results = futures::future::join_all(futures).await;
        for (i, next) in results {
            out[i] = next;
        }
    }
    out
}

async fn cache_csdn_covers(
    state: &AppState,
    items: &[crate::models::ExternalItem],
) -> Vec<crate::models::ExternalItem> {
    let mut cached = Vec::with_capacity(items.len());
    for item in items {
        let mut next = item.clone();
        if let Some(url) = item.cover_url.as_deref().filter(|value| !value.is_empty()) {
            if let Ok((bytes, extension)) = state.csdn.download_cover(url).await {
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

/// 文件导入后补封面缓存：复刻实时 B站收藏夹导入的 `cache_item_covers` 行为。
/// 仅对 bilibili / csdn 这类「远程封面为 http（或需带 Referer）」的来源下载到本地，
/// 其余来源（知乎 / GitHub / 浏览器）封面为 https 远程链接，无需本地缓存。
async fn cache_imported_covers(state: &AppState, items: &mut [crate::models::ExternalItem]) {
    for item in items.iter_mut() {
        let url = match item.cover_url.as_deref().filter(|value| !value.is_empty()) {
            Some(value) => value,
            None => continue,
        };
        let download = match item.source.as_str() {
            "bilibili" => state.bili.download_cover(url).await,
            "csdn" => state.csdn.download_cover(url).await,
            _ => continue,
        };
        if let Ok((bytes, extension)) = download {
            if let Ok(path) =
                save_cover_file(state, &item.source, &item.external_id, &bytes, &extension)
            {
                item.cover_local_path = Some(path);
            }
        }
    }
}

async fn resolve_collection(
    state: &AppState,
    input: &ImportRequest,
) -> Result<CollectionInfo, AppError> {
    match input.kind {
        crate::models::ImportKind::Favorites => {
            let media_id = input
                .media_id
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请选择要导入的收藏夹".into()))?;
            // 图文收藏不是收藏夹、没有 media_id，用哨兵 id 走独立的动态流接口。
            if media_id == crate::source::bilibili::OPUS_FAV_COLLECTION_ID {
                return state.bili.opus_favorite_info().await;
            }
            let collections = state.bili.list_collections().await?;
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

/// 优先复用前端已解析的 `collection`（识别结果），省去服务端重复的 resolve 网络调用；
/// 未提供时回退到 `resolve_collection`（深链接等场景）。
async fn resolve_collection_or_use(
    state: &AppState,
    input: &ImportRequest,
) -> Result<CollectionInfo, AppError> {
    if let Some(c) = &input.collection {
        return Ok(c.clone());
    }
    resolve_collection(state, input).await
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

/// 图文收藏的元信息（标题 + 条数）。图文收藏不是收藏夹，走独立动态流接口，
/// 因此单独成入口、不混入 `list_bilibili_favorites` 的视频收藏夹下拉。
#[tauri::command]
pub async fn list_bilibili_opus_favorite(
    state: State<'_, AppState>,
) -> Result<CollectionInfo, String> {
    state
        .bili
        .opus_favorite_info()
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
    let collection = resolve_collection_or_use(&state, &input)
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
    // 缓存本轮 enriched items，供 execute 阶段复用，跳过重复 fetch + enrich
    if let Ok(mut guard) = state.import_cache.lock() {
        *guard = Some(crate::state::PreviewCache {
            key: preview_cache_key(&collection),
            items: enriched.clone(),
        });
    }
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
    let collection = resolve_collection_or_use(&state, &input)
        .await
        .map_err(|error| error.to_string())?;
    // 预览阶段若已算好同一收藏夹的 enriched items，直接复用，跳过 fetch + enrich（满收藏夹约省 5–7 分钟）
    let enriched = match take_cached_enriched(&state, &collection) {
        Some(cached) => cached,
        None => {
            let items = state
                .bili
                .fetch_collection(&collection)
                .await
                .map_err(|error| error.to_string())?;
            state
                .bili
                .enrich_items(&items)
                .await
                .map_err(|error| error.to_string())?
        }
    };
    let enriched = cache_item_covers(&state, &enriched).await;
    let assignments = input
        .item_tag_assignments
        .iter()
        .map(|assignment| (assignment.external_id.as_str(), &assignment.tag_specs))
        .collect::<HashMap<_, _>>();
    // 前端「配置标签」步骤剔除的项不会进入 assignments：
    // assignments 即为本次要导入的白名单（external_id 在其中的项才导入）；
    // 白名单为空表示全部剔除，导入 0 条。
    let total = assignments.len() as i64;
    let run_id = db::create_import_run(&state.pool, &collection, total, false)
        .await
        .map_err(|error| error.to_string())?;

    let mut imported = 0i64;
    let mut skipped = 0i64;
    let mut failed = 0i64;
    let mut errors = Vec::new();
    for item in &enriched {
        if !assignments.contains_key(item.external_id.as_str()) {
            continue; // 被前端剔除的项跳过，不导入
        }
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

// ── 删除改为移入回收站（软删除） ──
#[tauri::command]
pub async fn delete_item(state: State<'_, AppState>, item_id: i64) -> Result<(), String> {
    db::soft_delete_item(&state.pool, item_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_items(state: State<'_, AppState>, item_ids: Vec<i64>) -> Result<usize, String> {
    let count = item_ids.len();
    db::soft_delete_items(&state.pool, &item_ids)
        .await
        .map_err(|error| error.to_string())?;
    Ok(count)
}

#[tauri::command]
pub async fn delete_items_by_tag(state: State<'_, AppState>, tag_id: i64) -> Result<usize, String> {
    db::soft_delete_items_by_tag(&state.pool, tag_id)
        .await
        .map_err(|error| error.to_string())
}

// ── 回收站操作 ──
#[tauri::command]
pub async fn restore_item(state: State<'_, AppState>, item_id: i64) -> Result<(), String> {
    db::restore_item(&state.pool, item_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn restore_items(
    state: State<'_, AppState>,
    item_ids: Vec<i64>,
) -> Result<usize, String> {
    let count = item_ids.len();
    db::restore_items(&state.pool, &item_ids)
        .await
        .map_err(|error| error.to_string())?;
    Ok(count)
}

#[tauri::command]
pub async fn purge_item(state: State<'_, AppState>, item_id: i64) -> Result<(), String> {
    let cover_paths = db::purge_item(&state.pool, item_id)
        .await
        .map_err(|error| error.to_string())?;
    remove_cover_files(&state, cover_paths.into_iter().collect::<Vec<_>>());
    Ok(())
}

#[tauri::command]
pub async fn purge_items(state: State<'_, AppState>, item_ids: Vec<i64>) -> Result<usize, String> {
    let cover_paths = db::purge_items(&state.pool, &item_ids)
        .await
        .map_err(|error| error.to_string())?;
    remove_cover_files(&state, cover_paths);
    Ok(item_ids.len())
}

#[tauri::command]
pub async fn empty_trash(state: State<'_, AppState>) -> Result<usize, String> {
    let cover_paths = db::empty_trash(&state.pool)
        .await
        .map_err(|error| error.to_string())?;
    let count = cover_paths.len();
    remove_cover_files(&state, cover_paths);
    Ok(count)
}

#[tauri::command]
pub async fn list_trash(state: State<'_, AppState>) -> Result<Vec<VideoItem>, String> {
    db::list_trash(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_trash_count(state: State<'_, AppState>) -> Result<i64, String> {
    db::get_trash_count(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn auto_purge_trash(
    state: State<'_, AppState>,
    retention_days: i64,
) -> Result<usize, String> {
    let cover_paths = db::auto_purge_expired(&state.pool, retention_days)
        .await
        .map_err(|error| error.to_string())?;
    let count = cover_paths.len();
    remove_cover_files(&state, cover_paths);
    Ok(count)
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
pub async fn reorder_tag_categories(
    state: State<'_, AppState>,
    ordered_ids: Vec<i64>,
) -> Result<(), String> {
    db::reorder_tag_categories(&state.pool, &ordered_ids)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn group_tag_categories(
    state: State<'_, AppState>,
    category_ids: Vec<i64>,
) -> Result<(), String> {
    db::group_tag_categories(&state.pool, &category_ids)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn ungroup_tag_category(
    state: State<'_, AppState>,
    category_id: i64,
) -> Result<(), String> {
    db::ungroup_tag_category(&state.pool, category_id)
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
pub async fn set_item_star(
    state: State<'_, AppState>,
    item_id: i64,
    starred: bool,
) -> Result<VideoItem, String> {
    db::set_item_starred(&state.pool, item_id, starred)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn update_item_notes(
    state: State<'_, AppState>,
    item_id: i64,
    notes: String,
) -> Result<VideoItem, String> {
    // 1) 先写库，批注本身必须成功保存（健壮性底线）
    let mut item = db::update_item_notes(&state.pool, item_id, &notes)
        .await
        .map_err(|error| error.to_string())?;

    // 2) 满足「开关开启 + 仓库已配置 + 有批注内容」才触发单向同步；任何失败都只告警，不影响批注已保存
    let settings = obsidian::load_settings(&state.data_dir);
    let should_sync = settings.enabled
        && !settings.vault_path.is_empty()
        && !notes.trim().is_empty();
    if should_sync {
        match obsidian::write_or_update_note(&settings, &item) {
            Ok(Some(rel)) => {
                if db::set_item_obsidian_path(&state.pool, item_id, &rel).await.is_ok() {
                    item.obsidian_path = Some(rel);
                }
            }
            Ok(None) => {
                // 托管标记被用户手动移除：跳过同步，不覆盖用户在 Obsidian 里的内容
                eprintln!("[obsidian] 跳过同步：托管标记已被移除 (item {item_id})");
            }
            Err(e) => {
                eprintln!("[obsidian] 同步失败（批注已保存）：{e}");
            }
        }
    }
    Ok(item)
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    webbrowser::open(&url).map_err(|error| error.to_string())
}

// ── Obsidian 单向联动 ──

#[tauri::command]
pub async fn get_obsidian_settings(
    state: State<'_, AppState>,
) -> Result<obsidian::ObsidianSettings, String> {
    Ok(obsidian::load_settings(&state.data_dir))
}

#[tauri::command]
pub async fn set_obsidian_settings(
    state: State<'_, AppState>,
    settings: obsidian::ObsidianSettings,
) -> Result<(), String> {
    // 未提供仓库名时，用所选目录名兜底（Obsidian URI 的 vault 参数需要的是仓库名而非路径）
    let mut settings = settings;
    if settings.vault_name.trim().is_empty() {
        if let Some(name) = std::path::Path::new(&settings.vault_path).file_name() {
            settings.vault_name = name.to_string_lossy().to_string();
        }
    }
    obsidian::save_settings(&state.data_dir, &settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_item_obsidian_path(
    state: State<'_, AppState>,
    item_id: i64,
) -> Result<Option<String>, String> {
    db::get_item_obsidian_path(&state.pool, item_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_note_in_obsidian(
    state: State<'_, AppState>,
    item_id: i64,
) -> Result<(), String> {
    let settings = obsidian::load_settings(&state.data_dir);
    if !settings.enabled || settings.vault_path.is_empty() {
        return Err("Obsidian 联动未启用或未配置仓库目录".into());
    }
    let item = db::get_item(&state.pool, item_id)
        .await
        .map_err(|e| e.to_string())?;
    obsidian::open_in_obsidian(&settings, &item).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_items_to_obsidian(
    state: State<'_, AppState>,
    item_ids: Vec<i64>,
) -> Result<usize, String> {
    let settings = obsidian::load_settings(&state.data_dir);
    if !settings.enabled || settings.vault_path.is_empty() {
        return Err("Obsidian 联动未启用或未配置仓库目录".into());
    }
    let mut exported = 0usize;
    // 收集首个真实错误：旧实现把 write_or_update_note 的 Err 静默吞掉、只回 0，
    // 前端会误报「该收藏暂无批注」—— 有批注却导不出来时根本查不到原因。
    let mut first_error: Option<String> = None;
    for id in &item_ids {
        let item = match db::get_item(&state.pool, *id).await {
            Ok(item) => item,
            Err(_) => continue,
        };
        // 仅同步写过批注的收藏（与「保存即同步」的创建范围一致）
        if item.notes.trim().is_empty() {
            continue;
        }
        match obsidian::write_or_update_note(&settings, &item) {
            Ok(Some(rel)) => {
                let _ = db::set_item_obsidian_path(&state.pool, *id, &rel).await;
                exported += 1;
            }
            Ok(None) => {
                if first_error.is_none() {
                    first_error = Some("笔记中的托管标记已被手动移除，已跳过".into());
                }
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e.to_string());
                }
            }
        }
    }
    if exported == 0 {
        if let Some(err) = first_error {
            return Err(format!("导出失败：{err}"));
        }
    }
    Ok(exported)
}

/// 用系统文件夹选择器选 Obsidian 仓库目录（复用已有 tauri-plugin-dialog，无需前端 npm 依赖）。
#[tauri::command]
pub async fn pick_obsidian_vault(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    // `blocking_pick_folder` 是阻塞式调用，绝不能放在同步命令（主线程）里跑，
    // 否则会卡死整个 UI —— 表现为「点击启用联动后程序无响应」。
    // 移到 `spawn_blocking` 里，在后台线程弹选择器，主线程保持响应。
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择 Obsidian 仓库目录")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| format!("选择目录失败: {e}"))?;

    Ok(picked
        .and_then(|fp| fp.into_path().ok())
        .map(|p| p.to_string_lossy().to_string()))
}

/// 浏览器扩展「快速入库」本地桥的状态与令牌，供设置页展示。
/// 扩展读不到本地文件，用户需要把 token 手动复制一次到扩展选项页。
#[tauri::command]
pub fn get_bridge_info(state: State<'_, AppState>) -> Result<BridgeInfo, String> {
    let token = capture::load_or_create_token(&state.data_dir).map_err(|error| error.to_string())?;
    let port = state.bridge_port.load(Ordering::Relaxed);
    Ok(BridgeInfo {
        port,
        running: port > 0,
        token,
    })
}

/// 重新生成本地桥令牌（旧 token 立即失效，需在扩展选项页同步更新）。
#[tauri::command]
pub fn regenerate_bridge_token(state: State<'_, AppState>) -> Result<BridgeInfo, String> {
    let token = capture::regenerate_token(&state.data_dir).map_err(|error| error.to_string())?;
    let port = state.bridge_port.load(Ordering::Relaxed);
    Ok(BridgeInfo {
        port,
        running: port > 0,
        token,
    })
}

#[tauri::command]
pub async fn import_browser_bookmarks(
    state: State<'_, AppState>,
    html_content: String,
    tag_specs: Vec<TagInput>,
    item_tag_assignments: Vec<ItemTagAssignment>,
) -> Result<ImportResult, String> {
    let items = BrowserBookmarkClient::parse_bookmarks_html(&html_content)
        .map_err(|error| error.to_string())?;

    let total = items.len() as i64;
    let collection = CollectionInfo {
        source: "browser".into(),
        id: "browser-bookmarks".into(),
        title: "浏览器书签".into(),
        owner: None,
        count: total,
        url: None,
    };

    // Build a lookup from external_id to user-specified tag specs
    let assignments: std::collections::HashMap<&str, &[TagInput]> = item_tag_assignments
        .iter()
        .map(|a| (a.external_id.as_str(), a.tag_specs.as_slice()))
        .collect();

    // 前端「配置标签」步骤剔除的项不会进入 item_tag_assignments：按白名单过滤导入项
    // assignments 即导入白名单（为空表示全部剔除，导入 0 条）。
    let import_total = assignments.len() as i64;

    let run_id = db::create_import_run(&state.pool, &collection, import_total, false)
        .await
        .map_err(|error| error.to_string())?;

    let mut imported = 0i64;
    let mut skipped = 0i64;
    let mut failed = 0i64;
    let mut errors = Vec::new();

    for item in &items {
        if !assignments.contains_key(item.external_id.as_str()) {
            continue; // 被前端剔除的项跳过，不导入
        }
        let result = async {
            let (item_id, inserted) = db::upsert_item(&state.pool, item).await?;
            // Attach folder name tags from extra.folder_tags (each item only gets its own folders)
            if let Some(folder_tags) = item.extra.get("folder_tags").and_then(|v| v.as_array()) {
                for folder_name in folder_tags {
                    if let Some(name) = folder_name.as_str() {
                        if !name.is_empty() {
                            let tag_input = TagInput {
                                id: None,
                                namespace: "auto".into(),
                                name: name.to_string(),
                                color: None,
                                description: None,
                                category_id: None,
                            };
                            let tag_id = db::get_or_create_tag(&state.pool, &tag_input).await?;
                            db::attach_tag(&state.pool, item_id, tag_id).await?;
                        }
                    }
                }
            }
            // Attach user-specified tags for this specific item
            if let Some(user_tags) = assignments.get(item.external_id.as_str()) {
                for tag_spec in *user_tags {
                    let tag_id = db::get_or_create_tag(&state.pool, tag_spec).await?;
                    db::attach_tag(&state.pool, item_id, tag_id).await?;
                }
            }
            // Also apply global tag_specs (shared across all items)
            for tag_spec in &tag_specs {
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

// ── Zhihu commands ──

#[tauri::command]
pub async fn zhihu_set_cookie(state: State<'_, AppState>, cookie: String) -> Result<(), String> {
    state.zhihu.set_cookie(Some(cookie.clone()));
    state
        .save_zhihu_cookie(Some(cookie))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn zhihu_logout(state: State<'_, AppState>) -> Result<(), String> {
    state.zhihu.set_cookie(None);
    state.save_zhihu_cookie(None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn zhihu_profile(state: State<'_, AppState>) -> Result<BilibiliProfile, String> {
    let cookie = state.zhihu.get_cookie();
    if cookie.is_none() {
        return Ok(BilibiliProfile {
            is_login: false,
            name: None,
            face: None,
            mid: None,
        });
    }
    // Try API, but return logged-in even if API fails (cookie might still work for collections)
    let name = state.zhihu.get_url_token().await.ok();
    Ok(BilibiliProfile {
        is_login: true,
        name,
        face: None,
        mid: None,
    })
}

#[tauri::command]
pub async fn list_zhihu_collections(
    state: State<'_, AppState>,
) -> Result<Vec<CollectionInfo>, String> {
    state
        .zhihu
        .list_collections()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn parse_zhihu_collection_url(
    state: State<'_, AppState>,
    url: String,
) -> Result<CollectionInfo, String> {
    state
        .zhihu
        .resolve_collection(&url)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preview_zhihu_import(
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportPreview, String> {
    let collection = resolve_zhihu_collection(&state, &input)
        .await
        .map_err(|e| e.to_string())?;
    let items = state
        .zhihu
        .fetch_collection(&collection)
        .await
        .map_err(|e| e.to_string())?;
    let items: Vec<VideoItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| to_video_item(item, -(i as i64 + 1)))
        .collect();
    let partition_suggestions: Vec<PartitionSuggestion> = items
        .iter()
        .filter_map(|item| item.partition_name.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .map(|name| PartitionSuggestion {
            name,
            count: 0,
            selected: false,
        })
        .collect();
    Ok(ImportPreview {
        collection,
        items,
        partition_suggestions,
    })
}

#[tauri::command]
pub async fn execute_zhihu_import(
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportResult, String> {
    let collection = resolve_zhihu_collection(&state, &input)
        .await
        .map_err(|e| e.to_string())?;
    let items = state
        .zhihu
        .fetch_collection(&collection)
        .await
        .map_err(|e| e.to_string())?;
    let enriched = state
        .zhihu
        .enrich_items(&items)
        .await
        .map_err(|e| e.to_string())?;
    let assignments = input
        .item_tag_assignments
        .iter()
        .map(|a| (a.external_id.as_str(), &a.tag_specs))
        .collect::<HashMap<_, _>>();
    // 前端「配置标签」步骤剔除的项不会进入 assignments：assignments 即导入白名单
    let total = assignments.len() as i64;
    let run_id = db::create_import_run(&state.pool, &collection, total, false)
        .await
        .map_err(|e| e.to_string())?;

    let mut imported = 0i64;
    let mut skipped = 0i64;
    let mut failed = 0i64;
    let mut errors = Vec::new();
    for item in &enriched {
        if !assignments.contains_key(item.external_id.as_str()) {
            continue; // 被前端剔除的项跳过，不导入
        }
        let result = async {
            let (item_id, inserted) = db::upsert_item(&state.pool, item).await?;
            let tag_specs: &[TagInput] = assignments
                .get(item.external_id.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
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
        .map_err(|e| e.to_string())?;
    db::build_import_result(&state.pool, run_id)
        .await
        .map_err(|e| e.to_string())
}

async fn resolve_zhihu_collection(
    state: &AppState,
    input: &ImportRequest,
) -> Result<CollectionInfo, AppError> {
    match input.kind {
        crate::models::ImportKind::Favorites => {
            let media_id = input
                .media_id
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请选择收藏夹".into()))?;
            state.zhihu.resolve_collection(media_id).await
        }
        crate::models::ImportKind::PublicUrl => {
            let url = input
                .url
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请提供收藏夹链接".into()))?;
            state.zhihu.resolve_collection(url).await
        }
    }
}

// ── Zhihu browser login ──

#[tauri::command]
pub async fn zhihu_browser_login(
    state: State<'_, AppState>,
    cookie: String,
) -> Result<BilibiliProfile, String> {
    state.zhihu.set_cookie(Some(cookie.clone()));
    let _ = state.save_zhihu_cookie(Some(cookie));
    let name = state.zhihu.get_url_token().await.ok();
    Ok(BilibiliProfile {
        is_login: true,
        name,
        face: None,
        mid: None,
    })
}

// ── CSDN commands ──

#[tauri::command]
pub async fn list_csdn_collections(
    state: State<'_, AppState>,
    username: String,
) -> Result<Vec<CollectionInfo>, String> {
    state
        .csdn
        .list_collections_for_user(&username)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn parse_csdn_collection_url(
    state: State<'_, AppState>,
    url: String,
) -> Result<CollectionInfo, String> {
    state
        .csdn
        .resolve_collection(&url)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preview_csdn_import(
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportPreview, String> {
    let collection = resolve_csdn_collection(&state, &input)
        .await
        .map_err(|e| e.to_string())?;
    let items = state
        .csdn
        .fetch_collection(&collection)
        .await
        .map_err(|e| e.to_string())?;
    let items: Vec<VideoItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| to_video_item(item, -(i as i64 + 1)))
        .collect();
    Ok(ImportPreview {
        collection,
        items,
        partition_suggestions: vec![],
    })
}

#[tauri::command]
pub async fn execute_csdn_import(
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportResult, String> {
    let collection = resolve_csdn_collection(&state, &input)
        .await
        .map_err(|e| e.to_string())?;
    let items = state
        .csdn
        .fetch_collection(&collection)
        .await
        .map_err(|e| e.to_string())?;
    let enriched = state
        .csdn
        .enrich_items(&items)
        .await
        .map_err(|e| e.to_string())?;
    // Download cover images to local storage
    let enriched = cache_csdn_covers(&state, &enriched).await;
    let assignments = input
        .item_tag_assignments
        .iter()
        .map(|a| (a.external_id.as_str(), &a.tag_specs))
        .collect::<HashMap<_, _>>();
    // 前端「配置标签」步骤剔除的项不会进入 assignments：assignments 即导入白名单
    let total = assignments.len() as i64;
    let run_id = db::create_import_run(&state.pool, &collection, total, false)
        .await
        .map_err(|e| e.to_string())?;

    let mut imported = 0i64;
    let mut skipped = 0i64;
    let mut failed = 0i64;
    let mut errors = Vec::new();
    for item in &enriched {
        if !assignments.contains_key(item.external_id.as_str()) {
            continue; // 被前端剔除的项跳过，不导入
        }
        let result = async {
            let (item_id, inserted) = db::upsert_item(&state.pool, item).await?;
            let tag_specs: &[TagInput] = assignments
                .get(item.external_id.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
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
        .map_err(|e| e.to_string())?;
    db::build_import_result(&state.pool, run_id)
        .await
        .map_err(|e| e.to_string())
}

async fn resolve_csdn_collection(
    state: &AppState,
    input: &ImportRequest,
) -> Result<CollectionInfo, AppError> {
    match input.kind {
        crate::models::ImportKind::Favorites => {
            let media_id = input
                .media_id
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请选择收藏夹".into()))?;
            let username = input
                .url
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请提供 CSDN 用户名".into()))?;
            // For CSDN favorites mode, media_id is the folder ID and url is the username
            Ok(CollectionInfo {
                source: "csdn".into(),
                id: media_id.to_string(),
                title: String::new(),
                owner: Some(username.to_string()),
                count: 0,
                url: None,
            })
        }
        crate::models::ImportKind::PublicUrl => {
            let url = input
                .url
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请提供收藏夹链接".into()))?;
            state.csdn.resolve_collection(url).await
        }
    }
}

// ── GitHub commands ──

#[tauri::command]
pub async fn list_github_stars(
    state: State<'_, AppState>,
    username: String,
) -> Result<Vec<CollectionInfo>, String> {
    state
        .github
        .list_stars_for_user(&username)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preview_github_import(
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportPreview, String> {
    let collection = resolve_github_collection(&state, &input)
        .await
        .map_err(|e| e.to_string())?;
    let items = state
        .github
        .fetch_collection(&collection)
        .await
        .map_err(|e| e.to_string())?;
    let items: Vec<VideoItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| to_video_item(item, -(i as i64 + 1)))
        .collect();
    Ok(ImportPreview {
        collection,
        items,
        partition_suggestions: vec![],
    })
}

#[tauri::command]
pub async fn execute_github_import(
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportResult, String> {
    let collection = resolve_github_collection(&state, &input)
        .await
        .map_err(|e| e.to_string())?;
    let items = state
        .github
        .fetch_collection(&collection)
        .await
        .map_err(|e| e.to_string())?;
    let enriched = state
        .github
        .enrich_items(&items)
        .await
        .map_err(|e| e.to_string())?;
    let assignments = input
        .item_tag_assignments
        .iter()
        .map(|a| (a.external_id.as_str(), &a.tag_specs))
        .collect::<HashMap<_, _>>();
    // 前端「配置标签」步骤剔除的项不会进入 assignments：assignments 即导入白名单
    let total = assignments.len() as i64;
    let run_id = db::create_import_run(&state.pool, &collection, total, false)
        .await
        .map_err(|e| e.to_string())?;

    let mut imported = 0i64;
    let mut skipped = 0i64;
    let mut failed = 0i64;
    let mut errors = Vec::new();
    for item in &enriched {
        if !assignments.contains_key(item.external_id.as_str()) {
            continue; // 被前端剔除的项跳过，不导入
        }
        let result = async {
            let (item_id, inserted) = db::upsert_item(&state.pool, item).await?;
            let tag_specs: &[TagInput] = assignments
                .get(item.external_id.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
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
        .map_err(|e| e.to_string())?;
    db::build_import_result(&state.pool, run_id)
        .await
        .map_err(|e| e.to_string())
}

async fn resolve_github_collection(
    state: &AppState,
    input: &ImportRequest,
) -> Result<CollectionInfo, AppError> {
    let username = input
        .url
        .as_deref()
        .ok_or_else(|| AppError::InvalidInput("请提供 GitHub 用户名".into()))?;
    state.github.resolve_collection(username).await
}

// ── 收藏库导出 / 导入 ──

#[tauri::command]
pub async fn export_collection(
    state: State<'_, AppState>,
    item_ids: Option<Vec<i64>>,
) -> Result<String, String> {
    let export = db::export_items(&state.pool, item_ids)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_string(&export).map_err(|e| e.to_string())
}

/// 弹出「另存为」对话框，让用户选择导出文件的保存位置并写入内容，
/// 返回最终保存的完整路径（含用户手动填写的文件名），供前端 toast 提示。
#[tauri::command]
pub async fn save_export_file(
    app: tauri::AppHandle,
    content: String,
    suggested_name: String,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    // 弹保存对话框，让用户选择路径（cancel 返回 None 时直接放弃，不写文件）
    let path = app
        .dialog()
        .file()
        .set_file_name(&suggested_name)
        .blocking_save_file();

    // FilePath 可能是 Url 或 Path，这里统一转成 PathBuf
    let Some(path) = path.and_then(|fp| fp.into_path().ok()) else {
        return Err("已取消保存".into());
    };

    // 确保文件以 .json 结尾（用户若没填扩展名则补上）
    let path = match path.extension() {
        Some(_) => path,
        None => path.with_extension("json"),
    };

    std::fs::write(&path, content).map_err(|e| format!("写入文件失败：{e}"))?;

    Ok(path.to_string_lossy().into_owned())
}

/// 自动备份：把整库（含标签与分类）导出为 JSON，直接写入指定备份文件夹，
/// 内容与设置页手动「导出全部」（export_items 全量）一致。
/// 文件名由前端按本地时间生成；后端只取文件名部分，杜绝路径分隔符/上级目录注入。
#[tauri::command]
pub async fn backup_now(
    state: State<'_, AppState>,
    folder: String,
    file_name: String,
) -> Result<String, String> {
    let dir = std::path::PathBuf::from(&folder);
    if !dir.is_dir() {
        return Err(format!("备份文件夹不存在：{folder}"));
    }
    let file_name = std::path::Path::new(&file_name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "非法的备份文件名".to_string())?;
    let export = db::export_items(&state.pool, None)
        .await
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&export).map_err(|e| e.to_string())?;
    let path = dir.join(file_name);
    std::fs::write(&path, json).map_err(|e| format!("写入备份文件失败：{e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// 弹出「选择文件夹」对话框（自动备份用）。阻塞式对话框放在 spawn_blocking 后台线程，
/// 避免在主线程阻塞整个 UI（与 pick_obsidian_vault 同款处理）。
#[tauri::command]
pub async fn pick_backup_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择自动备份文件夹")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| format!("选择文件夹失败: {e}"))?;

    Ok(picked
        .and_then(|fp| fp.into_path().ok())
        .map(|p| p.to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn import_collection(
    state: State<'_, AppState>,
    payload: String,
) -> Result<ImportResult, String> {
    let (result, new_items) = db::import_collection(&state.pool, &payload)
        .await
        .map_err(|e| e.to_string())?;

    // 像实时 B站收藏夹导入那样，把新导入项的封面下载到本地缓存，
    // 保证导入后封面正常显示（尤其 B站 http 封面在 WebView 中无法直接加载）。
    if !new_items.is_empty() {
        let mut items = new_items;
        cache_imported_covers(&state, &mut items).await;
        for item in &items {
            if let Some(path) = &item.cover_local_path {
                let _ = db::set_item_cover_local_path(
                    &state.pool,
                    &item.source,
                    &item.external_id,
                    path,
                )
                .await;
            }
        }
    }

    Ok(result)
}

/// 维护操作：把已存在但缺本地封面的 B站 / CSDN 项重新下载缓存（复刻实时导入行为）。
#[tauri::command]
pub async fn recache_covers(state: State<'_, AppState>) -> Result<RecacheResult, String> {
    let mut items = db::fetch_items_needing_cover_cache(&state.pool)
        .await
        .map_err(|e| e.to_string())?;
    let total = items.len() as i64;

    let mut cached: i64 = 0;
    let mut failed: i64 = 0;
    let mut errors: Vec<String> = Vec::new();

    for item in items.iter_mut() {
        let url = match item.cover_url.as_deref().filter(|v| !v.is_empty()) {
            Some(v) => v,
            None => {
                failed += 1;
                errors.push(format!(
                    "{}:{} 缺少 cover_url",
                    item.source, item.external_id
                ));
                continue;
            }
        };

        let download = match item.source.as_str() {
            "bilibili" => state.bili.download_cover(url).await,
            "csdn" => state.csdn.download_cover(url).await,
            other => {
                failed += 1;
                errors.push(format!(
                    "{}:{} 未知来源 {}",
                    item.source, item.external_id, other
                ));
                continue;
            }
        };

        match download {
            Ok((bytes, extension)) => {
                match save_cover_file(&state, &item.source, &item.external_id, &bytes, &extension) {
                    Ok(path) => {
                        item.cover_local_path = Some(path);
                    }
                    Err(e) => {
                        failed += 1;
                        errors.push(format!(
                            "{}:{} 保存封面失败: {}",
                            item.source, item.external_id, e
                        ));
                        continue;
                    }
                }
            }
            Err(e) => {
                failed += 1;
                errors.push(format!(
                    "{}:{} 下载封面失败: {}",
                    item.source, item.external_id, e
                ));
                continue;
            }
        }

        // 写回 cover_local_path
        match &item.cover_local_path {
            Some(path) => {
                if let Err(e) = db::set_item_cover_local_path(
                    &state.pool,
                    &item.source,
                    &item.external_id,
                    path,
                )
                .await
                {
                    failed += 1;
                    errors.push(format!(
                        "{}:{} 写回 cover_local_path 失败: {}",
                        item.source, item.external_id, e
                    ));
                } else {
                    cached += 1;
                }
            }
            None => {
                failed += 1;
                errors.push(format!(
                    "{}:{} 下载后无本地路径",
                    item.source, item.external_id
                ));
            }
        }
    }

    Ok(RecacheResult {
        total,
        cached,
        failed,
        errors,
    })
}
