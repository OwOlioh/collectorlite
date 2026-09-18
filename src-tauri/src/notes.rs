//! 批注（notes）的**唯一写入入口**。
//!
//! 背景：批注有两个写入方 —— app 内的 `update_item_notes` 命令，和浏览器扩展侧边栏
//! 经本地桥打过来的 `/note`。两边都要「写库 → 按需同步 Obsidian → 回写 obsidian_path」，
//! 一旦各写一份就必然漂移（见 DEVELOPMENT.md 3.16 的同类教训），所以抽到这里共用。
//!
//! 顺序至关重要：**先写库，再同步文件**。同步失败只记日志，绝不能影响批注已保存——
//! 这是整个功能的健壮性底线（DEVELOPMENT.md 7.5）。

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::db;
use crate::error::AppError;
use crate::models::VideoItem;
use crate::obsidian;
use crate::state::AppState;

/// 保存批注。
///
/// 1. 写 `items.notes`（必须成功，失败直接返回错误）
/// 2. 满足「开关开启 + vault 已配置 + 批注非空 + **已关联笔记文件**」时同步到 Obsidian（失败只告警）
/// 3. 回写 `items.obsidian_path`，并刷新双向同步的 base 快照
pub async fn save_notes(
    state: &AppState,
    item_id: i64,
    notes: &str,
) -> Result<VideoItem, AppError> {
    let mut item = db::update_item_notes(&state.pool, item_id, notes).await?;
    sync_to_obsidian(state, &mut item, notes).await;
    Ok(item)
}

/// 同步到 Obsidian（尽力而为，任何失败都不向上传播）。
///
/// ⚠️ **只同步已关联的条目**：`item.obsidian_path` 为 None 时直接跳过，绝不替用户建文件。
/// 以前少了这一条，导致「开关一开 + 写下第一行笔记」就自动在 vault 里生成一篇笔记，
/// 等于替用户做了「要不要建笔记」的决定。现在建/关联都必须显式操作
/// （侧边栏「新建笔记」/「关联已有」二选一，`POST /obsidian/create` 与 `/obsidian/link`）。
async fn sync_to_obsidian(state: &AppState, item: &mut VideoItem, notes: &str) {
    let settings = obsidian::load_settings(&state.data_dir);
    let should_sync = settings.enabled
        && !settings.vault_path.is_empty()
        && !notes.trim().is_empty()
        // 关键：没有关联文件 = 用户还没做选择，此时只写库，不碰 vault。
        && item.obsidian_path.is_some();
    if !should_sync {
        return;
    }
    match obsidian::write_or_update_note(&settings, item) {
        Ok(rel) => {
            // 路径回写失败不影响批注已保存，只是下次同步会重新解析文件名。
            let path_written = db::set_item_obsidian_path(&state.pool, item.id, &rel).await;
            match path_written {
                Ok(()) => item.obsidian_path = Some(rel.clone()),
                Err(error) => {
                    eprintln!("[obsidian] 回写 obsidian_path 失败（批注已保存）：{error}")
                }
            }
            // 刷新 base 快照：刚推下去的内容就是今后判定「谁改了」的基准。
            // 少了这一步，双向同步会把「app 自己刚写的内容」误判成「用户在 Obsidian 里改的」。
            if let Err(error) = record_sync_snapshot(state, &settings, item.id, &rel, notes).await {
                eprintln!("[obsidian] 写入同步快照失败（批注已保存）：{error}");
            }
        }
        Err(error) => {
            eprintln!("[obsidian] 同步失败（批注已保存）：{error}");
        }
    }
}

/// 记录一次成功推送后的同步快照（供 `obsidian_sync` 的三方比对使用）。
///
/// `mtime` / `size` 只是廉价快筛用，取不到也无妨 —— 真正拍板的是 `base_hash`。
async fn record_sync_snapshot(
    state: &AppState,
    settings: &obsidian::ObsidianSettings,
    item_id: i64,
    rel: &str,
    notes: &str,
) -> Result<(), AppError> {
    let abs = Path::new(&settings.vault_path).join(rel);
    let (file_mtime, file_size) = match fs::metadata(&abs) {
        Ok(meta) => (
            meta.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
            Some(meta.len() as i64),
        ),
        Err(_) => (None, None),
    };
    db::upsert_obsidian_sync_state(
        &state.pool,
        item_id,
        rel,
        // 必须与拉取时读到的内容同形态：笔记写进文件后首尾换行会被 trim 掉。
        &obsidian::hash_notes(notes.trim()),
        file_mtime,
        file_size,
    )
    .await
}
