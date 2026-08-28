use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqlitePool};

use crate::error::AppError;
use crate::models::{
    CollectionInfo, ExternalItem, ImportResult, ItemFilters, Tag, TagCategory, TagInput, VideoItem,
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
                favorite_time = ?, extra_json = ?, updated_at = ?
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
                author_name, author_id, partition_name, published_at, duration, favorite_time
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
                author_name, author_id, partition_name, published_at, duration, favorite_time
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
                author_name, author_id, partition_name, published_at, duration, favorite_time
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
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO tag_categories (name, normalized, color, position, created_at)
         VALUES (?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(name.trim())
    .bind(&normalized)
    .bind(&color)
    .bind(0)
    .bind(now_seconds())
    .fetch_one(pool)
    .await?;
    get_tag_category(pool, id).await
}

pub async fn rename_tag_category(
    pool: &SqlitePool,
    id: i64,
    name: &str,
    color: Option<String>,
) -> Result<TagCategory, AppError> {
    let normalized = normalize_tag(name);
    sqlx::query("UPDATE tag_categories SET name = ?, normalized = ?, color = ? WHERE id = ?")
        .bind(name.trim())
        .bind(&normalized)
        .bind(&color)
        .bind(id)
        .execute(pool)
        .await?;
    get_tag_category(pool, id).await
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

pub async fn delete_item(pool: &SqlitePool, item_id: i64) -> Result<Option<String>, AppError> {
    let cover_path =
        sqlx::query_scalar::<_, Option<String>>("SELECT cover_local_path FROM items WHERE id = ?")
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

pub async fn delete_items(pool: &SqlitePool, item_ids: &[i64]) -> Result<Vec<String>, AppError> {
    let mut cover_paths = Vec::with_capacity(item_ids.len());
    for item_id in item_ids {
        if let Some(path) = delete_item(pool, *item_id).await? {
            cover_paths.push(path);
        }
    }
    Ok(cover_paths)
}

pub async fn delete_items_by_tag(pool: &SqlitePool, tag_id: i64) -> Result<Vec<String>, AppError> {
    let item_ids = sqlx::query_scalar::<_, i64>("SELECT item_id FROM item_tags WHERE tag_id = ?")
        .bind(tag_id)
        .fetch_all(pool)
        .await?;
    delete_items(pool, &item_ids).await
}

async fn get_tag_category(pool: &SqlitePool, id: i64) -> Result<TagCategory, AppError> {
    let row = sqlx::query(
        "SELECT id, name, normalized, color, position FROM tag_categories WHERE id = ?",
    )
    .bind(id)
    .map(category_from_row)
    .fetch_one(pool)
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
                author_name, author_id, partition_name, published_at, duration, favorite_time
         FROM items i WHERE 1 = 1",
    );

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
