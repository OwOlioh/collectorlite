use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqlitePool};

use crate::error::AppError;
use crate::models::{
    CollectionExport, CollectionInfo, ExportItem, ExportTag, ExternalItem, ImportResult,
    ItemFilters, Tag, TagCategory, TagInput, VideoItem,
};

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn normalize_tag(name: &str) -> String {
    name.trim().to_lowercase()
}

fn tag_color(name: &str, requested: Option<String>) -> Option<String> {
    let normalized = normalize_tag(name);
    if normalized.contains("up主") {
        return Some("#3b82f6".into());
    }
    if requested.is_some() {
        return requested;
    }
    const COLORS: [&str; 7] = [
        "#ef4444", "#f97316", "#10b981", "#8b5cf6", "#0891b2", "#db2777", "#ca8a04",
    ];
    let hash = normalized
        .bytes()
        .fold(0u32, |acc, byte| acc.wrapping_add(byte as u32));
    Some(COLORS[(hash as usize) % COLORS.len()].into())
}

pub async fn connect(path: &std::path::Path) -> Result<SqlitePool, AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let options = SqliteConnectOptions::from_str(path.to_string_lossy().as_ref())?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|error| AppError::Other(error.to_string()))?;
    Ok(pool)
}

#[derive(Debug, Clone, FromRow)]
struct ItemRow {
    id: i64,
    source: String,
    external_id: String,
    source_url: String,
    title: String,
    description: String,
    notes: String,
    cover_url: Option<String>,
    cover_local_path: Option<String>,
    author_name: Option<String>,
    author_id: Option<String>,
    partition_name: Option<String>,
    published_at: Option<i64>,
    duration: Option<i64>,
    favorite_time: Option<i64>,
    deleted_at: Option<i64>,
}

impl ItemRow {
    fn to_item(&self, tags: Vec<Tag>) -> VideoItem {
        VideoItem {
            id: self.id,
            source: self.source.clone(),
            external_id: self.external_id.clone(),
            source_url: self.source_url.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            notes: self.notes.clone(),
            cover_url: self.cover_url.clone(),
            cover_local_path: self.cover_local_path.clone(),
            author_name: self.author_name.clone(),
            author_id: self.author_id.clone(),
            partition_name: self.partition_name.clone(),
            published_at: self.published_at,
            duration: self.duration,
            favorite_time: self.favorite_time,
            deleted_at: self.deleted_at,
            tags,
        }
    }
}

pub async fn upsert_item(pool: &SqlitePool, item: &ExternalItem) -> Result<(i64, bool), AppError> {
    let now = now_seconds();
    let existing =
        sqlx::query_scalar::<_, i64>("SELECT id FROM items WHERE source = ? AND external_id = ?")
            .bind(&item.source)
            .bind(&item.external_id)
            .fetch_optional(pool)
            .await?;

    if let Some(id) = existing {
        sqlx::query(
            "UPDATE items SET
                source_url = ?, title = ?, description = ?, cover_url = ?, cover_local_path = ?, author_name = ?,
                author_id = ?, partition_name = ?, published_at = ?, duration = ?,
                favorite_time = ?, extra_json = ?, deleted_at = NULL, updated_at = ?
             WHERE id = ?",
        )
        .bind(&item.source_url)
        .bind(&item.title)
        .bind(&item.description)
        .bind(&item.cover_url)
        .bind(&item.cover_local_path)
        .bind(&item.author_name)
        .bind(&item.author_id)
        .bind(&item.partition_name)
        .bind(item.published_at)
        .bind(item.duration)
        .bind(item.favorite_time)
        .bind(serde_json::to_string(&item.extra)?)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
        update_fts_row(pool, id, item).await?;
        Ok((id, false))
    } else {
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO items (
                source, external_id, source_url, title, description, cover_url, cover_local_path, author_name,
                author_id, partition_name, published_at, duration, favorite_time, extra_json,
                created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(&item.source)
        .bind(&item.external_id)
        .bind(&item.source_url)
        .bind(&item.title)
        .bind(&item.description)
        .bind(&item.cover_url)
        .bind(&item.cover_local_path)
        .bind(&item.author_name)
        .bind(&item.author_id)
        .bind(&item.partition_name)
        .bind(item.published_at)
        .bind(item.duration)
        .bind(item.favorite_time)
        .bind(serde_json::to_string(&item.extra)?)
        .bind(now)
        .bind(now)
        .fetch_one(pool)
        .await?;
        update_fts_row(pool, id, item).await?;
        Ok((id, true))
    }
}

async fn update_fts_row(
    pool: &SqlitePool,
    item_id: i64,
    item: &ExternalItem,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM items_fts WHERE rowid = ?")
        .bind(item_id)
        .execute(pool)
        .await?;
    let tags = item
        .extra
        .get("tag_names")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    sqlx::query(
        "INSERT INTO items_fts (rowid, title, description, author_name, partition_name, tags)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(item_id)
    .bind(&item.title)
    .bind(&item.description)
    .bind(&item.author_name)
    .bind(&item.partition_name)
    .bind(tags)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn rebuild_item_fts(pool: &SqlitePool, item_id: i64) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, ItemRow>(
        "SELECT id, source, external_id, source_url, title, description, notes, cover_url, cover_local_path,
                author_name, author_id, partition_name, published_at, duration, favorite_time, deleted_at
         FROM items WHERE id = ?",
    )
    .bind(item_id)
    .fetch_one(pool)
    .await?;
    let tag_names = sqlx::query_scalar::<_, String>(
        "SELECT name FROM tags t JOIN item_tags it ON it.tag_id = t.id WHERE it.item_id = ?",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    let item = ExternalItem {
        source: row.source,
        external_id: row.external_id,
        source_url: row.source_url,
        title: row.title,
        description: row.description,
        cover_url: row.cover_url,
        cover_local_path: row.cover_local_path,
        author_name: row.author_name,
        author_id: row.author_id,
        partition_name: row.partition_name,
        published_at: row.published_at,
        duration: row.duration,
        favorite_time: row.favorite_time,
        extra: serde_json::json!({ "tag_names": tag_names }),
    };
    update_fts_row(pool, item_id, &item).await
}

pub async fn get_or_create_tag(pool: &SqlitePool, input: &TagInput) -> Result<i64, AppError> {
    let normalized = normalize_tag(&input.name);
    if normalized.is_empty() {
        return Err(AppError::InvalidInput("标签名称不能为空".into()));
    }
    let color = tag_color(&input.name, input.color.clone());
    if let Some(id) = input.id {
        let exists = sqlx::query_scalar::<_, i64>("SELECT id FROM tags WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
        if exists.is_some() {
            sqlx::query(
                "UPDATE tags SET name = ?, normalized = ?, color = ?, description = ?, category_id = ?
                 WHERE id = ?",
            )
            .bind(input.name.trim())
            .bind(&normalized)
            .bind(&color)
            .bind(&input.description)
            .bind(input.category_id)
            .bind(id)
            .execute(pool)
            .await?;
            return Ok(id);
        }
    }

    if let Some(id) =
        sqlx::query_scalar::<_, i64>("SELECT id FROM tags WHERE normalized = ? ORDER BY id LIMIT 1")
            .bind(&normalized)
            .fetch_optional(pool)
            .await?
    {
        return Ok(id);
    }

    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO tags (namespace, name, normalized, color, description, category_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(input.namespace.as_str())
    .bind(input.name.trim())
    .bind(&normalized)
    .bind(&color)
    .bind(&input.description)
    .bind(input.category_id)
    .bind(now_seconds())
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn attach_tag(pool: &SqlitePool, item_id: i64, tag_id: i64) -> Result<(), AppError> {
    sqlx::query("INSERT OR IGNORE INTO item_tags (item_id, tag_id, created_at) VALUES (?, ?, ?)")
        .bind(item_id)
        .bind(tag_id)
        .bind(now_seconds())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_tags(pool: &SqlitePool) -> Result<Vec<Tag>, AppError> {
    let rows = sqlx::query(
        "SELECT t.id, t.namespace, t.name, t.normalized, t.color, t.description, t.category_id,
                COUNT(it.item_id) AS count
         FROM tags t
         LEFT JOIN item_tags it ON it.tag_id = t.id
         GROUP BY t.id
         ORDER BY count DESC, t.name COLLATE NOCASE",
    )
    .map(tag_from_row)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn tag_from_row(row: SqliteRow) -> Tag {
    Tag {
        id: row.get("id"),
        namespace: row.get("namespace"),
        name: row.get("name"),
        normalized: row.get("normalized"),
        color: row.get("color"),
        description: row.get("description"),
        count: row.get("count"),
        category_id: row.get("category_id"),
    }
}

pub async fn upsert_tag(pool: &SqlitePool, input: &TagInput) -> Result<Tag, AppError> {
    let id = get_or_create_tag(pool, input).await?;
    let tag = sqlx::query(
        "SELECT t.id, t.namespace, t.name, t.normalized, t.color, t.description, t.category_id,
                COUNT(it.item_id) AS count
         FROM tags t
         LEFT JOIN item_tags it ON it.tag_id = t.id
         WHERE t.id = ?
         GROUP BY t.id",
    )
    .bind(id)
    .map(tag_from_row)
    .fetch_one(pool)
    .await?;
    Ok(tag)
}

pub async fn replace_item_tags(
    pool: &SqlitePool,
    item_id: i64,
    tag_specs: &[TagInput],
) -> Result<VideoItem, AppError> {
    sqlx::query("DELETE FROM item_tags WHERE item_id = ?")
        .bind(item_id)
        .execute(pool)
        .await?;
    for spec in tag_specs {
        let tag_id = get_or_create_tag(pool, spec).await?;
        attach_tag(pool, item_id, tag_id).await?;
    }
    rebuild_item_fts(pool, item_id).await?;
    let row = sqlx::query_as::<_, ItemRow>(
        "SELECT id, source, external_id, source_url, title, description, notes, cover_url, cover_local_path,
                author_name, author_id, partition_name, published_at, duration, favorite_time, deleted_at
         FROM items WHERE id = ?",
    )
    .bind(item_id)
    .fetch_one(pool)
    .await?;
    let tags = sqlx::query(
        "SELECT t.id, t.namespace, t.name, t.normalized, t.color, t.description, t.category_id,
                COUNT(it2.item_id) AS count
         FROM tags t
         JOIN item_tags it ON it.tag_id = t.id
         LEFT JOIN item_tags it2 ON it2.tag_id = t.id
         WHERE it.item_id = ?
         GROUP BY t.id
         ORDER BY t.name COLLATE NOCASE",
    )
    .bind(item_id)
    .map(tag_from_row)
    .fetch_all(pool)
    .await?;
    Ok(row.to_item(tags))
}

pub async fn update_item_notes(
    pool: &SqlitePool,
    item_id: i64,
    notes: &str,
) -> Result<VideoItem, AppError> {
    sqlx::query("UPDATE items SET notes = ?, updated_at = ? WHERE id = ?")
        .bind(notes)
        .bind(now_seconds())
        .bind(item_id)
        .execute(pool)
        .await?;
    let row = sqlx::query_as::<_, ItemRow>(
        "SELECT id, source, external_id, source_url, title, description, notes, cover_url, cover_local_path,
                author_name, author_id, partition_name, published_at, duration, favorite_time, deleted_at
         FROM items WHERE id = ?",
    )
    .bind(item_id)
    .fetch_one(pool)
    .await?;
    let tags = sqlx::query(
        "SELECT t.id, t.namespace, t.name, t.normalized, t.color, t.description, t.category_id,
                COUNT(it2.item_id) AS count
         FROM tags t
         JOIN item_tags it ON it.tag_id = t.id
         LEFT JOIN item_tags it2 ON it2.tag_id = t.id
         WHERE it.item_id = ?
         GROUP BY t.id
         ORDER BY t.name COLLATE NOCASE",
    )
    .bind(item_id)
    .map(tag_from_row)
    .fetch_all(pool)
    .await?;
    Ok(row.to_item(tags))
}

pub async fn merge_tags(
    pool: &SqlitePool,
    source_tag_id: i64,
    target_tag_id: i64,
) -> Result<(), AppError> {
    if source_tag_id == target_tag_id {
        return Err(AppError::InvalidInput("不能合并到同一个标签".into()));
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO item_tags (item_id, tag_id, created_at)
         SELECT item_id, ?, ? FROM item_tags WHERE tag_id = ?",
    )
    .bind(target_tag_id)
    .bind(now_seconds())
    .bind(source_tag_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM item_tags WHERE tag_id = ?")
        .bind(source_tag_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(source_tag_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_tag(pool: &SqlitePool, tag_id: i64) -> Result<(), AppError> {
    sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(tag_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_tag_categories(pool: &SqlitePool) -> Result<Vec<TagCategory>, AppError> {
    let rows = sqlx::query(
        "SELECT id, name, normalized, color, position
         FROM tag_categories
         ORDER BY position, name COLLATE NOCASE",
    )
    .map(category_from_row)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn category_from_row(row: SqliteRow) -> TagCategory {
    TagCategory {
        id: row.get("id"),
        name: row.get("name"),
        normalized: row.get("normalized"),
        color: row.get("color"),
        position: row.get("position"),
    }
}

pub async fn create_tag_category(
    pool: &SqlitePool,
    name: &str,
    color: Option<String>,
) -> Result<TagCategory, AppError> {
    let normalized = normalize_tag(name);
    if normalized.is_empty() {
        return Err(AppError::InvalidInput("分类名称不能为空".into()));
    }
    // 同名（忽略大小写/首尾空格）已存在时直接返回已有分类，避免 UNIQUE 冲突报错，
    // 这样前端“新建分类”永远能得到该分类并立即显示。
    if let Some(existing) = get_tag_category_by_normalized(pool, &normalized).await? {
        return Ok(existing);
    }
    // 一条语句完成插入并 RETURNING 完整行，避免「INSERT 后另起连接二次 SELECT」
    // 在 WAL 模式下读不到刚提交行、触发 RowNotFound（"no rows returned"）的竞态。
    let row = sqlx::query(
        "INSERT INTO tag_categories (name, normalized, color, position, created_at)
         VALUES (?, ?, ?, ?, ?)
         RETURNING id, name, normalized, color, position",
    )
    .bind(name.trim())
    .bind(&normalized)
    .bind(&color)
    .bind(0)
    .bind(now_seconds())
    .map(category_from_row)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn rename_tag_category(
    pool: &SqlitePool,
    id: i64,
    name: &str,
    color: Option<String>,
) -> Result<TagCategory, AppError> {
    let normalized = normalize_tag(name);
    // UPDATE ... RETURNING 一步返回完整行，避免 UPDATE 后二次 SELECT 的 WAL 竞态。
    let row = sqlx::query(
        "UPDATE tag_categories SET name = ?, normalized = ?, color = ? WHERE id = ?
         RETURNING id, name, normalized, color, position",
    )
    .bind(name.trim())
    .bind(&normalized)
    .bind(&color)
    .bind(id)
    .map(category_from_row)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete_tag_category(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE tags SET category_id = NULL WHERE category_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM tag_categories WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn assign_tag_category(
    pool: &SqlitePool,
    tag_id: i64,
    category_id: Option<i64>,
) -> Result<Tag, AppError> {
    sqlx::query("UPDATE tags SET category_id = ? WHERE id = ?")
        .bind(category_id)
        .bind(tag_id)
        .execute(pool)
        .await?;
    let tag = sqlx::query(
        "SELECT t.id, t.namespace, t.name, t.normalized, t.color, t.description, t.category_id,
                COUNT(it.item_id) AS count
         FROM tags t
         LEFT JOIN item_tags it ON it.tag_id = t.id
         WHERE t.id = ?
         GROUP BY t.id",
    )
    .bind(tag_id)
    .map(tag_from_row)
    .fetch_one(pool)
    .await?;
    Ok(tag)
}

// ── 回收站（软删除） ──
// 软删除只置 deleted_at 并移除 FTS 行，保留 item_tags / import_run_items 关联与封面文件，
// 以便恢复时零成本还原、且不丢失标签与导入来源。

pub async fn soft_delete_item(pool: &SqlitePool, item_id: i64) -> Result<(), AppError> {
    let now = now_seconds();
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE items SET deleted_at = ?, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(now)
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM items_fts WHERE rowid = ?")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn soft_delete_items(pool: &SqlitePool, item_ids: &[i64]) -> Result<(), AppError> {
    for item_id in item_ids {
        soft_delete_item(pool, *item_id).await?;
    }
    Ok(())
}

pub async fn soft_delete_items_by_tag(pool: &SqlitePool, tag_id: i64) -> Result<usize, AppError> {
    let item_ids = sqlx::query_scalar::<_, i64>("SELECT item_id FROM item_tags WHERE tag_id = ?")
        .bind(tag_id)
        .fetch_all(pool)
        .await?;
    let count = item_ids.len();
    soft_delete_items(pool, &item_ids).await?;
    Ok(count)
}

pub async fn restore_item(pool: &SqlitePool, item_id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE items SET deleted_at = NULL, updated_at = ? WHERE id = ?")
        .bind(now_seconds())
        .bind(item_id)
        .execute(pool)
        .await?;
    rebuild_item_fts(pool, item_id).await?;
    Ok(())
}

pub async fn restore_items(pool: &SqlitePool, item_ids: &[i64]) -> Result<(), AppError> {
    for item_id in item_ids {
        restore_item(pool, *item_id).await?;
    }
    Ok(())
}

/// 永久删除单条（真正删库行 + FTS 行），返回封面本地路径供调用方删文件。
pub async fn purge_item(pool: &SqlitePool, item_id: i64) -> Result<Option<String>, AppError> {
    let cover_path = sqlx::query_scalar::<_, Option<String>>(
        "SELECT cover_local_path FROM items WHERE id = ?",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM items_fts WHERE rowid = ?")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM items WHERE id = ?")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(cover_path)
}

pub async fn purge_items(pool: &SqlitePool, item_ids: &[i64]) -> Result<Vec<String>, AppError> {
    let mut cover_paths = Vec::with_capacity(item_ids.len());
    for item_id in item_ids {
        if let Some(path) = purge_item(pool, *item_id).await? {
            cover_paths.push(path);
        }
    }
    Ok(cover_paths)
}

pub async fn empty_trash(pool: &SqlitePool) -> Result<Vec<String>, AppError> {
    let item_ids =
        sqlx::query_scalar::<_, i64>("SELECT id FROM items WHERE deleted_at IS NOT NULL")
            .fetch_all(pool)
            .await?;
    purge_items(pool, &item_ids).await
}

pub async fn list_trash(pool: &SqlitePool) -> Result<Vec<VideoItem>, AppError> {
    let rows = sqlx::query_as::<_, ItemRow>(
        "SELECT id, source, external_id, source_url, title, description, notes, cover_url, cover_local_path,
                author_name, author_id, partition_name, published_at, duration, favorite_time, deleted_at
         FROM items WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC",
    )
    .fetch_all(pool)
    .await?;
    hydrate_items(pool, rows).await
}

pub async fn get_trash_count(pool: &SqlitePool) -> Result<i64, AppError> {
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM items WHERE deleted_at IS NOT NULL")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

/// 清理超过保留期的回收站条目，返回被删封面路径（由调用方删文件）。
pub async fn auto_purge_expired(
    pool: &SqlitePool,
    retention_days: i64,
) -> Result<Vec<String>, AppError> {
    let threshold = now_seconds() - retention_days * 86400;
    let item_ids = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM items WHERE deleted_at IS NOT NULL AND deleted_at < ?",
    )
    .bind(threshold)
    .fetch_all(pool)
    .await?;
    purge_items(pool, &item_ids).await
}

async fn get_tag_category_by_normalized(
    pool: &SqlitePool,
    normalized: &str,
) -> Result<Option<TagCategory>, AppError> {
    let row = sqlx::query(
        "SELECT id, name, normalized, color, position FROM tag_categories WHERE normalized = ?",
    )
    .bind(normalized)
    .map(category_from_row)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn create_import_run(
    pool: &SqlitePool,
    collection: &CollectionInfo,
    total: i64,
    cleanup_eligible: bool,
) -> Result<i64, AppError> {
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO import_runs (
            source, collection_id, collection_title, collection_url, status, total,
            cleanup_eligible, started_at
         ) VALUES (?, ?, ?, ?, 'running', ?, ?, ?)
         RETURNING id",
    )
    .bind(&collection.source)
    .bind(&collection.id)
    .bind(&collection.title)
    .bind(&collection.url)
    .bind(total)
    .bind(if cleanup_eligible { 1i64 } else { 0i64 })
    .bind(now_seconds())
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn finish_import_run(
    pool: &SqlitePool,
    run_id: i64,
    imported: i64,
    skipped: i64,
    failed: i64,
    errors: &[String],
) -> Result<(), AppError> {
    let error_json = serde_json::to_string(errors)?;
    sqlx::query(
        "UPDATE import_runs
         SET status = 'done', imported = ?, skipped = ?, failed = ?,
             error_json = ?, finished_at = ?
         WHERE id = ?",
    )
    .bind(imported)
    .bind(skipped)
    .bind(failed)
    .bind(error_json)
    .bind(now_seconds())
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn link_import_item(
    pool: &SqlitePool,
    run_id: i64,
    item_id: i64,
) -> Result<(), AppError> {
    sqlx::query("INSERT OR IGNORE INTO import_run_items (run_id, item_id) VALUES (?, ?)")
        .bind(run_id)
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn search_items(
    pool: &SqlitePool,
    filters: &ItemFilters,
) -> Result<Vec<VideoItem>, AppError> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, source, external_id, source_url, title, description, notes, cover_url, cover_local_path,
                author_name, author_id, partition_name, published_at, duration, favorite_time, deleted_at
         FROM items i WHERE 1 = 1",
    );

    // 回收站过滤：默认（trash 为 None/false）仅显示正常在库项；trash=Some(true) 时只看回收站。
    match &filters.trash {
        Some(true) => {
            query.push(" AND i.deleted_at IS NOT NULL");
        }
        _ => {
            query.push(" AND i.deleted_at IS NULL");
        }
    }

    if !filters.tag_ids.is_empty() {
        if filters.tag_mode == "or" {
            query.push(" AND i.id IN (SELECT item_id FROM item_tags WHERE tag_id IN (");
            let mut separated = query.separated(", ");
            for tag_id in &filters.tag_ids {
                separated.push_bind(tag_id);
            }
            query.push("))");
        } else {
            for tag_id in &filters.tag_ids {
                query
                    .push(" AND i.id IN (SELECT item_id FROM item_tags WHERE tag_id = ")
                    .push_bind(tag_id)
                    .push(")");
            }
        }
    }

    if !filters.sources.is_empty() {
        query.push(" AND i.source IN (");
        let mut separated = query.separated(", ");
        for source in &filters.sources {
            separated.push_bind(source);
        }
        query.push(")");
    }

    if let Some(raw_query) = filters.query.as_deref().map(str::trim) {
        if !raw_query.is_empty() {
            let pattern = format!("%{raw_query}%");
            query
                .push(" AND (i.title LIKE ")
                .push_bind(pattern.clone())
                .push(" OR i.description LIKE ")
                .push_bind(pattern.clone())
                .push(" OR COALESCE(i.author_name, '') LIKE ")
                .push_bind(pattern.clone())
                .push(" OR COALESCE(i.partition_name, '') LIKE ")
                .push_bind(pattern.clone())
                .push(" OR EXISTS (SELECT 1 FROM item_tags it JOIN tags t ON t.id = it.tag_id WHERE it.item_id = i.id AND t.name LIKE ")
                .push_bind(pattern)
                .push("))");
        }
    }

    let sort_sql = match filters.sort.as_str() {
        "published_desc" => "i.published_at DESC",
        "duration_desc" => "i.duration DESC",
        "title_asc" => "i.title COLLATE NOCASE ASC",
        "imported_desc" => "i.id DESC",
        _ => "i.favorite_time DESC",
    };
    query.push(" ORDER BY ").push(sort_sql).push(" LIMIT 1000");

    let rows = query.build_query_as::<ItemRow>().fetch_all(pool).await?;
    hydrate_items(pool, rows).await
}

async fn hydrate_items(pool: &SqlitePool, rows: Vec<ItemRow>) -> Result<Vec<VideoItem>, AppError> {
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let tags = sqlx::query(
            "SELECT t.id, t.namespace, t.name, t.normalized, t.color, t.description, t.category_id,
                    COUNT(it2.item_id) AS count
             FROM tags t
             JOIN item_tags it ON it.tag_id = t.id
             LEFT JOIN item_tags it2 ON it2.tag_id = t.id
             WHERE it.item_id = ?
             GROUP BY t.id
             ORDER BY t.name COLLATE NOCASE",
        )
        .bind(row.id)
        .map(tag_from_row)
        .fetch_all(pool)
        .await?;
        items.push(row.to_item(tags));
    }
    Ok(items)
}

pub async fn build_import_result(pool: &SqlitePool, run_id: i64) -> Result<ImportResult, AppError> {
    let row = sqlx::query(
        "SELECT total, imported, skipped, failed, cleanup_status, error_json
         FROM import_runs WHERE id = ?",
    )
    .bind(run_id)
    .map(|row: SqliteRow| ImportResult {
        run_id,
        total: row.get("total"),
        imported: row.get("imported"),
        skipped: row.get("skipped"),
        failed: row.get("failed"),
        cleanup_status: row.get("cleanup_status"),
        errors: row
            .get::<Option<String>, _>("error_json")
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default(),
    })
    .fetch_one(pool)
    .await?;
    Ok(row)
}

// ── 收藏库导入 / 导出 ──

#[derive(Debug, Clone, FromRow)]
struct ExportRow {
    id: i64,
    source: String,
    external_id: String,
    source_url: String,
    title: String,
    description: String,
    notes: String,
    cover_url: Option<String>,
    author_name: Option<String>,
    author_id: Option<String>,
    partition_name: Option<String>,
    published_at: Option<i64>,
    duration: Option<i64>,
    favorite_time: Option<i64>,
    extra_json: String,
}

/// 把收藏库导出为 `CollectionExport`。`item_ids` 为 None 时导出全部，为空数组时不导出任何项。
pub async fn export_items(
    pool: &SqlitePool,
    item_ids: Option<Vec<i64>>,
) -> Result<CollectionExport, AppError> {
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT id, source, external_id, source_url, title, description, notes, cover_url,
                author_name, author_id, partition_name, published_at, duration, favorite_time, extra_json
         FROM items WHERE 1 = 1 AND deleted_at IS NULL",
    );
    match &item_ids {
        Some(ids) if !ids.is_empty() => {
            qb.push(" AND id IN (");
            let mut separated = qb.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            qb.push(")");
        }
        Some(_) => {
            // 空选择 -> 不导出任何项
            qb.push(" AND 0 = 1");
        }
        None => {}
    }
    let rows = qb.build_query_as::<ExportRow>().fetch_all(pool).await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let tags = sqlx::query(
            "SELECT t.namespace, t.name, t.color, tc.name AS category_name
             FROM tags t
             JOIN item_tags it ON it.tag_id = t.id
             LEFT JOIN tag_categories tc ON tc.id = t.category_id
             WHERE it.item_id = ?
             ORDER BY t.name COLLATE NOCASE",
        )
        .bind(row.id)
        .map(|r: SqliteRow| ExportTag {
            namespace: r.get("namespace"),
            name: r.get("name"),
            color: r.get("color"),
            category: r.get("category_name"),
        })
        .fetch_all(pool)
        .await?;

        let extra: serde_json::Value =
            serde_json::from_str(&row.extra_json).unwrap_or(serde_json::Value::Null);

        items.push(ExportItem {
            source: row.source,
            external_id: row.external_id,
            source_url: row.source_url,
            title: row.title,
            description: row.description,
            cover_url: row.cover_url,
            author_name: row.author_name,
            author_id: row.author_id,
            partition_name: row.partition_name,
            published_at: row.published_at,
            duration: row.duration,
            favorite_time: row.favorite_time,
            notes: row.notes,
            extra,
            tags,
        });
    }

    Ok(CollectionExport {
        format_version: 1,
        exported_at: now_seconds(),
        app: "bilibili_collector".into(),
        items,
    })
}

/// 把已导入项的封面本地路径写回数据库（文件导入后补缓存用）。
pub async fn set_item_cover_local_path(
    pool: &SqlitePool,
    source: &str,
    external_id: &str,
    path: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE items SET cover_local_path = ?, updated_at = ? WHERE source = ? AND external_id = ?",
    )
    .bind(path)
    .bind(now_seconds())
        .bind(source)
        .bind(external_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 取出需要补缓存封面的项：bilibili / csdn 来源、有远程 cover_url、但本地缓存为空的项。
pub async fn fetch_items_needing_cover_cache(
    pool: &SqlitePool,
) -> Result<Vec<ExternalItem>, AppError> {
    let rows = sqlx::query(
        "SELECT source, external_id, cover_url FROM items
         WHERE source IN ('bilibili', 'csdn')
           AND cover_url IS NOT NULL AND cover_url <> ''
           AND (cover_local_path IS NULL OR cover_local_path = '')",
    )
    .map(|r: SqliteRow| CoverNeedRow {
        source: r.get("source"),
        external_id: r.get("external_id"),
        cover_url: r.get("cover_url"),
    })
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| ExternalItem {
            source: row.source,
            external_id: row.external_id,
            source_url: String::new(),
            title: String::new(),
            description: String::new(),
            cover_url: Some(row.cover_url),
            cover_local_path: None,
            author_name: None,
            author_id: None,
            partition_name: None,
            published_at: None,
            duration: None,
            favorite_time: None,
            extra: serde_json::Value::Null,
        })
        .collect())
}

struct CoverNeedRow {
    source: String,
    external_id: String,
    cover_url: String,
}

/// 从导出文件增量导入：仅新增 `(source, external_id)` 不存在的项，已存在项跳过（不覆盖）。
/// 返回 `(导入结果, 新插入项清单)`，新项清单供调用方补下载封面缓存。
pub async fn import_collection(
    pool: &SqlitePool,
    payload: &str,
) -> Result<(ImportResult, Vec<ExternalItem>), AppError> {
    let export: CollectionExport = serde_json::from_str(payload)
        .map_err(|e| AppError::InvalidInput(format!("收藏文件格式无效：{e}")))?;

    // 版本兼容性守卫：当前支持 v1，拒绝更高版本以避免静默数据损坏
    if export.format_version > 1 {
        return Err(AppError::InvalidInput(format!(
            "文件格式版本 {} 高于当前支持的版本 1，请升级应用后再导入",
            export.format_version
        )));
    }

    let collection = CollectionInfo {
        source: "file".into(),
        id: "import".into(),
        title: format!("导入文件（{} 条）", export.items.len()),
        owner: None,
        count: export.items.len() as i64,
        url: None,
    };
    let run_id = create_import_run(pool, &collection, export.items.len() as i64, false).await?;

    let mut imported: i64 = 0;
    let mut skipped: i64 = 0;
    let mut failed: i64 = 0;
    let mut errors: Vec<String> = Vec::new();
    let mut new_items: Vec<ExternalItem> = Vec::new();

    for item in &export.items {
        if item.source.trim().is_empty() || item.external_id.trim().is_empty() {
            failed += 1;
            errors.push(format!("跳过无效项（缺少来源或 ID）：{}", item.title));
            continue;
        }

        // 增量模式：已存在则跳过，绝不覆盖原库
        let existing = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM items WHERE source = ? AND external_id = ?",
        )
        .bind(&item.source)
        .bind(&item.external_id)
        .fetch_optional(pool)
        .await?;
        if existing.is_some() {
            skipped += 1;
            continue;
        }

        let now = now_seconds();
        let extra_str = serde_json::to_string(&item.extra).unwrap_or_else(|_| "null".to_string());

        let insert_result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO items (
                source, external_id, source_url, title, description, cover_url, cover_local_path, author_name,
                author_id, partition_name, published_at, duration, favorite_time, notes, extra_json,
                created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(&item.source)
        .bind(&item.external_id)
        .bind(&item.source_url)
        .bind(&item.title)
        .bind(&item.description)
        .bind(&item.cover_url)
        .bind(&item.author_name)
        .bind(&item.author_id)
        .bind(&item.partition_name)
        .bind(item.published_at)
        .bind(item.duration)
        .bind(item.favorite_time)
        .bind(&item.notes)
        .bind(&extra_str)
        .bind(now)
        .bind(now)
        .fetch_one(pool)
        .await;

        let id = match insert_result {
            Ok(id) => id,
            Err(e) => {
                failed += 1;
                errors.push(format!("导入失败「{}」：{}", item.title, e));
                continue;
            }
        };

        // 标签：find-or-create，沿用库中已有同名标签的颜色，新标签才创建
        for tag in &item.tags {
            let mut input = TagInput {
                id: None,
                namespace: if tag.namespace.trim().is_empty() {
                    "manual".into()
                } else {
                    tag.namespace.clone()
                },
                name: tag.name.trim().to_string(),
                color: tag.color.clone(),
                description: None,
                category_id: None,
            };
            if let Some(cat_name) = &tag.category {
                if !cat_name.trim().is_empty() {
                    if let Ok(cat) = create_tag_category(pool, cat_name.trim(), None).await {
                        input.category_id = Some(cat.id);
                    }
                }
            }
            match get_or_create_tag(pool, &input).await {
                Ok(tag_id) => {
                    let _ = attach_tag(pool, id, tag_id).await;
                }
                Err(e) => {
                    errors.push(format!("标签「{}」创建失败：{}", tag.name, e));
                }
            }
        }

        // 重建全文索引，使标签可被检索
        let _ = rebuild_item_fts(pool, id).await;
        imported += 1;

        // 记录新插入项，供命令层下载封面缓存（复刻实时 B站导入行为）
        new_items.push(ExternalItem {
            source: item.source.clone(),
            external_id: item.external_id.clone(),
            source_url: item.source_url.clone(),
            title: item.title.clone(),
            description: item.description.clone(),
            cover_url: item.cover_url.clone(),
            cover_local_path: None,
            author_name: item.author_name.clone(),
            author_id: item.author_id.clone(),
            partition_name: item.partition_name.clone(),
            published_at: item.published_at,
            duration: item.duration,
            favorite_time: item.favorite_time,
            extra: item.extra.clone(),
        });
    }

    finish_import_run(pool, run_id, imported, skipped, failed, &errors).await?;
    let result = build_import_result(pool, run_id).await?;
    Ok((result, new_items))
}
