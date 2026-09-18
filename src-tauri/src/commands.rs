use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

// Emitter：向前端推事件（`netease://sync`）需要它
use tauri::{Emitter, Manager, State};

use crate::capture;
use crate::cover_cache;
use crate::db;
use crate::error::AppError;
use crate::models::{
    BilibiliProfile, BridgeInfo, CollectionExport, CollectionInfo, CollectionStats, CoverCacheStatus,
    DuplicateGroup, ExportItem,
    ExportTag, ImportPreview, ImportRequest, ImportResult, ItemFilters, ItemTagAssignment,
    NeteaseSyncReport, PartitionSuggestion, QrSession, QrStatus, QuickCaptureRequest,
    QuickCaptureResult, RecacheResult, Tag, TagCategory, TagInput, TrackResolveResult, VideoItem,
};
use crate::nowplaying;
use crate::obsidian;
use crate::open_prefs::{self, OpenPrefs};
use crate::source::browser::BrowserBookmarkClient;
use crate::source::netease;
use crate::source::SourceAdapter;
use crate::state::AppState;

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
pub async fn get_collection_stats(
    state: State<'_, AppState>,
    range_days: i64,
) -> Result<CollectionStats, String> {
    db::get_collection_stats(&state.pool, range_days)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_duplicate_groups(
    state: State<'_, AppState>,
) -> Result<Vec<DuplicateGroup>, String> {
    db::find_duplicate_groups(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn merge_duplicate_items(
    state: State<'_, AppState>,
    keep_id: i64,
    remove_ids: Vec<i64>,
) -> Result<(), String> {
    let merged = db::merge_duplicate_items(&state.pool, keep_id, &remove_ids)
        .await
        .map_err(|error| error.to_string())?;
    // 合并后同步 Obsidian（若启用）：失败不阻塞合并本身
    if !merged.trim().is_empty() {
        let _ = crate::notes::save_notes(&state, keep_id, &merged).await;
    }
    Ok(())
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
    // 「写库 → 同步 Obsidian → 回写 obsidian_path」集中在 notes::save_notes，
    // 与浏览器扩展侧边栏的 /note 端点共用同一份实现，避免两处漂移。
    crate::notes::save_notes(&state, item_id, &notes)
        .await
        .map_err(|error| error.to_string())
}

/// 读取某条收藏的轻量批注（独立于 Obsidian 笔记 `items.notes`）。
///
/// 批注与应用内每条收藏下方的「批注按钮」、浏览器侧边栏「批注模式」共用同一份数据，
/// 三者互相同步；批注绝不进 Obsidian。
#[tauri::command]
pub async fn get_item_annotation(
    state: State<'_, AppState>,
    item_id: i64,
) -> Result<String, String> {
    db::get_annotation(&state.pool, item_id)
        .await
        .map_err(|error| error.to_string())
}

/// 写入某条收藏的轻量批注，返回刷新后的 item 快照。
///
/// 只落本地 `annotations` 表，不触发 Obsidian 同步（那是 `items.notes` 的职责）。
#[tauri::command]
pub async fn update_item_annotation(
    state: State<'_, AppState>,
    item_id: i64,
    annotation: String,
) -> Result<VideoItem, String> {
    db::set_annotation(&state.pool, item_id, &annotation)
        .await
        .map_err(|error| error.to_string())?;
    db::get_item(&state.pool, item_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    webbrowser::open(&url).map_err(|error| error.to_string())
}

/// 在网易云**桌面客户端**打开并播放该歌曲（P0，见 DEVELOPMENT.md 9.x）。
///
/// 返回 `true` = 已递交给客户端；`false` = 客户端不可用（未安装 / 协议未注册 / 打开失败），
/// 已自动回退到浏览器打开网页版，由前端给一次 toast 说明。
///
/// ⚠️ `true` **不等于**客户端真的处理了：`ShellExecuteW` 返回码 >32 只能证明"已递交"。
/// 实测网易云的 `orpheus://openurl` 同样返回 42 却毫无反应，所以别拿它向用户宣称成功。
#[tauri::command]
pub async fn open_in_netease(
    state: State<'_, AppState>,
    external_id: String,
    fallback_url: String,
) -> Result<bool, String> {
    // 用户在设置页选了「浏览器」时直接走网页，连深链都不发。
    // 这一步放在 Rust 侧而不是前端 if：前端那边已经有好几处可能触发打开的路径
    // （卡片 / 将来的回收站 / 批量操作），偏好必须由后端兜底，不能指望每处都记得判断。
    // ⚠️ 这里必须 `spawn_blocking`：进程首次走 ShellExecute 实测要 ~310 ms
    // （见 `uri.rs` 里的 bench 注释，换 API、预热都消不掉）。直接放在 async 命令体内
    // 会占住 tokio 的工作线程那么久，跟着排队的还有封面缓存、同步这些后台任务。
    let prefs = open_prefs::load_open_prefs(&state.data_dir);
    let task = tokio::task::spawn_blocking(move || {
        if open_prefs::target_for(&prefs, "netease") == open_prefs::OpenTarget::Browser {
            return OpenOutcome::Browser;
        }
        match netease::open_song_in_client(&external_id) {
            Ok(()) => OpenOutcome::Client,
            Err(err) => OpenOutcome::Failed(err.to_string()),
        }
    });

    let outcome = task.await.map_err(|e| e.to_string())?;
    match outcome {
        OpenOutcome::Client => Ok(true),
        OpenOutcome::Browser => {
            webbrowser::open(&fallback_url).map_err(|e| e.to_string())?;
            Ok(false)
        }
        OpenOutcome::Failed(err) => {
            // 协议未注册等情况都属于"客户端不可用"，静默回退即可
            eprintln!("[netease] 客户端打开失败，回退浏览器：{err}");
            webbrowser::open(&fallback_url).map_err(|e| e.to_string())?;
            Ok(false)
        }
    }
}

/// `spawn_blocking` 任务的返回值：`AppError` 不带 Send，隔着线程边界只搬字符串。
enum OpenOutcome {
    Client,
    Browser,
    Failed(String),
}

// ── 打开方式偏好（客户端优先 / 浏览器） ──

#[tauri::command]
pub async fn get_open_prefs(state: State<'_, AppState>) -> Result<OpenPrefs, String> {
    Ok(open_prefs::load_open_prefs(&state.data_dir))
}

/// 设置某个来源的打开方式。返回完整偏好，前端直接拿去更新本地状态，
/// 省掉一次回读，也保证 UI 显示的与刚落盘的一致。
#[tauri::command]
pub async fn set_open_target(
    state: State<'_, AppState>,
    source: String,
    target: String,
) -> Result<OpenPrefs, String> {
    let target = match target.as_str() {
        "client" => open_prefs::OpenTarget::Client,
        "browser" => open_prefs::OpenTarget::Browser,
        other => return Err(format!("未知的打开方式：{other}")),
    };
    open_prefs::set_target(&state.data_dir, &source, target).map_err(|e| e.to_string())
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
pub async fn open_note_in_obsidian(state: State<'_, AppState>, item_id: i64) -> Result<(), String> {
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
            Ok(rel) => {
                let _ = db::set_item_obsidian_path(&state.pool, *id, &rel).await;
                exported += 1;
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
    let token =
        capture::load_or_create_token(&state.data_dir).map_err(|error| error.to_string())?;
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

/// 浏览器扩展后台桥是否设为「开机自启」（仅 Windows 有效）。
///
/// 开启后 `bili-collector.exe --bridge-only` 会在登录时静默拉起，
/// 这样即使没打开主界面，浏览器扩展也能照常收藏。非 Windows 恒返回 false。
#[tauri::command]
pub fn get_bridge_autostart() -> Result<bool, String> {
    #[cfg(windows)]
    {
        Ok(read_bridge_autostart())
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

/// 设置浏览器扩展后台桥是否「开机自启」。
///
/// - 开启：在 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 写入
///   `collectorlite-bridge` = `"<exe 路径>" --bridge-only`。
/// - 关闭：删除该注册表值。
/// - 非 Windows 平台不支持，恒返回 false。
#[tauri::command]
pub fn set_bridge_autostart(enabled: bool) -> Result<bool, String> {
    #[cfg(windows)]
    {
        write_bridge_autostart(enabled).map(|()| enabled)
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Ok(false)
    }
}

#[cfg(windows)]
fn read_bridge_autostart() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    let reg_path = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    let Ok(key) =
        winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(reg_path, KEY_READ)
    else {
        return false;
    };
    key.get_value::<String, _>("collectorlite-bridge").is_ok()
}

#[cfg(windows)]
fn write_bridge_autostart(enabled: bool) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    let reg_path = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    let key = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(reg_path)
        .map(|(k, _)| k)
        .map_err(|e| e.to_string())?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let value = format!("\"{}\" --bridge-only", exe.to_string_lossy());
        key.set_value("collectorlite-bridge", &value)
            .map_err(|e| e.to_string())?;
    } else {
        let _ = key.delete_value("collectorlite-bridge");
    }
    Ok(())
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

// ── Netease commands ──

#[tauri::command]
pub async fn netease_set_cookie(
    state: State<'_, AppState>,
    cookie: String,
) -> Result<(), String> {
    state.netease.set_cookie(Some(cookie.clone()));
    state
        .save_netease_cookie(Some(cookie))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn netease_logout(state: State<'_, AppState>) -> Result<(), String> {
    state.netease.set_cookie(None);
    state.save_netease_cookie(None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn netease_profile(state: State<'_, AppState>) -> Result<BilibiliProfile, String> {
    if state.netease.cookie_value().is_none() {
        return Ok(BilibiliProfile {
            is_login: false,
            name: None,
            face: None,
            mid: None,
        });
    }
    match state.netease.account_info().await {
        Ok((uid, nickname)) => Ok(BilibiliProfile {
            is_login: true,
            name: nickname,
            face: None,
            // profile 结构的 mid 是数字，网易云 uid 也是纯数字，解析失败就留空
            mid: uid.parse::<i64>().ok(),
        }),
        // cookie 还在但已失效（过期 / 被风控）：如实报未登录，让前端提示重新粘贴
        Err(_) => Ok(BilibiliProfile {
            is_login: false,
            name: None,
            face: None,
            mid: None,
        }),
    }
}

#[tauri::command]
pub async fn get_netease_sync_settings(
    state: State<'_, AppState>,
) -> Result<netease::SyncSettings, String> {
    Ok(netease::load_sync_settings(&state.data_dir))
}

#[tauri::command]
pub async fn save_netease_sync_settings(
    state: State<'_, AppState>,
    mut settings: netease::SyncSettings,
) -> Result<netease::SyncSettings, String> {
    // 下限保护：网易云对频繁请求敏感（实测风控码 8821 / -462），低于 5 分钟容易中招
    if settings.interval_minutes < netease::MIN_SYNC_INTERVAL_MINUTES {
        settings.interval_minutes = netease::MIN_SYNC_INTERVAL_MINUTES;
    }
    // 前端可能从 playlistIds 里删掉了某个歌单，顺手清掉它的水位
    netease::prune_stale_water_marks(&mut settings);
    netease::save_sync_settings(&state.data_dir, &settings).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
pub async fn sync_netease(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    force: bool,
) -> Result<NeteaseSyncReport, String> {
    // `state` 是 State<'_, AppState>，不是 Copy，直接进 async 会被搬走 —— 先取引用
    let state: &AppState = &state;
    sync_netease_inner(&app, state, force)
        .await
        .map_err(|e| e.to_string())
}

/// 本轮压根没跑（未登录 / 已关闭 / 太频繁 / 没登记歌单）时的返回。
/// 前端靠 `skippedReason` 区分「跑了但没变化」和「根本没跑」。
fn netease_sync_skipped(reason: &str) -> NeteaseSyncReport {
    NeteaseSyncReport {
        added: 0,
        removed: 0,
        playlists: 0,
        synced_at: None,
        skipped_reason: Some(reason.to_string()),
        errors: Vec::new(),
    }
}

/// 增量同步主流程。`force = true`（用户点「立即同步」）会忽略开关与间隔，
/// 但**不会**忽略登录态和歌单登记 —— 没登录强行跑只会白挨一次风控。
async fn sync_netease_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    force: bool,
) -> Result<NeteaseSyncReport, AppError> {
    let mut settings = netease::load_sync_settings(&state.data_dir);
    let now = db::now_seconds();

    if !force {
        if !settings.enabled {
            return Ok(netease_sync_skipped("自动同步已关闭"));
        }
        if let Some(last) = settings.last_sync_at {
            let minutes = settings
                .interval_minutes
                .max(netease::MIN_SYNC_INTERVAL_MINUTES) as i64
                * 60;
            if now - last < minutes {
                return Ok(netease_sync_skipped(&format!(
                    "距上次同步不足 {} 分钟",
                    settings.interval_minutes
                )));
            }
        }
    }
    if state.netease.cookie_value().is_none() {
        return Ok(netease_sync_skipped("未登录网易云，跳过同步"));
    }
    if settings.playlist_ids.is_empty() {
        return Ok(netease_sync_skipped("还没有登记要同步的歌单，先导入一次歌单"));
    }

    let mut remote_union: HashSet<String> = HashSet::new();
    let mut to_import: Vec<ExportItem> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut playlists: i64 = 0;
    // ⚠️ 只要有任意一个歌单没拉成功，就**不许**做取消收藏清理：
    // remote_union 缺了它的曲目，会把整张歌单误判成「用户全删了」。
    let mut all_playlists_ok = true;

    for playlist_id in settings.playlist_ids.clone() {
        let tracks = match state.netease.fetch_track_ids(&playlist_id).await {
            Ok(tracks) => tracks,
            Err(err) => {
                all_playlists_ok = false;
                errors.push(format!("歌单 {playlist_id} 曲目列表拉取失败：{err}"));
                continue;
            }
        };
        // ⚠️ 歌单突然变成 0 首：99% 是接口变了 / 参数不对，而不是用户真把两千首删空了。
        // 拿这个当「全部取消收藏」去软删除是灾难性的，所以直接判本轮不可信。
        if tracks.is_empty() {
            all_playlists_ok = false;
            errors.push(format!(
                "歌单 {playlist_id} 返回 0 首曲目，疑似接口异常，本轮不做取消收藏清理"
            ));
            continue;
        }
        playlists += 1;

        let water_mark = settings.water_marks.get(&playlist_id).copied();
        let plan = netease::plan_incremental(&tracks, water_mark);
        if let Some(mark) = plan.new_water_mark {
            settings.water_marks.insert(playlist_id.clone(), mark);
        }
        let added_at: HashMap<i64, Option<i64>> = tracks.iter().copied().collect();
        for (id, _) in &tracks {
            remote_union.insert(id.to_string());
        }

        if plan.new_ids.is_empty() {
            continue;
        }
        match state
            .netease
            .fetch_songs(&plan.new_ids, &added_at, &playlist_id)
            .await
        {
            Ok(items) => {
                for item in items {
                    to_import.push(ExportItem {
                        source: item.source,
                        external_id: item.external_id,
                        source_url: item.source_url,
                        title: item.title,
                        description: item.description,
                        cover_url: item.cover_url,
                        author_name: item.author_name,
                        author_id: item.author_id,
                        partition_name: item.partition_name,
                        published_at: item.published_at,
                        duration: item.duration,
                        favorite_time: item.favorite_time,
                        notes: String::new(),
                        obsidian_path: None,
                        starred: false,
                        starred_at: None,
                        extra: item.extra,
                        tags: Vec::new(),
                    });
                }
            }
            Err(err) => errors.push(format!("歌单 {playlist_id} 曲目详情拉取失败：{err}")),
        }
    }

    // ── 取消收藏 → 软删除进回收站（用户拍板：默认开，可关） ──
    let mut removed: i64 = 0;
    if settings.auto_remove_unfavorited {
        if all_playlists_ok {
            let rows = db::list_netease_active_items(&state.pool).await?;
            let rows: Vec<(i64, String, Option<String>)> = rows
                .into_iter()
                .map(|(id, external_id, extra)| {
                    (id, external_id, netease::extra_playlist_id(&extra))
                })
                .collect();
            let synced: HashSet<String> = settings.playlist_ids.iter().cloned().collect();
            let doomed = netease::pick_unfavorited(&rows, &synced, &remote_union);
            if !doomed.is_empty() {
                // 单事务批量软删（3.15）：逐条删每条一次 fsync
                removed = db::soft_delete_items_bulk(&state.pool, &doomed).await? as i64;
            }
        } else {
            errors.push("有歌单拉取失败，本轮跳过「取消收藏」清理以避免误删".into());
        }
    }

    // ── 新歌入库：复用 JSON 导入（单事务 + 回收站恢复语义） ──
    let mut added: i64 = 0;
    if !to_import.is_empty() {
        let payload = serde_json::to_string(&CollectionExport {
            format_version: 1,
            exported_at: now,
            app: "collectorlite".into(),
            items: to_import,
        })
        .map_err(|e| AppError::Other(format!("组装同步数据失败：{e}")))?;
        let (result, _) = db::import_collection(&state.pool, &payload).await?;
        added = result.imported;
        errors.extend(result.errors);
    }

    settings.last_sync_at = Some(now);
    // 水位已经更新了；写失败只影响下次是否重抓，不该让整轮同步报错
    let _ = netease::save_sync_settings(&state.data_dir, &settings);

    if added > 0 {
        cover_cache::spawn_cover_cache(app);
    }

    Ok(NeteaseSyncReport {
        added,
        removed,
        playlists,
        synced_at: Some(now),
        skipped_reason: None,
        errors,
    })
}

/// 启动后延迟多久跑第一次同步：别和首屏渲染、封面断点续传抢连接池和磁盘。
const NETEASE_STARTUP_DELAY_SECS: u64 = 20;

/// 拉起网易云自动同步的后台循环（启动时调一次，整个进程生命周期内常驻）。
///
/// 间隔每次都从配置里重新读，改了「同步频率」不用重启应用。
/// 关掉开关时循环**不退出**，只是每轮都空转 —— 否则重新打开还得重启。
pub fn start_netease_sync_loop(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(NETEASE_STARTUP_DELAY_SECS)).await;
        loop {
            let wait_seconds = {
                let guard = app.state::<AppState>();
                let state: &AppState = &guard;
                let settings = netease::load_sync_settings(&state.data_dir);
                let minutes = settings
                    .interval_minutes
                    .max(netease::MIN_SYNC_INTERVAL_MINUTES) as u64;

                // 没登录 / 没登记歌单 / 开关关着 → 直接空转，连请求都不发
                if settings.enabled
                    && !settings.playlist_ids.is_empty()
                    && state.netease.cookie_value().is_some()
                {
                    match sync_netease_inner(&app, state, false).await {
                        Ok(report) => {
                            if report.added > 0 || report.removed > 0 {
                                let _ = app.emit("netease://sync", &report);
                            }
                        }
                        Err(err) => eprintln!("[netease] 自动同步失败：{err}"),
                    }
                }
                minutes * 60
            };
            tokio::time::sleep(std::time::Duration::from_secs(wait_seconds)).await;
        }
    });
}

#[tauri::command]
pub async fn list_netease_collections(
    state: State<'_, AppState>,
) -> Result<Vec<CollectionInfo>, String> {
    state
        .netease
        .list_collections()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn parse_netease_collection_url(
    state: State<'_, AppState>,
    url: String,
) -> Result<CollectionInfo, String> {
    state
        .netease
        .resolve_collection(&url)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preview_netease_import(
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportPreview, String> {
    let collection = resolve_netease_collection(&state, &input)
        .await
        .map_err(|e| e.to_string())?;
    let items = state
        .netease
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
pub async fn execute_netease_import(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: ImportRequest,
) -> Result<ImportResult, String> {
    let collection = resolve_netease_collection(&state, &input)
        .await
        .map_err(|e| e.to_string())?;
    let items = state
        .netease
        .fetch_collection(&collection)
        .await
        .map_err(|e| e.to_string())?;
    let enriched = state
        .netease
        .enrich_items(&items)
        .await
        .map_err(|e| e.to_string())?;

    // 与其他来源一致：前端「配置标签」步骤剔除的项不会进 assignments，故 assignments 即白名单
    let assignments: HashMap<&str, &Vec<TagInput>> = input
        .item_tag_assignments
        .iter()
        .map(|a| (a.external_id.as_str(), &a.tag_specs))
        .collect();

    // 导出结构里分类用**名字**表示，而前端传的是 category_id，这里查一次做映射
    let category_names: HashMap<i64, String> = db::list_tag_categories(&state.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();

    let export_items: Vec<ExportItem> = enriched
        .iter()
        .filter(|item| assignments.contains_key(item.external_id.as_str()))
        .map(|item| ExportItem {
            source: item.source.clone(),
            external_id: item.external_id.clone(),
            source_url: item.source_url.clone(),
            title: item.title.clone(),
            description: item.description.clone(),
            cover_url: item.cover_url.clone(),
            author_name: item.author_name.clone(),
            author_id: item.author_id.clone(),
            partition_name: item.partition_name.clone(),
            published_at: item.published_at,
            duration: item.duration,
            favorite_time: item.favorite_time,
            notes: String::new(),
            obsidian_path: None,
            starred: false,
            starred_at: None,
            extra: item.extra.clone(),
            tags: assignments
                .get(item.external_id.as_str())
                .map(|specs| {
                    specs
                        .iter()
                        .map(|t| ExportTag {
                            namespace: t.namespace.clone(),
                            name: t.name.clone(),
                            color: t.color.clone(),
                            category: t
                                .category_id
                                .and_then(|id| category_names.get(&id).cloned()),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();

    let payload = serde_json::to_string(&CollectionExport {
        format_version: 1,
        exported_at: db::now_seconds(),
        app: "collectorlite".into(),
        items: export_items,
    })
    .map_err(|e| format!("组装导入数据失败：{e}"))?;

    // 复用 JSON 导入：它把所有写操作收进**一个事务**（3.15：3000 条 48 s → 0.6 s），
    // 还自带回收站恢复语义。逐条 upsert 在 2934 首这个量级会跑几十秒纯 fsync。
    let (result, _new_items) = db::import_collection(&state.pool, &payload)
        .await
        .map_err(|e| e.to_string())?;

    // 登记进自动同步范围，并用本次导入的最大收藏时间当水位：
    // 没有水位的话，「导入之后、首次同步之前」新收藏的歌会被漏掉。
    let water_mark = enriched.iter().filter_map(|item| item.favorite_time).max();
    if let Err(err) =
        netease::register_playlist_for_sync(&state.data_dir, &collection.id, water_mark)
    {
        // 登记失败不影响本次导入结果，只写日志
        eprintln!("[netease] 登记同步歌单失败：{err}");
    }

    // 封面不等：数据已落库，交给后台队列慢慢补（3.17）
    cover_cache::spawn_cover_cache(&app);

    Ok(result)
}

async fn resolve_netease_collection(
    state: &AppState,
    input: &ImportRequest,
) -> Result<CollectionInfo, AppError> {
    match input.kind {
        crate::models::ImportKind::Favorites => {
            let media_id = input
                .media_id
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请选择歌单".into()))?;
            state.netease.resolve_collection(media_id).await
        }
        crate::models::ImportKind::PublicUrl => {
            let url = input
                .url
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput("请提供歌单链接".into()))?;
            state.netease.resolve_collection(url).await
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
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: String,
) -> Result<ImportResult, String> {
    let (result, _new_items) = db::import_collection(&state.pool, &payload)
        .await
        .map_err(|e| e.to_string())?;

    // 封面**不在这里等**：数据已经落库，封面交给后台任务慢慢补，
    // 否则 3000 条要卡在导入中十几分钟。中途关掉应用也安全——
    // 「cover_url 有值但 cover_local_path 为空」本身就是待办队列，下次启动会接着缓存。
    cover_cache::spawn_cover_cache(&app);

    Ok(result)
}

/// 封面缓存队列状态：还有多少张没缓存、后台任务是否在跑。
/// 前端用它决定要不要提示用户「还有 N 张封面待缓存」。
#[tauri::command]
pub async fn cover_cache_status(state: State<'_, AppState>) -> Result<CoverCacheStatus, String> {
    let pending = db::count_pending_cover_cache(&state.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(CoverCacheStatus {
        pending,
        running: state.cover_cache_busy.load(Ordering::SeqCst),
    })
}

/// 维护操作：手动把缺本地封面的 B站 / CSDN 项重新下载缓存。
///
/// 与后台任务共用同一套并发下载逻辑（早期这里是逐条串行，3000 条要十几分钟）。
/// 后台任务正在跑时不重复启动，直接返回提示。
#[tauri::command]
pub async fn recache_covers(app: tauri::AppHandle) -> Result<RecacheResult, String> {
    if app
        .state::<AppState>()
        .cover_cache_busy
        .load(Ordering::SeqCst)
    {
        return Ok(RecacheResult {
            total: 0,
            cached: 0,
            failed: 0,
            errors: vec!["封面缓存任务正在后台运行中，请稍后再试".to_string()],
        });
    }

    let progress = cover_cache::run_pass(&app)
        .await
        .map_err(|e| e.to_string())?;

    Ok(RecacheResult {
        total: progress.total,
        cached: progress.cached,
        failed: progress.failed,
        errors: Vec::new(),
    })
}

// ── 速记浮窗（P1） ──────────────────────────────────────────────────────────
//
// 定位：**随手记**，不是第二个收藏入口。红心的歌由 P2 同步自动带进库，
// 这个面板负责同步做不到的两件事 —— 即时，以及批注 / 时间戳。
//
// 形态是**按需创建 / 用完销毁**：不常驻、不 hide/show、不轮询（见 nowplaying.rs 顶部）。

/// 当前播放状态（读网易云窗口标题）。`track` 为 `null` 时看 `hint`：
/// 有 hint = 客户端在跑但读不到曲目（迷你模式等）；没有 = 压根没启动。
#[tauri::command]
pub async fn now_playing_current() -> nowplaying::NowPlayingState {
    // 读窗口是同步 Win32 调用，挪出 tokio 工作线程
    tokio::task::spawn_blocking(nowplaying::current_state)
        .await
        .ok()
        .unwrap_or(nowplaying::NowPlayingState {
            track: None,
            hint: None,
        })
}

/// 把当前曲目反查成完整条目（真实 song id + 封面 + 时长）。
///
/// 反查失败**不报错**：`resolved = false` 时前端仍可用 `title` / `artist` 记批注，
/// 只是拿不到真实 id（最终会用合成 id 落库）。
/// 原则：**绝不让「认不出歌」挡住用户写字**。
#[tauri::command]
pub async fn now_playing_resolve(
    state: State<'_, AppState>,
    title: String,
    artist: String,
) -> Result<TrackResolveResult, String> {
    let item = state
        .netease
        .resolve_track(&title, &artist, None)
        .await
        .map_err(|e| e.to_string())?;

    let (resolved, song_id, cover_url, duration) = match &item {
        Some(it) => (
            true,
            Some(it.external_id.clone()),
            it.cover_url.clone(),
            it.duration,
        ),
        None => (false, None, None, None),
    };

    let existing = match &song_id {
        Some(id) => db::find_item_id(&state.pool, "netease", id)
            .await
            .map_err(|e| e.to_string())?,
        None => None,
    };

    Ok(TrackResolveResult {
        resolved,
        song_id,
        title,
        artist,
        cover_url,
        duration,
        in_library: existing.is_some(),
        item_id: existing,
    })
}

/// 速记面板一键入库：建条目 + 打标签 + 写批注。
#[tauri::command]
pub async fn now_playing_capture(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    input: QuickCaptureRequest,
) -> Result<QuickCaptureResult, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let unresolved = input.song_id.is_none();

    // 反查失败也要能记：用「曲名 + 歌手」合成稳定 id，
    // 保证同一首歌反复记不会堆出一堆重复条目。**批注比条目整洁重要**。
    let external_id = match &input.song_id {
        Some(id) => id.clone(),
        None => format!(
            "np:{}|{}",
            input.title.trim().to_lowercase(),
            input.artist.trim().to_lowercase()
        ),
    };
    let source_url = match &input.song_id {
        Some(id) => format!("https://music.163.com/#/song?id={id}"),
        None => "https://music.163.com".to_string(),
    };

    let item = crate::models::ExternalItem {
        source: "netease".to_string(),
        external_id,
        source_url,
        title: input.title.clone(),
        description: input.artist.clone(),
        cover_url: input.cover_url.clone(),
        cover_local_path: None,
        author_name: Some(input.artist.clone()),
        author_id: None,
        partition_name: None,
        published_at: None,
        duration: input.duration,
        favorite_time: Some(now),
        extra: serde_json::json!({
            "kind": "song",
            "capturedBy": "float",
            "unresolved": unresolved,
        }),
    };

    let (item_id, created) = db::upsert_item(&state.pool, &item)
        .await
        .map_err(|e| e.to_string())?;

    // 标签：前端只传名称，这里补成「空 namespace + 名称」
    let specs: Vec<TagInput> = input
        .tags
        .iter()
        .filter(|t| !t.trim().is_empty())
        .map(|t| TagInput {
            id: None,
            namespace: String::new(),
            name: t.trim().to_string(),
            color: None,
            description: None,
            category_id: None,
        })
        .collect();
    if !specs.is_empty() {
        db::replace_item_tags(&state.pool, item_id, &specs)
            .await
            .map_err(|e| e.to_string())?;
    }

    // 批注：走 notes::save_notes —— 它是唯一写入入口，Obsidian 联动自动生效
    if !input.note.trim().is_empty() {
        crate::notes::save_notes(&state, item_id, &input.note)
            .await
            .map_err(|e| e.to_string())?;
    }

    // 封面照旧交给后台队列，不等它
    cover_cache::spawn_cover_cache(&app);

    Ok(QuickCaptureResult {
        item_id,
        created,
        unresolved,
    })
}

/// 浮窗功能是否启用（影响全局快捷键是否注册）。
#[tauri::command]
pub fn nowplaying_enabled(state: State<'_, AppState>) -> bool {
    nowplaying::load_enabled(&state.data_dir)
}

#[tauri::command]
pub fn nowplaying_set_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    nowplaying::set_enabled(&state.data_dir, enabled).map_err(|e| e.to_string())
}

/// 打开速记面板。**由前端调用（设置页的「打开」按钮），Rust 侧建窗口，
/// 因此不需要任何 window 权限** —— 权限只拦前端直接调窗口 API。
/// ⚠️ **这个命令必须是 `async`，不能改成同步。**
///
/// 同步命令跑在主线程，而 `WebviewWindowBuilder::build()` 要等 WebView 初始化完成 —— 它依赖主线程
/// 继续泵消息。在主线程里等它 = 自锁，表现为**新窗口白屏且无响应**（连关闭按钮都点不动）。
/// 官方维护者在 tauri-apps/tauri#13963 明确要求：
/// “you'll need to use `async` command for it to work or the app (rust side) will dead lock”。
#[tauri::command]
pub async fn nowplaying_open(app: tauri::AppHandle) -> Result<(), String> {
    nowplaying::open_window(&app).map_err(|e| e.to_string())
}

/// 关闭速记面板 —— **销毁而非隐藏**（本形态的核心，见 nowplaying.rs）。
#[tauri::command]
pub async fn nowplaying_close(app: tauri::AppHandle) {
    nowplaying::close_window(&app);
}

/// 当前生效的快捷键，设置页展示用。
#[tauri::command]
pub fn nowplaying_hotkey(state: State<'_, AppState>) -> String {
    nowplaying::load_hotkey(&state.data_dir)
}

/// 网易云是否正在出声（暂停 / 停止 = false）。面板的计时器只在 true 时走。
/// ⚠️ 必须是 `async`：WASAPI 的 COM 调用是阻塞的，放 blocking 线程池，别占 tokio worker。
#[tauri::command]
pub async fn nowplaying_is_playing() -> bool {
    tauri::async_runtime::spawn_blocking(nowplaying::netease_is_playing)
        .await
        .unwrap_or(false)
}

// now_playing_progress 命令已移除（手动时间轴，时间戳不再走后端进度快照）。
