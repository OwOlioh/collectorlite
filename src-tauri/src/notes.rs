//! 批注（notes）的**唯一写入入口**。
//!
//! 背景：批注有两个写入方 —— app 内的 `update_item_notes` 命令，和浏览器扩展侧边栏
//! 经本地桥打过来的 `/note`。两边都要「写库 → 按需同步 Obsidian → 回写 obsidian_path」，
//! 一旦各写一份就必然漂移（见 DEVELOPMENT.md 3.16 的同类教训），所以抽到这里共用。
//!
//! 顺序至关重要：**先写库，再同步文件**。同步失败只记日志，绝不能影响批注已保存——
//! 这是整个功能的健壮性底线（DEVELOPMENT.md 7.5）。

use crate::db;
use crate::error::AppError;
use crate::models::VideoItem;
use crate::obsidian;
use crate::state::AppState;

/// 保存批注。
///
/// 1. 写 `items.notes`（必须成功，失败直接返回错误）
/// 2. 满足「开关开启 + vault 已配置 + 批注非空」时单向同步到 Obsidian（失败只告警）
/// 3. 回写 `items.obsidian_path`
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
async fn sync_to_obsidian(state: &AppState, item: &mut VideoItem, notes: &str) {
    let settings = obsidian::load_settings(&state.data_dir);
    let should_sync =
        settings.enabled && !settings.vault_path.is_empty() && !notes.trim().is_empty();
    if !should_sync {
        return;
    }
    match obsidian::write_or_update_note(&settings, item) {
        Ok(Some(rel)) => {
            // 路径回写失败不影响批注已保存，只是下次同步会重新解析文件名。
            match db::set_item_obsidian_path(&state.pool, item.id, &rel).await {
                Ok(()) => item.obsidian_path = Some(rel),
                Err(error) => {
                    eprintln!("[obsidian] 回写 obsidian_path 失败（批注已保存）：{error}")
                }
            }
        }
        Ok(None) => {
            // 托管标记被用户手动移除：跳过同步，不覆盖用户在 Obsidian 里的内容。
            eprintln!("[obsidian] 跳过同步：托管标记已被移除 (item {})", item.id);
        }
        Err(error) => {
            eprintln!("[obsidian] 同步失败（批注已保存）：{error}");
        }
    }
}
