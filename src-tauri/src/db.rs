use std::collections::HashMap;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha384};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions, SqliteRow,
};
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqlitePool};

use crate::error::AppError;
use crate::models::{
    CollectionExport, CollectionInfo, ExportItem, ExportTag, ExternalItem, ImportResult,
    ItemFilters, Tag, TagCategory, TagInput, VideoItem,
};

pub fn now_seconds() -> i64 {
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

/// 自愈 sqlx 迁移校验和的**行尾漂移**，返回修复的条数。
///
/// # 背景
/// `sqlx::migrate!` 在编译期把 `.sql` 按**字节** embed 进 exe，运行时对数据库
/// `_sqlx_migrations.checksum`（sha384）逐一比对。CRLF 与 LF 在 SQL 语义上完全等价，
/// 但字节不同 → sha384 不同 → sqlx 判定"迁移被篡改"并直接 panic
/// （`migration N was previously applied but has been modified`）。
///
/// 只要"建库时的文件行尾"与"当前 exe 内嵌的文件行尾"不一致就会触发，典型场景：
/// 老数据库 + 新 exe 的升级路径、跨平台 checkout（Windows `autocrlf`）、CI 与本地行尾策略不同。
///
/// # 安全边界（重要）
/// **只修"确认是纯行尾差异"的情况**：把当前 sql 分别归一化成全 LF / 全 CRLF 再算 sha384，
/// 只有命中其中之一才认定是行尾漂移并更新 checksum。若两者都不匹配，说明 SQL 内容真的被改了，
/// 此时**不做任何修改**，交由 sqlx 原有的校验照常 panic——防篡改语义完整保留。
///
/// # 失败策略
/// 全部软失败：表不存在（全新库）或查询异常都返回 0 并静默跳过，绝不阻断启动。
pub(crate) async fn heal_migration_line_endings(pool: &SqlitePool, migrator: &Migrator) -> usize {
    // 全新数据库还没有迁移表，这是正常情况
    let rows = match sqlx::query("SELECT version, checksum FROM _sqlx_migrations")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(_) => return 0,
    };

    let mut healed = 0usize;
    for row in rows {
        // 单行解析失败不应影响其余迁移的自愈
        let (Ok(version), Ok(applied)) = (
            row.try_get::<i64, _>("version"),
            row.try_get::<Vec<u8>, _>("checksum"),
        ) else {
            continue;
        };

        // 数据库记录了但代码里已不存在的迁移：交给 sqlx 自己报 VersionMissing
        let Some(migration) = migrator.migrations.iter().find(|m| m.version == version) else {
            continue;
        };

        // 完全一致，无需处理
        if migration.checksum.as_ref() == applied.as_slice() {
            continue;
        }

        let sql = migration.sql.as_ref();
        let lf = sql.replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        let is_line_ending_drift = [lf.as_bytes(), crlf.as_bytes()]
            .iter()
            .any(|candidate| Sha384::digest(candidate).as_slice() == applied.as_slice());

        if !is_line_ending_drift {
            // 真的改了内容：保留 sqlx 原有的 panic 行为，不静默放过
            eprintln!(
                "[db] 警告：迁移 {version} 的 SQL 内容与数据库记录不一致（非行尾差异），\
                 已保留 sqlx 的原始校验，不做自愈"
            );
            continue;
        }

        match sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(migration.checksum.as_ref().to_vec())
            .bind(version)
            .execute(pool)
            .await
        {
            Ok(_) => {
                healed += 1;
                eprintln!("[db] 已自愈迁移 {version} 的校验和（行尾 CRLF/LF 漂移，非内容变更）");
            }
            Err(error) => {
                eprintln!("[db] 迁移 {version} 校验和自愈失败（已忽略）：{error}");
            }
        }
    }
    healed
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
    let migrator = sqlx::migrate!("./migrations");
    // 先自愈行尾漂移（老库 + 新 exe 时 checksum 可能只是 CRLF/LF 差异），再让 sqlx 正常校验。
    // 自愈只处理确认的纯行尾差异，SQL 内容真被篡改时不动，由 sqlx 照常 panic。
    let healed = heal_migration_line_endings(&pool, &migrator).await;
    if healed > 0 {
        eprintln!("[db] 迁移校验和自愈完成，共修正 {healed} 条（行尾 CRLF/LF 漂移）");
    }
    migrator
        .run(&pool)
        .await
        .map_err(|error| AppError::Other(error.to_string()))?;
    // 幂等回填历史 GitHub 项的 author_id / author_name（早期版本未写入），
    // 失败仅打印告警、不阻断启动。
    if let Err(error) = backfill_github_author_ids(&pool).await {
        eprintln!("[db] GitHub author_id 回填失败（已忽略）：{error}");
    }
    // 幂等回填历史 B站图文（opus）项的 author_id（早期版本把 mid 当数字解析，
    // 而真实响应里 mid 是字符串导致全部丢失）。失败仅打印告警、不阻断启动。
    if let Err(error) = backfill_opus_author_ids(&pool).await {
        eprintln!("[db] B站图文 author_id 回填失败（已忽略）：{error}");
    }
    Ok(pool)
}

/// 一次性回填：为历史 GitHub 收藏项补全 `author_id` / `author_name`。
///
/// 早期版本的 GitHub 适配器未写入 `author_id`（恒为 NULL），导致卡片「作者」栏无法跳转
/// 到原作者主页。这里从 `source_url`（`https://github.com/{owner}/{repo}`）解析出仓库 owner
/// 作为 `author_id`，并在 `author_name` 为空时同样补上（保证前端作者链接可点）。
/// 幂等：仅更新 `author_id` 为空、来源为 github、且 URL 形如 github.com 的行。
pub async fn backfill_github_author_ids(pool: &SqlitePool) -> Result<u64, AppError> {
    let rows = sqlx::query(
        "SELECT id, source_url FROM items \
         WHERE source = 'github' AND author_id IS NULL AND source_url LIKE '%github.com/%'",
    )
    .fetch_all(pool)
    .await?;

    let mut updated = 0u64;
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let url: String = row.try_get("source_url")?;
        if let Some(owner) = github_owner_from_url(&url) {
            sqlx::query(
                "UPDATE items SET author_id = ?, author_name = COALESCE(author_name, ?) WHERE id = ?",
            )
            .bind(&owner)
            .bind(&owner)
            .bind(id)
            .execute(pool)
            .await?;
            updated += 1;
        }
    }
    if updated > 0 {
        eprintln!("[db] 回填 GitHub author_id / author_name 完成，更新 {updated} 条");
    }
    Ok(updated)
}

/// 从 GitHub 仓库 URL 解析 owner：`https://github.com/{owner}/{repo}` → `{owner}`。
/// 仅接受 `github.com/<owner>/<repo>` 形态（owner 后必有 `/`），避免误解析裸域名。
fn github_owner_from_url(url: &str) -> Option<String> {
    let marker = "github.com/";
    let start = url.find(marker)? + marker.len();
    let rest = &url[start..];
    let slash = rest.find('/')?;
    let owner = &rest[..slash];
    if owner.is_empty() {
        None
    } else {
        Some(owner.to_string())
    }
}

/// 一次性回填：为历史 B站图文（opus）收藏项补全 `author_id`。
///
/// 早期版本的 `parse_opus_item` 用 `Value::as_i64` 解析 `author.mid`，
/// 而「图文收藏」动态流接口返回的 mid 实际是**字符串**（`"3824575"`），
/// 导致 `author_id` 恒为 NULL、卡片「作者」栏无法跳转作者空间。
/// 这里从 `extra_json`（保留了完整原始响应）里重新取出 `author.mid` 回填。
/// 幂等：仅更新 `author_id` 为空、来源为 bilibili、且 external_id 以 `opus_` 开头的行。
pub async fn backfill_opus_author_ids(pool: &SqlitePool) -> Result<u64, AppError> {
    let rows = sqlx::query(
        "SELECT id, extra_json FROM items \
         WHERE source = 'bilibili' AND external_id LIKE 'opus_%' AND author_id IS NULL",
    )
    .fetch_all(pool)
    .await?;

    let mut updated = 0u64;
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let extra: String = row.try_get("extra_json")?;
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&extra) {
            if let Some(mid) = json.pointer("/author/mid").and_then(opus_mid_to_string) {
                sqlx::query("UPDATE items SET author_id = ? WHERE id = ?")
                    .bind(&mid)
                    .bind(id)
                    .execute(pool)
                    .await?;
                updated += 1;
            }
        }
    }
    if updated > 0 {
        eprintln!("[db] 回填 B站图文 author_id 完成，更新 {updated} 条");
    }
    Ok(updated)
}

/// 把 opus 响应里的 `author.mid` 统一成字符串 id（兼容字符串/数字两种形态）。
fn opus_mid_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => n.as_i64().map(|mid| mid.to_string()),
        _ => None,
    }
}

/// `ItemRow` 解码所需的列清单（唯一来源）。
///
/// `sqlx::query_as` 是**运行时按列名**解码的：给 `ItemRow` 加了字段但某处 SELECT 漏掉它时，
/// 编译期不会报错，只会在运行时抛 `ColumnNotFound`，表现为整个列表空白（曾因此导致收藏库
/// 与回收站页面全空）。所以所有 `ItemRow` 查询都必须引用这个常量，禁止手写列清单。
const ITEM_ROW_COLUMNS: &str = "id, source, external_id, source_url, title, description, notes, \
     cover_url, cover_local_path, author_name, author_id, partition_name, published_at, duration, \
     favorite_time, deleted_at, obsidian_path, starred, starred_at";

/// `tag_categories` 的行查询列清单。`category_from_row` 是运行时按列名解码的，
/// 漏列在 `cargo check` 阶段完全无感，只在运行时抛 `ColumnNotFound`。
/// （曾漏 `group_id`，导致「同名分类已存在」这条路径直接 panic，见回归测试
/// `tag_category_queries_return_every_column`。）加列时改这里即可。
const CATEGORY_ROW_COLUMNS: &str = "id, name, normalized, color, position, group_id";

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
    obsidian_path: Option<String>,
    starred: bool,
    starred_at: Option<i64>,
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
            obsidian_path: self.obsidian_path.clone(),
            starred: self.starred,
            starred_at: self.starred_at,
            tags,
        }
    }
}

/// 按复合键 `(source, external_id)` 查本地条目 id，查不到返回 `None`。
///
/// 速记面板用它判断「这首歌是不是已经在库里」，从而显示「更新」而不是「新建」。
pub async fn find_item_id(
    pool: &SqlitePool,
    source: &str,
    external_id: &str,
) -> Result<Option<i64>, AppError> {
    let id =
        sqlx::query_scalar::<_, i64>("SELECT id FROM items WHERE source = ? AND external_id = ?")
            .bind(source)
            .bind(external_id)
            .fetch_optional(pool)
            .await?;
    Ok(id)
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
        let mut fts_conn = pool.acquire().await?;
        update_fts_row(&mut fts_conn, id, item).await?;
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
        let mut fts_conn = pool.acquire().await?;
        update_fts_row(&mut fts_conn, id, item).await?;
        Ok((id, true))
    }
}

/// 泛型执行器版本：既接受 `&SqlitePool`，也接受事务里的 `&mut SqliteConnection`，
/// 让批量导入能把所有写操作收敛进同一个事务（每条语句一次 WAL fsync → 整个导入一次）。
async fn update_fts_row(
    conn: &mut SqliteConnection,
    item_id: i64,
    item: &ExternalItem,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM items_fts WHERE rowid = ?")
        .bind(item_id)
        .execute(&mut *conn)
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
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn rebuild_item_fts(pool: &SqlitePool, item_id: i64) -> Result<(), AppError> {
    let mut conn = pool.acquire().await?;
    rebuild_item_fts_conn(&mut conn, item_id).await
}

async fn rebuild_item_fts_conn(conn: &mut SqliteConnection, item_id: i64) -> Result<(), AppError> {
    let item_sql = format!("SELECT {ITEM_ROW_COLUMNS} FROM items WHERE id = ?");
    let row = sqlx::query_as::<_, ItemRow>(&item_sql)
        .bind(item_id)
        .fetch_one(&mut *conn)
        .await?;
    let tag_names = sqlx::query_scalar::<_, String>(
        "SELECT name FROM tags t JOIN item_tags it ON it.tag_id = t.id WHERE it.item_id = ?",
    )
    .bind(item_id)
    .fetch_all(&mut *conn)
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
    update_fts_row(&mut *conn, item_id, &item).await
}

pub async fn get_or_create_tag(pool: &SqlitePool, input: &TagInput) -> Result<i64, AppError> {
    let mut conn = pool.acquire().await?;
    get_or_create_tag_conn(&mut conn, input).await
}

async fn get_or_create_tag_conn(
    conn: &mut SqliteConnection,
    input: &TagInput,
) -> Result<i64, AppError> {
    let normalized = normalize_tag(&input.name);
    if normalized.is_empty() {
        return Err(AppError::InvalidInput("标签名称不能为空".into()));
    }
    let color = tag_color(&input.name, input.color.clone());

    // 纯 find-or-create：已有同名（normalized）标签直接复用，绝不改动其任何字段
    // （名称/颜色/描述/分类）。此前若调用方携带 id 会触发全字段 UPDATE 覆盖，
    // 导致导入/挂接标签时把「已分类标签」打回未分类（category_id 被写成 NULL）。
    // 编辑标签本体请走 upsert_tag（显式 id = 全字段更新）。
    if let Some(id) =
        sqlx::query_scalar::<_, i64>("SELECT id FROM tags WHERE normalized = ? ORDER BY id LIMIT 1")
            .bind(&normalized)
            .fetch_optional(&mut *conn)
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
    .fetch_one(&mut *conn)
    .await?;
    Ok(id)
}

pub async fn attach_tag(pool: &SqlitePool, item_id: i64, tag_id: i64) -> Result<(), AppError> {
    let mut conn = pool.acquire().await?;
    attach_tag_conn(&mut conn, item_id, tag_id).await
}

async fn attach_tag_conn(
    conn: &mut SqliteConnection,
    item_id: i64,
    tag_id: i64,
) -> Result<(), AppError> {
    sqlx::query("INSERT OR IGNORE INTO item_tags (item_id, tag_id, created_at) VALUES (?, ?, ?)")
        .bind(item_id)
        .bind(tag_id)
        .bind(now_seconds())
        .execute(&mut *conn)
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
    // 显式 id = 编辑既有标签：允许全字段覆盖（改名 / 改色 / 改描述 / 移出分类都可行）。
    // 这是唯一允许覆盖既有标签字段的入口；其余 find-or-create 路径一律不动已有行。
    // 若 id 在库中已不存在（被删过），退化为 find-or-create 兜底。
    let tag_id = match input.id {
        Some(id) => {
            let exists = sqlx::query_scalar::<_, i64>("SELECT id FROM tags WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await?;
            if exists.is_some() {
                let normalized = normalize_tag(&input.name);
                if normalized.is_empty() {
                    return Err(AppError::InvalidInput("标签名称不能为空".into()));
                }
                let color = tag_color(&input.name, input.color.clone());
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
                id
            } else {
                get_or_create_tag(pool, input).await?
            }
        }
        None => get_or_create_tag(pool, input).await?,
    };
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
    let item_sql = format!("SELECT {ITEM_ROW_COLUMNS} FROM items WHERE id = ?");
    let row = sqlx::query_as::<_, ItemRow>(&item_sql)
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
    let item_sql = format!("SELECT {ITEM_ROW_COLUMNS} FROM items WHERE id = ?");
    let row = sqlx::query_as::<_, ItemRow>(&item_sql)
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

/// 读取单条收藏（供 Obsidian 打开 / 导出命令使用）。
pub async fn get_item(pool: &SqlitePool, item_id: i64) -> Result<VideoItem, AppError> {
    let item_sql = format!("SELECT {ITEM_ROW_COLUMNS} FROM items WHERE id = ?");
    let row = sqlx::query_as::<_, ItemRow>(&item_sql)
        .bind(item_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("收藏不存在: {item_id}")))?;
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

/// 打星 / 取消星标。打星时记录时间（首次打星保留原时间，重复置顶不刷新次序），
/// 取消时清空 starred 与 starred_at。
pub async fn set_item_starred(
    pool: &SqlitePool,
    item_id: i64,
    starred: bool,
) -> Result<VideoItem, AppError> {
    let now = now_seconds();
    if starred {
        sqlx::query(
            "UPDATE items SET starred = 1, starred_at = COALESCE(starred_at, ?), updated_at = ? \
             WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(item_id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("UPDATE items SET starred = 0, starred_at = NULL, updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(item_id)
            .execute(pool)
            .await?;
    }
    get_item(pool, item_id).await
}
/// 回写该收藏同步到的 Obsidian 笔记相对路径（相对 vault 根）。
pub async fn set_item_obsidian_path(
    pool: &SqlitePool,
    item_id: i64,
    rel_path: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE items SET obsidian_path = ?, updated_at = ? WHERE id = ?")
        .bind(rel_path)
        .bind(now_seconds())
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 读取该收藏同步到的 Obsidian 笔记相对路径。
/// 批注弹窗打开时调用：即使前端 item 快照里 obsidianPath 为空，
/// 也能据此点亮「在 Obsidian 中打开」（导出成功后数据库已回写该列）。
pub async fn get_item_obsidian_path(
    pool: &SqlitePool,
    item_id: i64,
) -> Result<Option<String>, AppError> {
    let row =
        sqlx::query_as::<_, (Option<String>,)>("SELECT obsidian_path FROM items WHERE id = ?")
            .bind(item_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|r| r.0))
}

/// 快速入库 / 侧边栏笔记面板用：按 `(source, external_id)` 查已有条目。
///
/// 只取回填侧边栏与乐观锁需要的最小字段，**不要**扩成完整行——
/// 多一个列就多一处 `query_as` 漏列风险（见 `ITEM_ROW_COLUMNS` 的约定）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CapturedItem {
    pub id: i64,
    pub source: String,
    pub external_id: String,
    /// 入库时记录的原始链接；侧边栏按 URL 匹配时要用它做优先级排序。
    pub source_url: String,
    pub title: String,
    pub notes: String,
    /// 乐观锁基准（`items.updated_at`）。侧边栏保存时回传，用于检测「笔记已在别处被改」。
    pub updated_at: Option<i64>,
    /// 已同步到 vault 的相对路径；未同步为 None。
    pub obsidian_path: Option<String>,
}

pub async fn find_item_by_source_id(
    pool: &SqlitePool,
    source: &str,
    external_id: &str,
) -> Result<Option<CapturedItem>, AppError> {
    let row = sqlx::query_as::<_, CapturedItem>(
        "SELECT id, source, external_id, source_url, title, notes, updated_at, obsidian_path
         FROM items WHERE source = ? AND external_id = ?",
    )
    .bind(source)
    .bind(external_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 按一组候选 `source_url` 查条目，返回**优先匹配**的那条。
///
/// 侧边栏拿到的是当前地址栏里的 URL，常带 `?spm_id_from=` / `?t=` 等参数，
/// 与入库时的 `source_url` 并不字面相等。调用方把「原始 → 归一化 → 去参数」
/// 几个形态一起传进来，这里按传入顺序取第一个命中的，`candidates[0]` 即最高优先级。
/// 空候选集直接返回 `None`，不发 SQL。
pub async fn find_item_by_source_urls(
    pool: &SqlitePool,
    candidates: &[String],
) -> Result<Option<CapturedItem>, AppError> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let placeholders = candidates.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT id, source, external_id, source_url, title, notes, updated_at, obsidian_path
         FROM items WHERE source_url IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<_, CapturedItem>(&sql);
    for url in candidates {
        query = query.bind(url);
    }
    let rows = query.fetch_all(pool).await?;
    if rows.is_empty() {
        return Ok(None);
    }
    // IN 不保证顺序，按 candidates 的优先级重排后再取第一条。
    let mut best: Option<(usize, CapturedItem)> = None;
    for row in rows {
        let index = match candidates.iter().position(|u| *u == row.source_url) {
            Some(i) => i,
            None => continue,
        };
        if best.as_ref().map_or(true, |(b, _)| index < *b) {
            best = Some((index, row));
        }
    }
    Ok(best.map(|(_, row)| row))
}

/// 读取条目的 `updated_at`，供乐观锁在**写入前**取当前值比对。
pub async fn get_item_updated_at(pool: &SqlitePool, item_id: i64) -> Result<Option<i64>, AppError> {
    let row = sqlx::query_as::<_, (Option<i64>,)>("SELECT updated_at FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.0))
}

/// 快速入库用：更新已有条目的标题与备注。
///
/// 刻意**不**走 `upsert_item`——那会清空 `cover_local_path` 并整体替换 `extra_json`，
/// 把书签导入写进去的 `folder_tags` 抹掉，已下载的封面也会白重下一次。
/// 顺带把 `deleted_at` 置空：重新收藏回收站里的条目等同于恢复它。
pub async fn update_captured_item(
    pool: &SqlitePool,
    item_id: i64,
    title: &str,
    notes: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE items SET title = ?, notes = ?, deleted_at = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(title)
    .bind(notes)
    .bind(now_seconds())
    .bind(item_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 读取某条目已挂载的标签名（侧边栏回填用）。
pub async fn item_tag_names(pool: &SqlitePool, item_id: i64) -> Result<Vec<String>, AppError> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT t.name FROM tags t
         JOIN item_tags it ON it.tag_id = t.id
         WHERE it.item_id = ?
         ORDER BY t.name COLLATE NOCASE",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    Ok(names)
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
    // 先收集源标签挂过的 item（合并后这些 item 的标签集合变了，FTS 需要重建）
    let affected: Vec<i64> =
        sqlx::query_scalar("SELECT DISTINCT item_id FROM item_tags WHERE tag_id = ?")
            .bind(source_tag_id)
            .fetch_all(&mut *tx)
            .await?;
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
    // 合并改变了受影响 item 的标签集合，逐个重建 FTS 索引（items_fts 无触发器）
    for item_id in affected {
        rebuild_item_fts(pool, item_id).await?;
    }
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
    let sql = format!(
        "SELECT {CATEGORY_ROW_COLUMNS}
         FROM tag_categories
         ORDER BY position, name COLLATE NOCASE"
    );
    let rows = sqlx::query(&sql)
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
        group_id: row.get("group_id"),
    }
}

pub async fn create_tag_category(
    pool: &SqlitePool,
    name: &str,
    color: Option<String>,
) -> Result<TagCategory, AppError> {
    let mut conn = pool.acquire().await?;
    create_tag_category_conn(&mut conn, name, color).await
}

async fn create_tag_category_conn(
    conn: &mut SqliteConnection,
    name: &str,
    color: Option<String>,
) -> Result<TagCategory, AppError> {
    let normalized = normalize_tag(name);
    if normalized.is_empty() {
        return Err(AppError::InvalidInput("分类名称不能为空".into()));
    }
    // 同名（忽略大小写/首尾空格）已存在时直接返回已有分类，避免 UNIQUE 冲突报错，
    // 这样前端“新建分类”永远能得到该分类并立即显示。
    if let Some(existing) = get_tag_category_by_normalized(&mut *conn, &normalized).await? {
        return Ok(existing);
    }
    // 一条语句完成插入并 RETURNING 完整行，避免「INSERT 后另起连接二次 SELECT」
    // 在 WAL 模式下读不到刚提交行、触发 RowNotFound（"no rows returned"）的竞态。
    // position 取当前最大值 + 1：新建分类追加到列表末尾，随后用户可拖动重排。
    let row = sqlx::query(
        "INSERT INTO tag_categories (name, normalized, color, position, created_at)
         VALUES (?, ?, ?, (SELECT COALESCE(MAX(position), -1) + 1 FROM tag_categories), ?)
         RETURNING id, name, normalized, color, position, group_id",
    )
    .bind(name.trim())
    .bind(&normalized)
    .bind(&color)
    .bind(now_seconds())
    .map(category_from_row)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

/// 重排分类：按 ordered_ids 的顺序重写各分类的 position（0..n）。
/// 前端拖拽分类行（或整组块）后传入全量有序 id；列表查询按 position, name 排序。
pub async fn reorder_tag_categories(
    pool: &SqlitePool,
    ordered_ids: &[i64],
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    for (index, category_id) in ordered_ids.iter().enumerate() {
        sqlx::query("UPDATE tag_categories SET position = ? WHERE id = ?")
            .bind(index as i64)
            .bind(category_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// 把若干分类合并为一个「组」：
/// 1. 若选中分类原本属于其它组，先从旧组拆离（旧组剩下 ≥2 个成员仍成一组并重指 leader；
///    只剩 1 个则解除分组）；
/// 2. 整组颜色统一为「位置最靠前」成员的颜色，group_id 指向该 leader；
/// 3. 把组成员按原位置顺序聚拢成连续块（保持从 leader 所在位置起），并重写全表 position，
///    保证前端「按块拖动排序」时组成员在列表里始终连续。
pub async fn group_tag_categories(pool: &SqlitePool, category_ids: &[i64]) -> Result<(), AppError> {
    if category_ids.len() < 2 {
        return Err(AppError::InvalidInput("合并为组至少需要 2 个分类".into()));
    }
    let mut tx = pool.begin().await?;

    // 读取全部分类（按 position 排序），构造成员集
    struct RowInfo {
        id: i64,
        position: i64,
        color: Option<String>,
        group_id: Option<i64>,
    }
    let all: Vec<RowInfo> = sqlx::query_as::<_, (i64, i64, Option<String>, Option<i64>)>(
        "SELECT id, position, color, group_id FROM tag_categories ORDER BY position, name COLLATE NOCASE",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(id, position, color, group_id)| RowInfo {
        id,
        position,
        color,
        group_id,
    })
    .collect();

    let ids_set: std::collections::HashSet<i64> = category_ids.iter().copied().collect();
    if ids_set.len() < 2 {
        return Err(AppError::InvalidInput(
            "合并为组至少需要 2 个不同的分类".into(),
        ));
    }

    // 收集选中成员原属的组（用于拆离）
    let mut old_groups: Vec<i64> = all
        .iter()
        .filter(|r| ids_set.contains(&r.id))
        .filter_map(|r| r.group_id)
        .collect();
    old_groups.sort_unstable();
    old_groups.dedup();

    // 旧组残局处理：选中成员先脱组；旧组剩余成员重新成组/解组
    for group_leader in &old_groups {
        let remaining: Vec<i64> = all
            .iter()
            .filter(|r| !ids_set.contains(&r.id) && r.group_id.as_ref() == Some(group_leader))
            .map(|r| r.id)
            .collect();
        if remaining.len() >= 2 {
            // 剩余成员按 position 最小者作新 leader
            let new_leader = all
                .iter()
                .filter(|r| remaining.contains(&r.id))
                .min_by_key(|r| r.position)
                .map(|r| r.id)
                .unwrap();
            sqlx::query("UPDATE tag_categories SET group_id = ? WHERE id = ?")
                .bind(new_leader)
                .bind(new_leader)
                .execute(&mut *tx)
                .await?;
        } else if remaining.len() == 1 {
            sqlx::query("UPDATE tag_categories SET group_id = NULL WHERE id = ?")
                .bind(remaining[0])
                .execute(&mut *tx)
                .await?;
        }
        // remaining.len() == 0：整组并入新组，无需额外动作
    }

    // 选出 leader：成员中 position 最小者；其颜色作为整组颜色
    let mut members: Vec<&RowInfo> = all.iter().filter(|r| ids_set.contains(&r.id)).collect();
    members.sort_by_key(|r| r.position);
    let leader = members[0];
    let leader_color = leader.color.clone();

    // 用新组颜色统一全部成员并设置 group_id（leader 自指）
    for member in &members {
        sqlx::query("UPDATE tag_categories SET color = ?, group_id = ? WHERE id = ?")
            .bind(&leader_color)
            .bind(leader.id)
            .bind(member.id)
            .execute(&mut *tx)
            .await?;
    }

    // 重排：成员从 leader 原位置起连续排列，其余分类保持相对顺序，重写全表 position
    let ordered_ids: Vec<i64> = all.iter().map(|r| r.id).collect();
    let mut compact: Vec<i64> = ordered_ids
        .iter()
        .copied()
        .filter(|id| !ids_set.contains(id))
        .collect();
    let insert_at = compact
        .iter()
        .position(|id| {
            let pos = all
                .iter()
                .find(|r| r.id == *id)
                .map(|r| r.position)
                .unwrap_or(i64::MAX);
            pos > leader.position
        })
        .unwrap_or(compact.len());
    let member_ids: Vec<i64> = members.iter().map(|r| r.id).collect();
    compact.splice(insert_at..insert_at, member_ids.iter().copied());
    for (index, category_id) in compact.iter().enumerate() {
        sqlx::query("UPDATE tag_categories SET position = ? WHERE id = ?")
            .bind(index as i64)
            .bind(category_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// 拆分组：把某个分类所属的整组解除（该组全部成员的 group_id 置 NULL）。
/// 颜色保留现状（拆分后各成员可各自改色）。未分组的分类调用为 no-op。
pub async fn ungroup_tag_category(pool: &SqlitePool, category_id: i64) -> Result<(), AppError> {
    let group: Option<i64> = sqlx::query_scalar("SELECT group_id FROM tag_categories WHERE id = ?")
        .bind(category_id)
        .fetch_optional(pool)
        .await?
        .flatten();
    let Some(g) = group else {
        return Ok(());
    };
    sqlx::query("UPDATE tag_categories SET group_id = NULL WHERE group_id = ?")
        .bind(g)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn rename_tag_category(
    pool: &SqlitePool,
    id: i64,
    name: &str,
    color: Option<String>,
) -> Result<TagCategory, AppError> {
    let normalized = normalize_tag(name);
    // 该分类若属于某组，颜色改动要同步给整组（组成员共享同色）
    let group_id: Option<i64> =
        sqlx::query_scalar("SELECT group_id FROM tag_categories WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .flatten();
    // UPDATE ... RETURNING 一步返回完整行，避免 UPDATE 后二次 SELECT 的 WAL 竞态。
    let row = sqlx::query(
        "UPDATE tag_categories SET name = ?, normalized = ?, color = ? WHERE id = ?
         RETURNING id, name, normalized, color, position, group_id",
    )
    .bind(name.trim())
    .bind(&normalized)
    .bind(&color)
    .bind(id)
    .map(category_from_row)
    .fetch_one(pool)
    .await?;
    if let Some(g) = group_id {
        sqlx::query("UPDATE tag_categories SET color = ? WHERE group_id = ? AND id != ?")
            .bind(&color)
            .bind(g)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(row)
}

/// 删除分类后的组修复：某组（leader_hint 指 group_id 目标）内除 exclude 外的剩余成员
/// ≥2 时以 position 最小者为新 leader 继续成组；恰剩 1 个时解除分组。
async fn repair_category_group_after_delete(
    pool: &SqlitePool,
    leader_hint: Option<i64>,
    exclude: i64,
) -> Result<(), AppError> {
    let Some(g) = leader_hint else { return Ok(()) };
    let remain: Vec<(i64, i64)> =
        sqlx::query_as("SELECT id, position FROM tag_categories WHERE group_id = ? AND id != ?")
            .bind(g)
            .bind(exclude)
            .fetch_all(pool)
            .await?;
    if remain.len() >= 2 {
        let new_leader = remain
            .iter()
            .min_by_key(|(_, pos)| *pos)
            .map(|(id, _)| *id)
            .unwrap();
        sqlx::query("UPDATE tag_categories SET group_id = ? WHERE group_id = ?")
            .bind(new_leader)
            .bind(g)
            .execute(pool)
            .await?;
    } else if remain.len() == 1 {
        sqlx::query("UPDATE tag_categories SET group_id = NULL WHERE id = ?")
            .bind(remain[0].0)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn delete_tag_category(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    // 该分类所在组 / 该分类作为 leader 的组，删除后要让剩余成员重新成组或解组
    let own_group: Option<i64> =
        sqlx::query_scalar("SELECT group_id FROM tag_categories WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .flatten();

    sqlx::query("UPDATE tags SET category_id = NULL WHERE category_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM tag_categories WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    // 1) 被删行是普通成员：它所属的组（leader=own_group）其余成员需重组
    repair_category_group_after_delete(pool, own_group, id).await?;
    // 2) 被删行是组长：指向它的所有行需重组（此时它们 group_id 仍 = id）
    repair_category_group_after_delete(pool, Some(id), id).await?;

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

/// 批量软删除，**一个事务**完成，返回实际被删的行数。
///
/// 别用 `soft_delete_items`：它逐条各开一个事务，每条一次 WAL fsync
/// （3.15：自动提交 ~2.3 ms/条）。同步一轮可能要删几十上百条，必须收敛进一个事务。
pub async fn soft_delete_items_bulk(
    pool: &SqlitePool,
    item_ids: &[i64],
) -> Result<usize, AppError> {
    if item_ids.is_empty() {
        return Ok(0);
    }
    let now = now_seconds();
    let mut tx = pool.begin().await?;
    let mut count = 0usize;
    for item_id in item_ids {
        let res = sqlx::query("UPDATE items SET deleted_at = ?, updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(now)
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() > 0 {
            // items_fts 是 contentless FTS5，**没有触发器**，必须手动删
            sqlx::query("DELETE FROM items_fts WHERE rowid = ?")
                .bind(item_id)
                .execute(&mut *tx)
                .await?;
            count += 1;
        }
    }
    tx.commit().await?;
    Ok(count)
}

/// 网易云同步用：列出全部未删除的 netease 条目 `(item_id, external_id, extra_json)`。
///
/// `extra_json` 里有 `playlistId`，同步靠它判断一首歌归属哪个歌单，
/// 从而区分「用户取消收藏」和「这首歌本来就不在同步范围内」。
pub async fn list_netease_active_items(
    pool: &SqlitePool,
) -> Result<Vec<(i64, String, String)>, AppError> {
    let rows = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT id, external_id, COALESCE(extra_json, '') FROM items
         WHERE source = 'netease' AND deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn restore_item(pool: &SqlitePool, item_id: i64) -> Result<(), AppError> {
    let mut conn = pool.acquire().await?;
    restore_item_conn(&mut conn, item_id).await
}

async fn restore_item_conn(conn: &mut SqliteConnection, item_id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE items SET deleted_at = NULL, updated_at = ? WHERE id = ?")
        .bind(now_seconds())
        .bind(item_id)
        .execute(&mut *conn)
        .await?;
    rebuild_item_fts_conn(&mut *conn, item_id).await?;
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
    let item_sql = format!(
        "SELECT {ITEM_ROW_COLUMNS} FROM items WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC"
    );
    let rows = sqlx::query_as::<_, ItemRow>(&item_sql)
        .fetch_all(pool)
        .await?;
    hydrate_items(pool, rows).await
}

pub async fn get_trash_count(pool: &SqlitePool) -> Result<i64, AppError> {
    let count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM items WHERE deleted_at IS NOT NULL")
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
    conn: &mut SqliteConnection,
    normalized: &str,
) -> Result<Option<TagCategory>, AppError> {
    let sql = format!("SELECT {CATEGORY_ROW_COLUMNS} FROM tag_categories WHERE normalized = ?");
    let row = sqlx::query(&sql)
        .bind(normalized)
        .map(category_from_row)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row)
}

pub async fn create_import_run(
    pool: &SqlitePool,
    collection: &CollectionInfo,
    total: i64,
    cleanup_eligible: bool,
) -> Result<i64, AppError> {
    let mut conn = pool.acquire().await?;
    create_import_run_conn(&mut conn, collection, total, cleanup_eligible).await
}

async fn create_import_run_conn(
    conn: &mut SqliteConnection,
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
    .fetch_one(&mut *conn)
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
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {ITEM_ROW_COLUMNS} FROM items i WHERE 1 = 1"
    ));

    // 回收站过滤：默认（trash 为 None/false）仅显示正常在库项；trash=Some(true) 时只看回收站。
    match &filters.trash {
        Some(true) => {
            query.push(" AND i.deleted_at IS NOT NULL");
        }
        _ => {
            query.push(" AND i.deleted_at IS NULL");
        }
    }

    // 无标签筛选：item 未挂任何标签（item_tags 无关联行）。
    // 与 tag_ids 互斥——前端在 UI 上已保证，这里再做防御：开启时忽略 tag_ids。
    if filters.untagged {
        query.push(" AND NOT EXISTS (SELECT 1 FROM item_tags it WHERE it.item_id = i.id)");
    } else if !filters.tag_ids.is_empty() {
        if filters.strict {
            // 严格匹配：item 的标签集合「恰好等于」输入的 tag_ids。
            // (1) 必须包含所有输入标签：按 item_id 分组后，命中的不同标签数 == 输入标签数。
            // (2) 不能包含任何输入之外的标签：排除掉那些挂了「非输入标签」的 item。
            // 两个条件合起来即「标签集合完全相等」——例如输入 {a} 时过滤掉同时含 a、b 的 item。
            let n = filters.tag_ids.len() as i64;
            query.push(" AND i.id IN (SELECT item_id FROM item_tags WHERE tag_id IN (");
            {
                let mut separated = query.separated(", ");
                for tag_id in &filters.tag_ids {
                    separated.push_bind(tag_id);
                }
            }
            query
                .push(") GROUP BY item_id HAVING COUNT(DISTINCT tag_id) = ")
                .push_bind(n)
                .push(")");
            query.push(" AND i.id NOT IN (SELECT item_id FROM item_tags WHERE tag_id NOT IN (");
            {
                let mut separated = query.separated(", ");
                for tag_id in &filters.tag_ids {
                    separated.push_bind(tag_id);
                }
            }
            query.push("))");
        } else if filters.tag_mode == "or" {
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
    // 星标置顶：任何排序下星标项都排在前面（星标组内部按打星时间倒序）。
    // 非星标行在第二个键上为 NULL——SQLite 中 DESC 排序 NULL 落在最后，
    // 因此它们继续按用户所选 sort_sql 排序，互不干扰。
    query
        .push(
            " ORDER BY i.starred DESC, \
             CASE WHEN i.starred = 1 THEN COALESCE(i.starred_at, i.favorite_time) END DESC, ",
        )
        .push(sort_sql);

    let rows = query.build_query_as::<ItemRow>().fetch_all(pool).await?;
    hydrate_items(pool, rows).await
}

async fn hydrate_items(pool: &SqlitePool, rows: Vec<ItemRow>) -> Result<Vec<VideoItem>, AppError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // 一次性取出每个标签的「全局使用次数」：对整张 item_tags 做一次 GROUP BY 即可。
    // 这与旧实现「逐条查询时 COUNT(it2.item_id) 且 it2 不受 item_id 限制」的语义完全一致
    // （count 是该标签在所有 item 上的总次数，而非当前结果集内次数），
    // 但成本是 O(总 item_tags 行数)，与本次返回多少 item 无关 —— 不会随收藏量膨胀。
    let counts: HashMap<i64, i64> =
        sqlx::query("SELECT tag_id, COUNT(*) AS cnt FROM item_tags GROUP BY tag_id")
            .map(|row: SqliteRow| (row.get::<i64, _>("tag_id"), row.get::<i64, _>("cnt")))
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();

    // 按 item_id 批量取标签，避免逐条查询（N+1）。
    // IN 子句按 500 个一批切分，既把查询次数压到「行数/500」，
    // 又不超过 SQLite 默认 999 绑定变量上限。查询本身只 JOIN 命中的 (item,tag) 行，
    // 不再自连接全表，因此成本随返回量线性增长而非爆炸。
    let mut tags_by_item: HashMap<i64, Vec<Tag>> = HashMap::with_capacity(rows.len());
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    for chunk in ids.chunks(500) {
        let mut qb = QueryBuilder::new(
            "SELECT it.item_id AS item_id, t.id, t.namespace, t.name, t.normalized, t.color, \
             t.description, t.category_id \
             FROM tags t \
             JOIN item_tags it ON it.tag_id = t.id \
             WHERE it.item_id IN (",
        );
        let mut separated = qb.separated(", ");
        for id in chunk {
            separated.push_bind(*id);
        }
        qb.push(") ORDER BY t.name COLLATE NOCASE");
        let pairs = qb
            .build()
            .map(|row: SqliteRow| {
                let tag_id: i64 = row.get("id");
                let tag = Tag {
                    id: tag_id,
                    namespace: row.get("namespace"),
                    name: row.get("name"),
                    normalized: row.get("normalized"),
                    color: row.get("color"),
                    description: row.get("description"),
                    count: *counts.get(&tag_id).unwrap_or(&0),
                    category_id: row.get("category_id"),
                };
                (row.get::<i64, _>("item_id"), tag)
            })
            .fetch_all(pool)
            .await?;
        for (item_id, tag) in pairs {
            tags_by_item.entry(item_id).or_default().push(tag);
        }
    }

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let tags = tags_by_item.remove(&row.id).unwrap_or_default();
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
    obsidian_path: Option<String>,
    extra_json: String,
    starred: bool,
    starred_at: Option<i64>,
}

/// 把收藏库导出为 `CollectionExport`。`item_ids` 为 None 时导出全部，为空数组时不导出任何项。
pub async fn export_items(
    pool: &SqlitePool,
    item_ids: Option<Vec<i64>>,
) -> Result<CollectionExport, AppError> {
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT id, source, external_id, source_url, title, description, notes, cover_url,
                author_name, author_id, partition_name, published_at, duration, favorite_time,
                obsidian_path, extra_json, starred, starred_at
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
            obsidian_path: row.obsidian_path,
            starred: row.starred,
            starred_at: row.starred_at,
            extra,
            tags,
        });
    }

    Ok(CollectionExport {
        format_version: 1,
        exported_at: now_seconds(),
        app: "collectorlite".into(),
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

/// 批量回写封面本地路径。逐条 UPDATE 时每条都是一次自动提交事务（实测约 2.3 ms/条，
/// 3000 条约 7 秒纯 fsync），收进一个事务后只剩一次提交。
pub async fn set_items_cover_local_paths(
    pool: &SqlitePool,
    rows: &[(String, String, String)],
) -> Result<(), AppError> {
    if rows.is_empty() {
        return Ok(());
    }
    let now = now_seconds();
    let mut tx = pool.begin().await?;
    for (source, external_id, path) in rows {
        sqlx::query(
            "UPDATE items SET cover_local_path = ?, updated_at = ? WHERE source = ? AND external_id = ?",
        )
        .bind(path)
        .bind(now)
        .bind(source)
        .bind(external_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// 该项是否已有本地封面缓存。capture 路径据此决定是否要补下载，避免对已缓存项重复抓取。
pub async fn item_has_local_cover(
    pool: &SqlitePool,
    source: &str,
    external_id: &str,
) -> Result<bool, AppError> {
    let path = sqlx::query_scalar::<_, Option<String>>(
        "SELECT cover_local_path FROM items WHERE source = ? AND external_id = ?",
    )
    .bind(source)
    .bind(external_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(path.map(|value| !value.is_empty()).unwrap_or(false))
}

/// 「待缓存封面」的判定条件。数据库本身就是封面缓存的任务队列，这个条件就是队列的定义，
/// 计数与取任务必须用它，避免两边漂移。
///
/// 历史上只覆盖 bilibili/csdn（带 source 适配器的「准官方」源）；browser/zhihu/github
/// 走 `download_cover_generic`（capture.rs），但它们的 item 从不入队，导入时下载一旦失败
/// 就再也救不回来 —— 是「侧边栏入库封面爬不到」系列的连锁放大点之一。把它们也纳入队列
/// 后，任何漏抓的封面都会被后台 `cover_cache` 任务自然补上。
const PENDING_COVER_WHERE: &str = "source IN ('bilibili', 'csdn', 'browser', 'zhihu', 'github')
           AND cover_url IS NOT NULL AND cover_url <> ''
           AND (cover_local_path IS NULL OR cover_local_path = '')";

/// 还有多少张封面等着我缓存（用于给用户展示进度与「上次没缓存完」的提示）。
pub async fn count_pending_cover_cache(pool: &SqlitePool) -> Result<i64, AppError> {
    let sql = format!("SELECT COUNT(*) FROM items WHERE {PENDING_COVER_WHERE}");
    let count = sqlx::query_scalar::<_, i64>(&sql).fetch_one(pool).await?;
    Ok(count)
}

/// 取出需要补缓存封面的项：bilibili / csdn 来源、有远程 cover_url、但本地缓存为空的项。
pub async fn fetch_items_needing_cover_cache(
    pool: &SqlitePool,
) -> Result<Vec<ExternalItem>, AppError> {
    let sql = format!(
        "SELECT source, external_id, cover_url FROM items
         WHERE {PENDING_COVER_WHERE}"
    );
    let rows = sqlx::query(&sql)
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
    let mut imported: i64 = 0;
    let mut skipped: i64 = 0;
    let mut failed: i64 = 0;
    let mut errors: Vec<String> = Vec::new();
    let mut new_items: Vec<ExternalItem> = Vec::new();

    // ── 性能：所有写操作收敛进一个事务，并把标签 / 分类查表结果缓存在内存 ──
    // 自动提交模式下每条**写**语句都要一次 WAL fsync：实测 3000 条约 2.3 ms/条，
    // 事务内约 0.02 ms/条（差 100 倍）。导入 3000 条：48 s → ~1 s。
    // 单条语句失败不会中断事务，仍按 failed 计并继续（与旧行为一致）。
    let mut tx = pool.begin().await?;
    // 注意：run 记录必须在**同一个事务 / 同一条连接**上创建。
    // 若先用连接池的另一条连接写入再回来「先读后写」，SQLite 会返回
    // SQLITE_BUSY「database is locked」（写锁冲突，且不会被 busy handler 重试）。
    let run_id =
        create_import_run_conn(&mut *tx, &collection, export.items.len() as i64, false).await?;

    // ① 一次性读入库中已有的 (source, external_id)，省掉每条 1 次存在性 SELECT
    let mut known: HashMap<(String, String), (i64, Option<i64>)> =
        sqlx::query_as::<_, (i64, String, String, Option<i64>)>(
            "SELECT id, source, external_id, deleted_at FROM items",
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|(id, source, external_id, deleted_at)| ((source, external_id), (id, deleted_at)))
        .collect();
    known.reserve(export.items.len());

    // ② 标签 / 分类缓存：3000 条收藏通常只对应几十个不同标签，没必要重复查库
    let mut tag_cache: HashMap<String, i64> = HashMap::new(); // normalized → tag_id
    let mut category_cache: HashMap<String, i64> = HashMap::new(); // normalized → category_id

    for item in &export.items {
        if item.source.trim().is_empty() || item.external_id.trim().is_empty() {
            failed += 1;
            errors.push(format!("跳过无效项（缺少来源或 ID）：{}", item.title));
            continue;
        }

        // 已存在时区分两种情况：
        //  - 正常在库 → 增量模式，跳过，绝不覆盖（保护用户可能已改的批注 / 标签 / 封面）
        //  - 在回收站 → 视为"重新收藏"，直接恢复（与实时导入 `upsert_item` 的语义一致）
        //
        // 旧行为是一律 `skipped += 1`，导致回收站里的条目用备份文件**永远导不回来**：
        // 只能在保留期内去回收站手动恢复，一旦过期被自动清理就彻底丢失，
        // 而用户侧只看到"导入 0 条"，完全不知道为什么。
        let key = (
            item.source.trim().to_string(),
            item.external_id.trim().to_string(),
        );
        if let Some(&(existing_id, deleted_at)) = known.get(&key) {
            if deleted_at.is_none() {
                skipped += 1;
                continue;
            }
            // 恢复回收站条目（清 deleted_at + 重建 FTS），保留其原有标签与封面
            restore_item_conn(&mut *tx, existing_id).await?;
            known.insert(key, (existing_id, None));
            // 计入 imported：此刻它确实回到了收藏库，比报"跳过"更符合用户预期
            imported += 1;
            continue;
        }

        let now = now_seconds();
        let extra_str = serde_json::to_string(&item.extra).unwrap_or_else(|_| "null".to_string());

        let insert_result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO items (
                source, external_id, source_url, title, description, cover_url, cover_local_path, author_name,
                author_id, partition_name, published_at, duration, favorite_time, notes, obsidian_path, extra_json,
                starred, starred_at, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(&item.obsidian_path)
        .bind(&extra_str)
        .bind(item.starred)
        .bind(item.starred_at)
        .bind(now)
        .bind(now)
        .fetch_one(&mut *tx)
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
        let mut tag_names: Vec<String> = Vec::with_capacity(item.tags.len());
        for tag in &item.tags {
            let name = tag.name.trim();
            let normalized = normalize_tag(name);
            if normalized.is_empty() {
                continue;
            }
            // 分类：同名分类只查一次库（缓存命中后零查询）
            let category_id = match tag
                .category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(cat_name) => {
                    let cat_norm = normalize_tag(cat_name);
                    match category_cache.get(&cat_norm) {
                        Some(&cid) => Some(cid),
                        None => match create_tag_category_conn(&mut *tx, cat_name, None).await {
                            Ok(cat) => {
                                category_cache.insert(cat_norm, cat.id);
                                Some(cat.id)
                            }
                            Err(_) => None,
                        },
                    }
                }
                None => None,
            };
            let tag_id = match tag_cache.get(&normalized) {
                Some(&tid) => tid,
                None => {
                    let input = TagInput {
                        id: None,
                        namespace: if tag.namespace.trim().is_empty() {
                            "manual".into()
                        } else {
                            tag.namespace.clone()
                        },
                        name: name.to_string(),
                        color: tag.color.clone(),
                        description: None,
                        category_id,
                    };
                    match get_or_create_tag_conn(&mut *tx, &input).await {
                        Ok(tid) => {
                            tag_cache.insert(normalized.clone(), tid);
                            tid
                        }
                        Err(e) => {
                            errors.push(format!("标签「{}」创建失败：{}", tag.name, e));
                            continue;
                        }
                    }
                }
            };
            let _ = attach_tag_conn(&mut *tx, id, tag_id).await;
            tag_names.push(name.to_string());
        }

        // 全文索引：直接用已知字段写入，省掉 `rebuild_item_fts` 的两次 SELECT
        // （旧实现每条都要把刚插入的行再读回来拼 FTS）
        let _ = update_fts_row(
            &mut *tx,
            id,
            &ExternalItem {
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
                extra: serde_json::json!({ "tag_names": tag_names }),
            },
        )
        .await;

        known.insert(key, (id, None));
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

    tx.commit().await?;

    finish_import_run(pool, run_id, imported, skipped, failed, &errors).await?;
    let result = build_import_result(pool, run_id).await?;
    Ok((result, new_items))
}

#[cfg(test)]
mod tests {
    use super::{github_owner_from_url, heal_migration_line_endings, opus_mid_to_string};
    use serde_json::json;
    use sha2::{Digest, Sha384};
    use sqlx::migrate::{Migration, MigrationType, Migrator};
    use std::borrow::Cow;

    /// 建一张只含 `version` / `checksum` 的迁移表（`heal_migration_line_endings` 只用到这两列）。
    async fn seed_migration_table(pool: &sqlx::SqlitePool) {
        sqlx::query(
            "CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, checksum BLOB NOT NULL)",
        )
        .execute(pool)
        .await
        .expect("建迁移表失败");
    }

    fn migrator_with(version: i64, sql: &str) -> Migrator {
        Migrator {
            migrations: Cow::Owned(vec![Migration::new(
                version,
                "test".into(),
                MigrationType::Simple,
                Cow::Owned(sql.to_string()),
                false,
            )]),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        }
    }

    async fn stored_checksum(pool: &sqlx::SqlitePool, version: i64) -> Vec<u8> {
        sqlx::query_as::<_, (Vec<u8>,)>("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
            .bind(version)
            .fetch_one(pool)
            .await
            .expect("读取 checksum 失败")
            .0
    }

    /// 场景：库里记的是 LF 版校验和，当前 exe 内嵌的是 CRLF 版 → 应自愈成 CRLF 版。
    /// 这正是本次本地 `cargo run` panic 的真实形态。
    #[tokio::test]
    async fn heals_checksum_when_db_has_lf_and_binary_has_crlf() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        seed_migration_table(&pool).await;

        let sql_crlf = "CREATE TABLE demo (id INTEGER PRIMARY KEY);\r\n";
        let lf_digest = Sha384::digest(sql_crlf.replace("\r\n", "\n").as_bytes()).to_vec();
        sqlx::query("INSERT INTO _sqlx_migrations (version, checksum) VALUES (?, ?)")
            .bind(1i64)
            .bind(&lf_digest)
            .execute(&pool)
            .await
            .expect("插入失败");

        let migrator = migrator_with(1, sql_crlf);
        let healed = heal_migration_line_endings(&pool, &migrator).await;

        assert_eq!(healed, 1, "应自愈 1 条");
        assert_eq!(
            stored_checksum(&pool, 1).await,
            migrator.migrations[0].checksum.as_ref(),
            "checksum 应更新为当前 exe 内嵌（CRLF）版本"
        );
    }

    /// 反方向：库里记的是 CRLF 版，当前 exe 内嵌的是 LF 版 → 同样应自愈。
    #[tokio::test]
    async fn heals_checksum_when_db_has_crlf_and_binary_has_lf() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        seed_migration_table(&pool).await;

        let sql_lf = "CREATE TABLE demo (id INTEGER PRIMARY KEY);\n";
        let crlf_digest = Sha384::digest(sql_lf.replace('\n', "\r\n").as_bytes()).to_vec();
        sqlx::query("INSERT INTO _sqlx_migrations (version, checksum) VALUES (?, ?)")
            .bind(1i64)
            .bind(&crlf_digest)
            .execute(&pool)
            .await
            .expect("插入失败");

        let migrator = migrator_with(1, sql_lf);
        let healed = heal_migration_line_endings(&pool, &migrator).await;

        assert_eq!(healed, 1, "应自愈 1 条");
        assert_eq!(
            stored_checksum(&pool, 1).await,
            migrator.migrations[0].checksum.as_ref()
        );
    }

    /// 关键安全边界：SQL 内容真被篡改时**绝不能**静默放行，
    /// 必须交还给 sqlx 原有的校验去 panic。
    #[tokio::test]
    async fn leaves_genuinely_modified_migration_alone() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        seed_migration_table(&pool).await;

        let tampered = Sha384::digest(b"DROP TABLE items;").to_vec();
        sqlx::query("INSERT INTO _sqlx_migrations (version, checksum) VALUES (?, ?)")
            .bind(1i64)
            .bind(&tampered)
            .execute(&pool)
            .await
            .expect("插入失败");

        let migrator = migrator_with(1, "CREATE TABLE demo (id INTEGER PRIMARY KEY);\n");
        let healed = heal_migration_line_endings(&pool, &migrator).await;

        assert_eq!(healed, 0, "内容篡改不得自愈");
        assert_eq!(
            stored_checksum(&pool, 1).await,
            tampered,
            "checksum 必须保持原样，让 sqlx 照常报错"
        );
    }

    /// 全新数据库没有迁移表：应静默返回 0，不能因为表不存在就阻断启动。
    #[tokio::test]
    async fn no_op_on_fresh_database_without_migration_table() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        let migrator = migrator_with(1, "CREATE TABLE demo (id INTEGER PRIMARY KEY);\n");
        assert_eq!(heal_migration_line_endings(&pool, &migrator).await, 0);
    }

    /// 校验和本就一致时不应产生任何写入。
    #[tokio::test]
    async fn no_op_when_checksums_already_match() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        seed_migration_table(&pool).await;

        let sql = "CREATE TABLE demo (id INTEGER PRIMARY KEY);\n";
        let digest = Sha384::digest(sql.as_bytes()).to_vec();
        sqlx::query("INSERT INTO _sqlx_migrations (version, checksum) VALUES (?, ?)")
            .bind(1i64)
            .bind(&digest)
            .execute(&pool)
            .await
            .expect("插入失败");

        assert_eq!(
            heal_migration_line_endings(&pool, &migrator_with(1, sql)).await,
            0
        );
        assert_eq!(stored_checksum(&pool, 1).await, digest);
    }

    #[test]
    fn parses_github_owner_from_repo_url() {
        assert_eq!(
            github_owner_from_url("https://github.com/torvalds/linux"),
            Some("torvalds".to_string())
        );
        assert_eq!(
            github_owner_from_url("http://github.com/rust-lang/rust"),
            Some("rust-lang".to_string())
        );
        // owner 含点号也正常
        assert_eq!(
            github_owner_from_url("https://github.com/foo.bar/repo"),
            Some("foo.bar".to_string())
        );
    }

    #[test]
    fn rejects_non_repo_github_urls() {
        // 裸域名（owner 后无 `/`）解析不出 owner
        assert_eq!(github_owner_from_url("https://github.com/torvalds"), None);
        // 非 github 域名
        assert_eq!(github_owner_from_url("https://gitlab.com/a/b"), None);
        assert_eq!(github_owner_from_url("not a url"), None);
    }

    #[test]
    fn parses_opus_mid_as_string_and_number() {
        // 图文收藏接口实测返回字符串形态的 mid
        assert_eq!(
            opus_mid_to_string(&json!("3824575")),
            Some("3824575".to_string())
        );
        // 兼容以后可能改为数字的情形
        assert_eq!(
            opus_mid_to_string(&json!(3824575)),
            Some("3824575".to_string())
        );
        // 缺失 / 非标量应返回 None，避免编造 id
        assert_eq!(opus_mid_to_string(&json!(null)), None);
        assert_eq!(opus_mid_to_string(&json!({ "x": 1 })), None);
    }

    /// 回归测试：`ItemRow` 的所有查询都必须覆盖结构体全部字段。
    ///
    /// `sqlx::query_as` 是**运行时按列名**解码的 —— 给 `ItemRow` 加了字段却漏改某处 SELECT 时，
    /// 编译期不会报错，只会在运行时抛 `ColumnNotFound`，表现为收藏库 / 回收站**整页空白**
    /// （曾因 `obsidian_path` 漏列真实触发）。这里把三条主要读取路径都跑一遍钉死。
    #[tokio::test]
    async fn item_row_queries_return_every_column() {
        use super::{get_item, list_trash, search_items, SqlitePoolOptions};
        use crate::models::ItemFilters;

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        sqlx::query(
            "INSERT INTO items (source, external_id, source_url, title, created_at, updated_at)
             VALUES ('bilibili', 'BV_TEST_0001', 'https://example.com/1', '测试标题', 1, 1)",
        )
        .execute(&pool)
        .await
        .expect("插入测试收藏失败");

        // 1) 收藏库列表（LibraryPage 的取数入口）
        let filters = ItemFilters {
            query: None,
            tag_ids: vec![],
            tag_mode: "and".to_string(),
            strict: false,
            untagged: false,
            sort: "favorite_desc".to_string(),
            sources: vec![],
            trash: None,
        };
        let list = search_items(&pool, &filters)
            .await
            .expect("search_items 缺列会抛 ColumnNotFound");
        assert_eq!(list.len(), 1, "收藏库应返回 1 条");

        // 2) 单条读取（Obsidian 打开 / 导出用）
        let id = list[0].id;
        get_item(&pool, id)
            .await
            .expect("get_item 缺列会抛 ColumnNotFound");

        // 3) 软删除后走回收站列表
        sqlx::query("UPDATE items SET deleted_at = 1 WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await
            .expect("软删除失败");
        let trash = list_trash(&pool)
            .await
            .expect("list_trash 缺列会抛 ColumnNotFound");
        assert_eq!(trash.len(), 1, "回收站应返回 1 条");
    }

    /// 基准：大规模数据下验证 `search_items` 仍高效（验证「批量水合」修复）。
    ///
    /// 测的是无 LIMIT + 批量 `IN (...)` 取标签的路径：一次性返回全部行，
    /// 标签分批（每批 500）一次取回，而非逐条 N+1。打印耗时，便于直观对比
    /// 「裸删 LIMIT 但保留 N+1」时 ~1 万次串行查询的秒级卡顿。
    /// 建数据（1 万 item + 标签关联）在计时外，只计时 `search_items` 本身。
    #[tokio::test]
    async fn search_items_scales_to_ten_thousand() {
        use super::{search_items, SqlitePoolOptions};
        use crate::models::ItemFilters;
        use std::time::Instant;

        // 用共享缓存内存库（cache=shared），跨连接池可见同一份数据，
        // 既贴近「单库 + 连接池」的真实场景，又避开文件路径中冒号导致的 URL 解析问题。
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:?cache=shared")
            .await
            .expect("连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        let n: usize = 10_000;
        // 建 8 个标签
        let mut tag_ids: Vec<i64> = Vec::new();
        for i in 0..8u32 {
            sqlx::query(
                "INSERT INTO tags (namespace, name, normalized, created_at) VALUES ('user', ?, ?, 1)",
            )
            .bind(format!("tag{i}"))
            .bind(format!("tag{i}"))
            .execute(&pool)
            .await
            .expect("插标签失败");
            tag_ids.push((i + 1) as i64);
        }

        // 批量插入 n 条 item（每条挂 2 个标签），单事务加速
        let mut tx = pool.begin().await.expect("开事务失败");
        for i in 0..n {
            sqlx::query(
                "INSERT INTO items (source, external_id, source_url, title, created_at, updated_at, favorite_time) \
                 VALUES ('bilibili', ?, ?, ?, 1, 1, 1)",
            )
            .bind(format!("BV{i:06}"))
            .bind(format!("https://example.com/{i}"))
            .bind(format!("标题 {i}"))
            .execute(&mut *tx)
            .await
            .expect("插 item 失败");
            let item_id = (i + 1) as i64; // 自增 id 从 1 起
                                          // 第二标签取 tag1..tag7，永远不等于第一标签 tag0，避免 (item_id,tag_id) 唯一冲突
            for t in [tag_ids[0], tag_ids[1 + (i % 7) as usize]] {
                sqlx::query("INSERT INTO item_tags (item_id, tag_id, created_at) VALUES (?, ?, 1)")
                    .bind(item_id)
                    .bind(t)
                    .execute(&mut *tx)
                    .await
                    .expect("插 item_tags 失败");
            }
        }
        tx.commit().await.expect("提交失败");

        let filters = ItemFilters {
            query: None,
            tag_ids: vec![],
            tag_mode: "and".to_string(),
            strict: false,
            untagged: false,
            sort: "favorite_desc".to_string(),
            sources: vec![],
            trash: None,
        };

        // 预热一次（建索引缓存、JIT 等），不计入
        let _ = search_items(&pool, &filters).await.expect("warmup 失败");

        // search_items 总耗时（含「全局 count 一次」+「批量 IN 取标签」+ 序列化）
        let t0 = Instant::now();
        let list = search_items(&pool, &filters)
            .await
            .expect("search_items 失败");
        let total = t0.elapsed();

        assert_eq!(list.len(), n, "应返回全部 {n} 条");
        let sample = &list[5000];
        assert_eq!(sample.tags.len(), 2, "每条应挂 2 个标签");

        eprintln!(
            "[bench] search_items({n} 条, 无 LIMIT, 全局 count + 批量水合) 总耗时: {:.2?}",
            total
        );
    }

    /// 辅助：按 strict / tag_ids 查询，返回按 id 升序排列的结果集。
    async fn run_strict_search(pool: &sqlx::SqlitePool, strict: bool, ids: Vec<i64>) -> Vec<i64> {
        use super::search_items;
        use crate::models::ItemFilters;

        let f = ItemFilters {
            query: None,
            tag_ids: ids,
            tag_mode: "and".to_string(),
            strict,
            untagged: false,
            sort: "favorite_desc".to_string(),
            sources: vec![],
            trash: None,
        };
        let list = search_items(pool, &f).await.expect("search_items 失败");
        let mut got: Vec<i64> = list.iter().map(|v| v.id).collect();
        got.sort();
        got
    }

    /// 严格匹配：item 的标签集合必须「恰好等于」输入的 tag_ids。
    /// 输入 {a} 时只返回仅含 a 的 item，过滤掉同时含 a、b 的 item；
    /// 关闭 strict（and 模式）时则返回含 a 的所有 item。
    #[tokio::test]
    async fn strict_tag_match_filters_supersets() {
        use super::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        // 三个标签 a / b / c
        let mut tag_ids: Vec<i64> = Vec::new();
        for name in ["a", "b", "c"] {
            sqlx::query(
                "INSERT INTO tags (namespace, name, normalized, created_at) VALUES ('user', ?, ?, 1)",
            )
            .bind(name)
            .bind(name)
            .execute(&pool)
            .await
            .expect("插标签失败");
            tag_ids.push(tag_ids.len() as i64 + 1);
        }
        let [a, b, c] = [tag_ids[0], tag_ids[1], tag_ids[2]];

        // item1: 仅 a；item2: a + b；item3: 仅 b；item4: a + b + c
        let cases: &[(i64, &[i64])] = &[(1, &[a]), (2, &[a, b]), (3, &[b]), (4, &[a, b, c])];
        let mut tx = pool.begin().await.expect("开事务失败");
        for (id, tgs) in cases {
            sqlx::query(
                "INSERT INTO items (id, source, external_id, source_url, title, created_at, updated_at, favorite_time) \
                 VALUES (?, 'bilibili', ?, ?, ?, 1, 1, 1)",
            )
            .bind(id)
            .bind(format!("BV{id:06}"))
            .bind(format!("https://example.com/{id}"))
            .bind(format!("标题 {id}"))
            .execute(&mut *tx)
            .await
            .expect("插 item 失败");
            for t in *tgs {
                sqlx::query("INSERT INTO item_tags (item_id, tag_id, created_at) VALUES (?, ?, 1)")
                    .bind(id)
                    .bind(t)
                    .execute(&mut *tx)
                    .await
                    .expect("插 item_tags 失败");
            }
        }
        tx.commit().await.expect("提交失败");

        // 查询走模块级 helper run_strict_search（闭包捕获 &pool 会引入生命周期错误，故改为顶层 async fn）

        // 输入 {a} 严格：仅 item1（只含 a），排除 item2(a,b)、item4(a,b,c)
        assert_eq!(
            run_strict_search(&pool, true, vec![a]).await,
            vec![1],
            "严格「a」应只返回 item1"
        );
        // 输入 {a} 非严格(and)：含 a 的所有 item → 1,2,4
        assert_eq!(
            run_strict_search(&pool, false, vec![a]).await,
            vec![1, 2, 4],
            "非严格「a」应返回 1,2,4"
        );
        // 输入 {a,b} 严格：仅 item2（恰好 a+b），排除 item4(a+b+c)
        assert_eq!(
            run_strict_search(&pool, true, vec![a, b]).await,
            vec![2],
            "严格「a、b」应只返回 item2"
        );
        // 输入 {b} 严格：仅 item3（只含 b）
        assert_eq!(
            run_strict_search(&pool, true, vec![b]).await,
            vec![3],
            "严格「b」应只返回 item3"
        );
    }

    /// 回归测试：引用既有标签不得重置其分类。
    /// 曾因 get_or_create_tag 携带 id 即全字段 UPDATE（category_id 绑成 NULL），
    /// 导致导入 / 给视频补标签时把「已分类标签」打回未分类。
    #[tokio::test]
    async fn referencing_existing_tag_preserves_its_category() {
        use super::{create_tag_category, get_or_create_tag, upsert_tag, SqlitePoolOptions};
        use crate::models::TagInput;

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        // 建分类 + 已分类标签 a
        let cat = create_tag_category(&pool, "我的分类", None)
            .await
            .expect("建分类失败");
        let tag_a = sqlx::query_scalar::<_, i64>(
            "INSERT INTO tags (namespace, name, normalized, color, category_id, created_at)
             VALUES ('user', 'a', 'a', NULL, ?, 1)
             RETURNING id",
        )
        .bind(cat.id)
        .fetch_one(&pool)
        .await
        .expect("插标签失败");

        // 模拟导入/挂接路径：spec 携带 id 但不带 category_id —— 不得清掉分类
        let reference = TagInput {
            id: Some(tag_a),
            namespace: "manual".into(),
            name: "a".into(),
            color: None,
            description: None,
            category_id: None,
        };
        let got = get_or_create_tag(&pool, &reference)
            .await
            .expect("get_or_create_tag 失败");
        assert_eq!(got, tag_a, "应复用既有标签");
        let cat_now: Option<i64> = sqlx::query_scalar("SELECT category_id FROM tags WHERE id = ?")
            .bind(tag_a)
            .fetch_one(&pool)
            .await
            .expect("查分类失败");
        assert_eq!(
            cat_now,
            Some(cat.id),
            "find-or-create 引用标签不得重置其分类"
        );

        // 显式编辑（upsert_tag 带 id 且 category_id=None）才允许移出分类
        let edited = upsert_tag(&pool, &reference)
            .await
            .expect("upsert_tag 失败");
        assert_eq!(edited.id, tag_a);
        assert_eq!(edited.category_id, None, "显式编辑可清空分类（语义不同）");
    }

    /// 无标签筛选：仅返回未挂任何标签的 item；与 tag_ids 互斥（untagged 优先）。
    #[tokio::test]
    async fn untagged_filter_returns_items_without_tags() {
        use super::{attach_tag, get_or_create_tag, search_items, SqlitePoolOptions};
        use crate::models::{ItemFilters, TagInput};

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        // 一个标签 a；item1 无标签；item2 / item3 挂 a
        let tag_a = get_or_create_tag(
            &pool,
            &TagInput {
                id: None,
                namespace: "manual".into(),
                name: "a".into(),
                color: None,
                description: None,
                category_id: None,
            },
        )
        .await
        .expect("建标签失败");
        let mut ids = Vec::new();
        for i in 1..=3i64 {
            let item_id = sqlx::query_scalar::<_, i64>(
                "INSERT INTO items (source, external_id, source_url, title, created_at, updated_at)
                 VALUES ('bilibili', ?, ?, ?, 1, 1) RETURNING id",
            )
            .bind(format!("BV{id:06}", id = i))
            .bind(format!("https://example.com/{i}"))
            .bind(format!("标题 {i}"))
            .fetch_one(&pool)
            .await
            .expect("插 item 失败");
            ids.push(item_id);
        }
        attach_tag(&pool, ids[1], tag_a).await.expect("挂标签失败");
        attach_tag(&pool, ids[2], tag_a).await.expect("挂标签失败");

        let base = |untagged: bool| ItemFilters {
            query: None,
            tag_ids: if untagged { vec![] } else { vec![tag_a] },
            tag_mode: "and".to_string(),
            strict: false,
            untagged,
            sort: "favorite_desc".to_string(),
            sources: vec![],
            trash: None,
        };
        let list = search_items(&pool, &base(true))
            .await
            .expect("search_items 失败");
        assert_eq!(list.len(), 1, "无标签筛选应只返回 item1");
        assert_eq!(list[0].id, ids[0]);
        assert_eq!(list[0].tags.len(), 0, "返回项应确实无标签");

        // untagged 优先于 tag_ids：即使携带 tag_ids 也只按无标签过滤
        let mut mixed = base(true);
        mixed.tag_ids = vec![tag_a];
        let list2 = search_items(&pool, &mixed)
            .await
            .expect("search_items 失败");
        assert_eq!(list2.len(), 1, "untagged=true 时应忽略 tag_ids");

        let list3 = search_items(&pool, &base(false))
            .await
            .expect("search_items 失败");
        assert_eq!(list3.len(), 2, "按标签 a 应返回 item2、item3");
    }

    /// 分类重排：reorder 后按 position 返回新顺序，新建分类追加到末尾。
    #[tokio::test]
    async fn reorder_tag_categories_rewrites_positions() {
        use super::{
            create_tag_category, list_tag_categories, reorder_tag_categories, SqlitePoolOptions,
        };

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        let a = create_tag_category(&pool, "甲", None)
            .await
            .expect("建分类失败");
        let b = create_tag_category(&pool, "乙", None)
            .await
            .expect("建分类失败");
        let c = create_tag_category(&pool, "丙", None)
            .await
            .expect("建分类失败");

        // 新建时 position 递增 → 初始顺序 = 创建顺序
        let names = |list: &[crate::models::TagCategory]| -> Vec<String> {
            list.iter().map(|c| c.name.clone()).collect()
        };
        let initial = list_tag_categories(&pool).await.expect("list 失败");
        assert_eq!(names(&initial), vec!["甲", "乙", "丙"], "新分类应追加末尾");

        // 重排为 丙、甲、乙
        reorder_tag_categories(&pool, &[c.id, a.id, b.id])
            .await
            .expect("reorder 失败");
        let after = list_tag_categories(&pool).await.expect("list 失败");
        assert_eq!(names(&after), vec!["丙", "甲", "乙"], "应遵循重排后的位置");
        assert_eq!(
            after.iter().map(|c| c.position).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "position 应被重写为连续序号"
        );

        // 新建分类应排到末尾（position 最大）
        let d = create_tag_category(&pool, "丁", None)
            .await
            .expect("建分类失败");
        assert_eq!(d.position, 3);
        let final_list = list_tag_categories(&pool).await.expect("list 失败");
        assert_eq!(names(&final_list), vec!["丙", "甲", "乙", "丁"]);
    }

    /// 按指定排序返回全部 item id（星标测试用）。
    async fn search_ids_sorted(pool: &sqlx::SqlitePool, sort: &str) -> Vec<i64> {
        use super::search_items;
        use crate::models::ItemFilters;

        let filters = ItemFilters {
            query: None,
            tag_ids: vec![],
            tag_mode: "and".to_string(),
            strict: false,
            untagged: false,
            sort: sort.to_string(),
            sources: vec![],
            trash: None,
        };
        let list = search_items(pool, &filters)
            .await
            .expect("search_items 失败");
        list.iter().map(|v| v.id).collect()
    }

    /// 星标置顶：任何排序下星标项排最前；导出文件携带 starred 状态。
    #[tokio::test]
    async fn starred_items_pinned_and_exported() {
        use super::{export_items, set_item_starred, SqlitePoolOptions};

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        // 三个 item：favorite_time 越大越靠前（无星时 A 最后）
        let mut ids = Vec::new();
        for (i, fav) in [(1i64, 100i64), (2, 200), (3, 300)] {
            let item_id = sqlx::query_scalar::<_, i64>(
                "INSERT INTO items (source, external_id, source_url, title, favorite_time, created_at, updated_at) \
                 VALUES ('bilibili', ?, ?, ?, ?, 1, 1) RETURNING id",
            )
            .bind(format!("BV{i:06}"))
            .bind(format!("https://example.com/{i}"))
            .bind(format!("标题 {i}"))
            .bind(fav)
            .fetch_one(&pool)
            .await
            .expect("插 item 失败");
            ids.push(item_id);
        }
        let [a, b, c] = [ids[0], ids[1], ids[2]];

        // 给收藏时间最小的两条打星（A 与 B），C 不打星
        set_item_starred(&pool, a, true).await.expect("打星失败");
        set_item_starred(&pool, b, true).await.expect("打星失败");

        // favorite_desc 下星标两条仍压过收藏时间最大的 C
        let by_fav = search_ids_sorted(&pool, "favorite_desc").await;
        assert!(
            by_fav[0] != c && by_fav[1] != c,
            "前两条应为星标项 A/B，实际 {by_fav:?}"
        );
        assert_eq!(by_fav.len(), 3);
        // title_asc 下星标同样置顶
        let by_title = search_ids_sorted(&pool, "title_asc").await;
        let first_two: Vec<i64> = by_title.iter().take(2).copied().collect();
        assert!(
            !first_two.contains(&c),
            "title_asc 星标也应置顶，实际 {first_two:?}"
        );

        // 取消星标后回到常规排序（C 最前）
        set_item_starred(&pool, a, false)
            .await
            .expect("取消打星失败");
        set_item_starred(&pool, b, false)
            .await
            .expect("取消打星失败");
        let normal = search_ids_sorted(&pool, "favorite_desc").await;
        assert_eq!(normal, vec![c, b, a], "取消星标后按 favorite_time desc");

        // 导出文件携带 starred 状态（含打星时间列）
        set_item_starred(&pool, a, true)
            .await
            .expect("重新打星失败");
        let export = export_items(&pool, None).await.expect("导出失败");
        let exported_a = export
            .items
            .iter()
            .find(|it| it.external_id == "BV000001")
            .expect("导出应有 A");
        assert!(exported_a.starred, "导出应携带 starred=true");
        assert!(exported_a.starred_at.is_some(), "导出应携带 starred_at");
    }

    /// 标签合并：源标签视频并入目标（去重），源标签被删除。
    #[tokio::test]
    async fn merge_tags_moves_items_and_removes_source() {
        use super::{attach_tag, get_or_create_tag, merge_tags, SqlitePoolOptions};
        use crate::models::TagInput;

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        let input_a = TagInput {
            id: None,
            namespace: "manual".into(),
            name: "a".to_string(),
            color: None,
            description: None,
            category_id: None,
        };
        let input_b = TagInput {
            id: None,
            namespace: "manual".into(),
            name: "b".to_string(),
            color: None,
            description: None,
            category_id: None,
        };
        let tag_a = get_or_create_tag(&pool, &input_a)
            .await
            .expect("建标签失败");
        let tag_b = get_or_create_tag(&pool, &input_b)
            .await
            .expect("建标签失败");

        // item1 同时挂 a、b；item2 只挂 a
        let mut item_ids = Vec::new();
        for i in 1..=2i64 {
            let item_id = sqlx::query_scalar::<_, i64>(
                "INSERT INTO items (source, external_id, source_url, title, created_at, updated_at) \
                 VALUES ('bilibili', ?, ?, ?, 1, 1) RETURNING id",
            )
            .bind(format!("BV{i:06}"))
            .bind(format!("https://example.com/{i}"))
            .bind(format!("标题 {i}"))
            .fetch_one(&pool)
            .await
            .expect("插 item 失败");
            item_ids.push(item_id);
        }
        attach_tag(&pool, item_ids[0], tag_a)
            .await
            .expect("挂 a 失败");
        attach_tag(&pool, item_ids[0], tag_b)
            .await
            .expect("挂 b 失败");
        attach_tag(&pool, item_ids[1], tag_a)
            .await
            .expect("挂 a 失败");

        merge_tags(&pool, tag_a, tag_b).await.expect("合并失败");

        // 源标签 a 已删除
        let a_exists: Option<i64> = sqlx::query_scalar("SELECT id FROM tags WHERE id = ?")
            .bind(tag_a)
            .fetch_optional(&pool)
            .await
            .expect("查询失败");
        assert!(a_exists.is_none(), "源标签应被删除");

        // 目标 b 名下应有 2 条（item1 原本就有 b，去重后仍只一条）
        let b_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM item_tags WHERE tag_id = ?")
            .bind(tag_b)
            .fetch_one(&pool)
            .await
            .expect("查询失败");
        assert_eq!(b_count, 2, "合并后 b 名下应有 2 条视频");
    }

    /// 分类组：合并后组连续、颜色统一为最上层成员色、成员改色整组同步、删除组长后残局正确。
    #[tokio::test]
    async fn category_groups_unify_color_and_stay_contiguous() {
        use super::{
            create_tag_category, delete_tag_category, group_tag_categories, list_tag_categories,
            rename_tag_category, SqlitePoolOptions,
        };

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        let a = create_tag_category(&pool, "甲", Some("#111111".into()))
            .await
            .expect("建分类失败");
        let _b = create_tag_category(&pool, "乙", Some("#222222".into()))
            .await
            .expect("建分类失败");
        let c = create_tag_category(&pool, "丙", Some("#333333".into()))
            .await
            .expect("建分类失败");
        let _d = create_tag_category(&pool, "丁", Some("#444444".into()))
            .await
            .expect("建分类失败");

        // 合并 甲(0) 与 丙(2)：leader=甲，颜色统一为甲色，组落在原 leader 位置且连续
        group_tag_categories(&pool, &[a.id, c.id])
            .await
            .expect("合并失败");
        let list = list_tag_categories(&pool).await.expect("list 失败");
        let seq: Vec<String> = list.iter().map(|x| x.name.clone()).collect();
        assert_eq!(
            seq,
            vec!["甲", "丙", "乙", "丁"],
            "组成员应连续于原 leader 位置"
        );
        let row_a = list.iter().find(|x| x.id == a.id).unwrap();
        let row_c = list.iter().find(|x| x.id == c.id).unwrap();
        assert_eq!(row_a.group_id, Some(a.id), "leader 自指");
        assert_eq!(row_c.group_id, Some(a.id), "成员指向 leader");
        assert_eq!(row_a.color.as_deref(), Some("#111111"));
        assert_eq!(
            row_c.color.as_deref(),
            Some("#111111"),
            "颜色统一为最上层成员色"
        );

        // 组内成员改色 → 整组同步
        rename_tag_category(&pool, c.id, "丙", Some("#ff0000".to_string()))
            .await
            .expect("改色失败");
        let list2 = list_tag_categories(&pool).await.expect("list 失败");
        let row_a2 = list2.iter().find(|x| x.id == a.id).unwrap();
        assert_eq!(row_a2.color.as_deref(), Some("#ff0000"), "改色应同步整组");

        // 删除组长后：仅剩成员(丙) → 解除分组
        delete_tag_category(&pool, a.id).await.expect("删除失败");
        let list3 = list_tag_categories(&pool).await.expect("list 失败");
        let row_c3 = list3.iter().find(|x| x.id == c.id).unwrap();
        assert_eq!(row_c3.group_id, None, "组长删除后仅剩一个成员应解组");
    }

    /// 拆分整组：任一组内成员触发 ungroup，整组成员 group_id 全部清空。
    #[tokio::test]
    async fn ungroup_tag_category_splits_whole_group() {
        use super::{
            create_tag_category, group_tag_categories, list_tag_categories, ungroup_tag_category,
            SqlitePoolOptions,
        };

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        let a = create_tag_category(&pool, "甲", None)
            .await
            .expect("建分类失败");
        let b = create_tag_category(&pool, "乙", None)
            .await
            .expect("建分类失败");
        let c = create_tag_category(&pool, "丙", None)
            .await
            .expect("建分类失败");
        group_tag_categories(&pool, &[a.id, b.id])
            .await
            .expect("合并失败");

        // 从非组长成员触发拆分
        ungroup_tag_category(&pool, b.id).await.expect("拆分失败");
        let list = list_tag_categories(&pool).await.expect("list 失败");
        let row_a = list.iter().find(|x| x.id == a.id).unwrap();
        let row_b = list.iter().find(|x| x.id == b.id).unwrap();
        assert_eq!(row_a.group_id, None, "组长也应被解除");
        assert_eq!(row_b.group_id, None, "触发成员解除");
        // 无关分类不受影响
        assert!(c.group_id.is_none());
    }

    /// 回归：`get_tag_category_by_normalized` 曾漏 `group_id` 列，导致「同名分类已存在」
    /// 这条路径（JSON 导入带 category 的标签、UI 新建重名分类）运行时直接 panic。
    #[tokio::test]
    async fn tag_category_queries_return_every_column() {
        use super::{create_tag_category, list_tag_categories, SqlitePoolOptions};

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        // 首次创建走 INSERT ... RETURNING
        let first = create_tag_category(&pool, "甲", None)
            .await
            .expect("首次创建分类失败");
        // 第二次命中「已存在」分支 → get_tag_category_by_normalized，这里曾因漏列 panic
        let second = create_tag_category(&pool, "甲", None)
            .await
            .expect("重复创建分类应复用已有分类");
        assert_eq!(first.id, second.id, "应复用同一分类");
        assert_eq!(second.group_id, None, "group_id 应可解码");

        let list = list_tag_categories(&pool)
            .await
            .expect("list_tag_categories 缺列会抛 ColumnNotFound");
        assert_eq!(list.len(), 1);
    }

    /// 文件导入的语义回归：标签 / 分类 / FTS / 批注 / 星标都要落对，
    /// 且重复导入必须幂等（跳过而非覆盖）。导入路径做过「单事务 + 缓存」重写，
    /// 这个测试用来钉住语义没有悄悄改变。
    #[tokio::test]
    async fn import_collection_writes_tags_categories_and_fts() {
        use super::{import_collection, search_items, SqlitePoolOptions};
        use crate::models::{CollectionExport, ExportItem, ExportTag, ItemFilters};

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        let tag = |name: &str, category: Option<&str>| ExportTag {
            namespace: "manual".into(),
            name: name.into(),
            color: None,
            category: category.map(str::to_string),
        };
        let export = CollectionExport {
            format_version: 1,
            exported_at: 1,
            app: "collectorlite".into(),
            items: vec![
                ExportItem {
                    source: "bilibili".into(),
                    external_id: "BV1".into(),
                    source_url: "https://example.com/1".into(),
                    title: "深入异步运行时".into(),
                    description: "简介".into(),
                    cover_url: None,
                    author_name: Some("UP主".into()),
                    author_id: None,
                    partition_name: None,
                    published_at: None,
                    duration: Some(600),
                    favorite_time: Some(100),
                    notes: "我的批注".into(),
                    obsidian_path: Some("收藏/深入异步运行时.md".into()),
                    starred: true,
                    starred_at: Some(200),
                    extra: serde_json::json!({ "k": "v" }),
                    tags: vec![tag("前端", Some("技术")), tag("必看", Some("精选"))],
                },
                ExportItem {
                    source: "bilibili".into(),
                    external_id: "BV2".into(),
                    source_url: "https://example.com/2".into(),
                    title: "另一条".into(),
                    description: String::new(),
                    cover_url: None,
                    author_name: None,
                    author_id: None,
                    partition_name: None,
                    published_at: None,
                    duration: None,
                    favorite_time: None,
                    notes: String::new(),
                    obsidian_path: None,
                    starred: false,
                    starred_at: None,
                    extra: serde_json::Value::Null,
                    // 同名标签不带分类：必须复用已有标签，且其分类不能被清掉
                    tags: vec![tag("前端", None)],
                },
                ExportItem {
                    source: "zhihu".into(),
                    external_id: "Z1".into(),
                    source_url: "https://example.com/3".into(),
                    title: "无标签条目".into(),
                    description: String::new(),
                    cover_url: None,
                    author_name: None,
                    author_id: None,
                    partition_name: None,
                    published_at: None,
                    duration: None,
                    favorite_time: None,
                    notes: String::new(),
                    obsidian_path: None,
                    starred: false,
                    starred_at: None,
                    extra: serde_json::Value::Null,
                    tags: vec![],
                },
            ],
        };
        let payload = serde_json::to_string(&export).expect("序列化失败");

        let (result, new_items) = import_collection(&pool, &payload).await.expect("导入失败");
        assert_eq!((result.imported, result.skipped, result.failed), (3, 0, 0));
        assert_eq!(new_items.len(), 3, "新项清单应包含 3 条");

        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items")
            .fetch_one(&pool)
            .await
            .expect("计数失败");
        assert_eq!(total, 3);

        // 批注 / 星标 / obsidian_path 都要原样落库
        let (notes, starred, obsidian): (String, bool, Option<String>) = sqlx::query_as(
            "SELECT notes, starred, obsidian_path FROM items WHERE external_id = 'BV1'",
        )
        .fetch_one(&pool)
        .await
        .expect("读取 BV1 失败");
        assert_eq!(notes, "我的批注");
        assert!(starred, "星标状态应保留");
        assert_eq!(obsidian.as_deref(), Some("收藏/深入异步运行时.md"));

        // 标签：两个标签各建一次，分类归属正确
        for (name, category) in [("前端", "技术"), ("必看", "精选")] {
            let (count, cat): (i64, Option<String>) = sqlx::query_as(
                "SELECT COUNT(*), (SELECT tc.name FROM tag_categories tc WHERE tc.id = t.category_id)
                 FROM tags t WHERE t.name = ?",
            )
            .bind(name)
            .fetch_one(&pool)
            .await
            .expect("读标签失败");
            assert_eq!(count, 1, "标签「{name}」应只建一次");
            assert_eq!(cat.as_deref(), Some(category), "标签「{name}」分类不对");
        }
        let linked: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM item_tags it JOIN items i ON i.id = it.item_id WHERE i.external_id = 'BV1'",
        )
        .fetch_one(&pool)
        .await
        .expect("统计关联失败");
        assert_eq!(linked, 2, "BV1 应挂 2 个标签");

        // FTS：每条都要有索引行，否则应用内搜不到
        let fts_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items_fts")
            .fetch_one(&pool)
            .await
            .expect("统计 FTS 失败");
        assert_eq!(fts_rows, 3, "每条收藏都应有 FTS 行");

        // 搜索（标题 / 标签名）
        let filters = |query: &str| ItemFilters {
            query: Some(query.into()),
            tag_ids: vec![],
            tag_mode: "and".into(),
            strict: false,
            untagged: false,
            sort: "favorite_desc".into(),
            sources: vec![],
            trash: None,
        };
        let hit = search_items(&pool, &filters("运行时"))
            .await
            .expect("标题搜索失败");
        assert_eq!(hit.len(), 1, "标题应能搜到");
        let hit = search_items(&pool, &filters("必看"))
            .await
            .expect("标签搜索失败");
        assert_eq!(hit.len(), 1, "标签名应能搜到");

        // 重复导入：全部跳过，不覆盖、不重复
        let (again, new_again) = import_collection(&pool, &payload)
            .await
            .expect("二次导入失败");
        assert_eq!((again.imported, again.skipped, again.failed), (0, 3, 0));
        assert!(new_again.is_empty());
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items")
            .fetch_one(&pool)
            .await
            .expect("计数失败");
        assert_eq!(total, 3, "重复导入不应新增");
    }

    /// 封面缓存队列的定义：有 cover_url 但 cover_local_path 为空 = 待缓存。
    /// 计数与取任务共用 `PENDING_COVER_WHERE`，写回路径后必须自动出队，
    /// 否则「断点续传」会变成每次启动都重复下载整库封面。
    #[tokio::test]
    async fn pending_cover_queue_shrinks_after_writeback() {
        use super::{
            count_pending_cover_cache, fetch_items_needing_cover_cache,
            set_items_cover_local_paths, SqlitePoolOptions,
        };

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        let insert = |source: &str, external_id: &str, cover_url: Option<&str>| {
            let source = source.to_string();
            let external_id = external_id.to_string();
            let cover_url = cover_url.map(str::to_string);
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO items (source, external_id, source_url, title, description, cover_url, created_at, updated_at)
                     VALUES (?, ?, '', 't', '', ?, 1, 1)",
                )
                .bind(source)
                .bind(external_id)
                .bind(cover_url)
                .execute(&pool)
                .await
                .expect("插入失败");
            }
        };

        insert("bilibili", "BV1", Some("http://cover/1.jpg")).await;
        insert("bilibili", "BV2", Some("http://cover/2.jpg")).await;
        // 知乎 / 浏览器 / GitHub 现在也进队列 —— 历史版本注释「走远程 https 封面、不需要本地缓存」
        // 是基于「WebView 能直接加载」的错误假设，实际上 WebView 默认不继承 app 代理，
        // 远程 favicon / 部分 https 封面加载会失败，必须落本地。改 PENDING_COVER_WHERE 时同步。
        insert("zhihu", "Z1", Some("https://cover/3.jpg")).await;
        insert("browser", "B1", Some("https://favicon/4.ico")).await;
        insert("github", "G1", Some("https://avatars/5.jpg")).await;
        // 没有封面的项不该进队列
        insert("bilibili", "BV3", None).await;

        assert_eq!(
            count_pending_cover_cache(&pool).await.expect("计数失败"),
            5,
            "所有有 cover_url 的项都该进队列（bilibili/zhihu/browser/github 四个源）"
        );
        let pending = fetch_items_needing_cover_cache(&pool)
            .await
            .expect("取队列失败");
        assert_eq!(pending.len(), 5, "计数与取任务必须用同一套条件");

        // 写回本地路径后自动出队
        set_items_cover_local_paths(
            &pool,
            &[("bilibili".into(), "BV1".into(), "C:/covers/1.jpg".into())],
        )
        .await
        .expect("回写失败");
        assert_eq!(
            count_pending_cover_cache(&pool).await.expect("计数失败"),
            4,
            "已缓存的项应出队"
        );
    }

    /// 回收站里的条目用备份文件重新导入时应当被「恢复」，而不是被判为已存在而跳过。
    #[tokio::test]
    async fn import_collection_restores_trashed_items() {
        use super::{import_collection, soft_delete_item, SqlitePoolOptions};
        use crate::models::{CollectionExport, ExportItem};

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        let export = CollectionExport {
            format_version: 1,
            exported_at: 1,
            app: "collectorlite".into(),
            items: vec![ExportItem {
                source: "bilibili".into(),
                external_id: "BV1".into(),
                source_url: "https://example.com/1".into(),
                title: "被误删的收藏".into(),
                description: String::new(),
                cover_url: None,
                author_name: None,
                author_id: None,
                partition_name: None,
                published_at: None,
                duration: None,
                favorite_time: None,
                notes: String::new(),
                obsidian_path: None,
                starred: false,
                starred_at: None,
                extra: serde_json::Value::Null,
                tags: vec![],
            }],
        };
        let payload = serde_json::to_string(&export).expect("序列化失败");

        let (first, _) = import_collection(&pool, &payload)
            .await
            .expect("首次导入失败");
        assert_eq!(first.imported, 1);

        let id: i64 = sqlx::query_scalar("SELECT id FROM items WHERE external_id = 'BV1'")
            .fetch_one(&pool)
            .await
            .expect("查 id 失败");
        soft_delete_item(&pool, id).await.expect("软删除失败");

        // 重新导入：应恢复而不是跳过
        let (second, _) = import_collection(&pool, &payload)
            .await
            .expect("二次导入失败");
        assert_eq!(
            (second.imported, second.skipped, second.failed),
            (1, 0, 0),
            "回收站条目应被恢复"
        );
        let deleted: Option<i64> = sqlx::query_scalar("SELECT deleted_at FROM items WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("查 deleted_at 失败");
        assert!(deleted.is_none(), "恢复后 deleted_at 应为 NULL");
        let fts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items_fts WHERE rowid = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("查 FTS 失败");
        assert_eq!(fts, 1, "恢复后应重建 FTS 行");
    }

    /// 3000 条收藏的文件导入基准（默认 #[ignore]，手动跑：
    /// `cargo test --lib bench_import_collection_3000 -- --ignored --nocapture`）
    ///
    /// 用真实文件库（非内存）以反映 WAL + `synchronous=FULL` 下的 fsync 成本。
    #[tokio::test]
    #[ignore]
    async fn bench_import_collection_3000() {
        use crate::models::{CollectionExport, ExportItem, ExportTag};

        const N: usize = 3000;

        let dir = std::env::temp_dir().join(format!("bench_import_{}", super::now_seconds()));
        std::fs::create_dir_all(&dir).expect("建临时目录失败");
        let db_path = dir.join("bench.sqlite3");
        let pool = super::connect(&db_path).await.expect("connect 失败");

        let sources = ["bilibili", "zhihu", "browser", "csdn", "github"];
        let mut items = Vec::with_capacity(N);
        for i in 0..N {
            let source = sources[i % sources.len()];
            items.push(ExportItem {
                source: source.to_string(),
                external_id: format!("bench_{i}"),
                source_url: format!("https://example.com/{source}/{i}"),
                title: format!("基准测试条目 {i} 一个不算太短的标题"),
                description: "用于性能评估的描述文本，长度接近真实收藏的简介。".to_string(),
                cover_url: Some(format!("https://i0.hdslb.com/bfs/{i}.jpg")),
                author_name: Some(format!("作者 {i}")),
                author_id: Some(format!("{i}")),
                partition_name: Some("知识".to_string()),
                published_at: Some(1_700_000_000 + i as i64),
                duration: Some(600),
                favorite_time: Some(1_700_000_000 + i as i64),
                notes: String::new(),
                obsidian_path: None,
                starred: false,
                starred_at: None,
                extra: serde_json::json!({ "folder_tags": ["默认"] }),
                tags: vec![
                    ExportTag {
                        namespace: "manual".into(),
                        name: format!("标签{}", i % 40),
                        color: None,
                        category: None,
                    },
                    ExportTag {
                        namespace: "manual".into(),
                        name: format!("分类{}", i % 7),
                        color: None,
                        category: Some(format!("大类{}", i % 7)),
                    },
                ],
            });
        }
        let export = CollectionExport {
            format_version: 1,
            exported_at: super::now_seconds(),
            app: "collectorlite".into(),
            items,
        };

        let t_payload = std::time::Instant::now();
        let payload = serde_json::to_string(&export).expect("序列化失败");
        let payload_ms = t_payload.elapsed().as_millis();
        let payload_kb = payload.len() / 1024;

        // ① 全新库导入（全部为新增）
        let t0 = std::time::Instant::now();
        let (result, new_items) = super::import_collection(&pool, &payload)
            .await
            .expect("导入失败");
        let first_ms = t0.elapsed().as_millis();

        // ② 重复导入（全部命中跳过路径）
        let t1 = std::time::Instant::now();
        let (result2, new_items2) = super::import_collection(&pool, &payload)
            .await
            .expect("二次导入失败");
        let second_ms = t1.elapsed().as_millis();

        // ③ 诊断：把 3000 条拆成各阶段，看耗时到底花在哪
        let db3 = dir.join("bench_diag.sqlite3");
        let pool3 = super::connect(&db3).await.expect("connect3 失败");
        let insert_sql = "INSERT INTO items (source, external_id, source_url, title, description,
                          created_at, updated_at) VALUES (?, ?, '', ?, '', 1, 1)";

        // A) 3000 次裸 INSERT（每条一个自动提交事务）
        let t = std::time::Instant::now();
        for i in 0..N {
            sqlx::query(insert_sql)
                .bind("diag")
                .bind(format!("a_{i}"))
                .bind(format!("标题 {i}"))
                .execute(&pool3)
                .await
                .expect("A 插入失败");
        }
        let a_ms = t.elapsed().as_millis();

        // B) 同样 3000 次 INSERT，但包进一个事务
        let t = std::time::Instant::now();
        {
            let mut tx = pool3.begin().await.expect("begin 失败");
            for i in 0..N {
                sqlx::query(insert_sql)
                    .bind("diag")
                    .bind(format!("b_{i}"))
                    .bind(format!("标题 {i}"))
                    .execute(&mut *tx)
                    .await
                    .expect("B 插入失败");
            }
            tx.commit().await.expect("commit 失败");
        }
        let b_ms = t.elapsed().as_millis();

        // C) 3000 次 FTS 写入（DELETE + INSERT）
        let t = std::time::Instant::now();
        {
            let mut tx = pool3.begin().await.expect("begin 失败");
            for i in 1..=N as i64 {
                sqlx::query("DELETE FROM items_fts WHERE rowid = ?")
                    .bind(i)
                    .execute(&mut *tx)
                    .await
                    .expect("FTS delete 失败");
                sqlx::query(
                    "INSERT INTO items_fts (rowid, title, description, author_name, partition_name, tags)
                     VALUES (?, ?, '', '', '', '')",
                )
                .bind(i)
                .bind(format!("标题 {i}"))
                .execute(&mut *tx)
                .await
                .expect("FTS insert 失败");
            }
            tx.commit().await.expect("commit 失败");
        }
        let c_ms = t.elapsed().as_millis();

        // D) 3000 次「存在性 SELECT」
        let t = std::time::Instant::now();
        for i in 0..N {
            let _: Option<(i64, Option<i64>)> = sqlx::query_as(
                "SELECT id, deleted_at FROM items WHERE source = ? AND external_id = ?",
            )
            .bind("diag")
            .bind(format!("a_{i}"))
            .fetch_optional(&pool3)
            .await
            .expect("D 查询失败");
        }
        let d_ms = t.elapsed().as_millis();

        println!("\n=== 文件导入基准：{N} 条 ===");
        println!("payload: {payload_kb} KB，序列化 {payload_ms} ms");
        println!(
            "① 全新导入: {first_ms} ms  (导入 {} / 跳过 {} / 失败 {}，新项 {})",
            result.imported,
            result.skipped,
            result.failed,
            new_items.len()
        );
        println!(
            "② 重复导入: {second_ms} ms  (导入 {} / 跳过 {} / 失败 {}，新项 {})",
            result2.imported,
            result2.skipped,
            result2.failed,
            new_items2.len()
        );
        println!("--- 阶段拆解（各 3000 次）---");
        println!("A 裸 INSERT（自动提交）: {a_ms} ms");
        println!("B 事务内 INSERT      : {b_ms} ms");
        println!("C FTS 写入（事务内）  : {c_ms} ms");
        println!("D 存在性 SELECT      : {d_ms} ms");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 网易云增量同步的数据路径 ────────────────────────────────────────
    //
    // 同步的**判定逻辑**是纯函数、在 netease.rs 里已有单测；这里补的是**数据路径**：
    // 入库 → 判定取消收藏 → 批量软删 → 回收站 → 重新收藏后恢复。
    //
    // 必须钉死的理由：`items_fts` 是 contentless FTS5、**没有触发器**，
    // 漏删 / 漏建 FTS 时 `cargo check` 照过，运行时才表现为「搜索还能搜到已删的」
    // 或「恢复后搜不到」—— 正是本项目踩过三次的那类运行时才炸的坑。

    #[allow(unused_imports)]
    use super::{
        import_collection, list_netease_active_items, list_trash, search_items,
        soft_delete_items_bulk, CollectionExport, ExportItem, ItemFilters, SqlitePool,
        SqlitePoolOptions,
    };

    fn netease_export(playlist_id: &str) -> String {
        let items = [("100", "下等马"), ("200", "鼓楼")]
            .iter()
            .map(|(external_id, title)| ExportItem {
                source: "netease".into(),
                external_id: (*external_id).into(),
                source_url: format!("https://music.163.com/#/song?id={external_id}"),
                title: (*title).into(),
                description: "测试专辑".into(),
                cover_url: None,
                author_name: Some("测试歌手".into()),
                author_id: Some("1".into()),
                partition_name: None,
                published_at: Some(1),
                duration: Some(186),
                favorite_time: Some(1757570000),
                notes: String::new(),
                obsidian_path: None,
                starred: false,
                starred_at: None,
                extra: serde_json::json!({ "kind": "song", "playlistId": playlist_id }),
                tags: Vec::new(),
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&CollectionExport {
            format_version: 1,
            exported_at: 1,
            app: "collectorlite".into(),
            items,
        })
        .expect("组装导出数据失败")
    }

    fn build_filters(query: Option<&str>) -> ItemFilters {
        ItemFilters {
            query: query.map(|q| q.to_string()),
            tag_ids: Vec::new(),
            tag_mode: "or".into(),
            strict: false,
            untagged: false,
            sort: "favorite_time".into(),
            sources: Vec::new(),
            trash: None,
        }
    }

    #[tokio::test]
    async fn netease_sync_soft_delete_and_restore_roundtrip() {
        use crate::source::netease::{extra_playlist_id, pick_unfavorited};

        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("迁移失败");

        // ① 两条 netease 曲目入库
        let payload = netease_export("A");
        let (result, _) = import_collection(&pool, &payload).await.expect("导入失败");
        assert_eq!(result.imported, 2);

        // ② 能列出来，且能读出归属歌单（同步靠它判定要不要删）
        let rows: Vec<(i64, String, Option<String>)> =
            list_netease_active_items(&pool)
                .await
                .expect("查询失败")
                .into_iter()
                .map(|(id, external_id, extra)| {
                    (id, external_id, extra_playlist_id(&extra))
                })
                .collect();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter().all(|(_, _, p)| p.as_deref() == Some("A")),
            "extra 里应能读出 playlistId"
        );

        // ③ 云端只剩 100 —— 200 被用户取消收藏
        let synced: std::collections::HashSet<String> =
            ["A".to_string()].into_iter().collect();
        let remote: std::collections::HashSet<String> =
            ["100".to_string()].into_iter().collect();
        let doomed = pick_unfavorited(&rows, &synced, &remote);
        assert_eq!(doomed.len(), 1, "只应挑出被取消收藏的那一条");

        // ④ 批量软删：列表少一条，且 **items_fts 行必须一起清掉**
        let removed = soft_delete_items_bulk(&pool, &doomed)
            .await
            .expect("批量软删失败");
        assert_eq!(removed, 1);
        let rows = list_netease_active_items(&pool).await.expect("查询失败");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "100");
        assert_eq!(list_trash(&pool).await.expect("回收站查询失败").len(), 1);
        // ⚠️ 必须**直接查 items_fts**：contentless FTS5 没有触发器，不手动删就永远留着。
        // 用 search_items 间接验证是**无效的** —— 它的主查询先按 deleted_at IS NULL 过滤，
        // 所以 FTS 有没有残留都搜不出来（一开始写成那样，变异测试直接把它洗白了）。
        // 残留行的真实危害是 rowid 复用：SQLite 会把释放出来的 rowid 再分配给新插入的
        // 条目，于是新条目被旧关键词错误索引。
        assert_eq!(
            fts_row_exists(&pool, doomed[0]).await,
            false,
            "软删必须手动删 items_fts 行，否则 rowid 复用时会污染新条目"
        );
        // 顺带确认它确实不在正常搜索结果里（行为层面，与上面的物理断言互补）
        let found = search_items(&pool, &build_filters(Some("鼓楼")))
            .await
            .expect("搜索失败");
        assert!(found.is_empty(), "软删后不应出现在收藏库搜索结果里");

        // ⑤ 进了回收站，还能恢复
        let trash = list_trash(&pool).await.expect("回收站查询失败");
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].external_id, "200");

        // ⑥ 用户又重新收藏 → 再导一次应恢复（deleted_at 归 NULL + FTS 重建）
        let (result, _) = import_collection(&pool, &payload).await.expect("再导入失败");
        assert_eq!(result.imported, 1, "只应恢复被删的那条，已存在的不重复计");
        let rows = list_netease_active_items(&pool).await.expect("查询失败");
        assert_eq!(rows.len(), 2);
        // FTS 必须**重建**：同样是 contentless FTS5 的手动维护义务
        assert_eq!(
            fts_row_exists(&pool, doomed[0]).await,
            true,
            "恢复必须重建 items_fts 行，否则新恢复的条目搜不到"
        );
        let found = search_items(&pool, &build_filters(Some("鼓楼")))
            .await
            .expect("搜索失败");
        assert_eq!(found.len(), 1, "恢复后应能重新搜到");
    }

    /// 直接查 `items_fts` 有没有某个 rowid —— 用来验证「contentless FTS5 必须手动维护」。
    async fn fts_row_exists(pool: &SqlitePool, rowid: i64) -> bool {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items_fts WHERE rowid = ?")
            .bind(rowid)
            .fetch_one(pool)
            .await
            .expect("查 items_fts 失败");
        count > 0
    }
}
