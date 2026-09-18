//! Obsidian **双向**同步引擎。
//!
//! 单向同步（app 写笔记 → 推送到 vault）已有实现，见 `notes.rs`；这里补的是反方向
//! —— 用户在 Obsidian 里改了笔记，app 要能把改动读回来。
//!
//! ## 为什么必须三方比对
//!
//! 只拿「文件内容」和「数据库内容」两两比对是判不出方向的：两者不同时，既可能是用户在
//! Obsidian 改了，也可能是 app 侧改了。所以必须额外保存一个「上次同步时」的快照 base，
//! 形成三方判定（git merge 的同款思路）：
//!
//! | 文件 vs base | 库 vs base | 结论         | 动作           |
//! |--------------|-----------|--------------|----------------|
//! | 同           | 同        | 无变化       | 跳过           |
//! | 同           | 异        | 只有 app 改了     | 推送 → 文件    |
//! | 异           | 同        | 只有 Obsidian 改了 | 拉取 → 库      |
//! | 异           | 异        | 两边都改了   | 冲突，按策略裁决 |
//!
//! ## 为什么是轮询而不是文件系统监听
//!
//! Obsidian（以及多数现代编辑器）保存的实际动作是「写临时文件 → 删原文件 → rename」，
//! 监听器收到的是一串事件，还可能在写一半时就触发，要去抖。轮询逻辑简单、跨平台一致、
//! 不怕编辑器的保存花活。代价只是最多延迟一个周期，而这可以用「关键时刻额外查一次」缓解。
//!
//! ## 为什么轮询不会把自己搞死
//!
//! 每轮同步结束都会把 `base_hash` 更新成刚刚写入的内容。下一轮比对时发现
//! 「当前文件 == base」即判定无变化，自然收敛 —— 不需要任何「忽略自身写入」的标志位。
//! 前提是**写入格式与读取格式严格互逆**（见 obsidian.rs 里那条经历了 bug 的血泪注释）。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::db;
use crate::error::AppError;
use crate::obsidian;
use crate::state::AppState;

/// 默认轮询间隔（秒）。设置文件里可覆盖。
pub const DEFAULT_INTERVAL_SECS: u64 = 30;
/// 启动后多久开始第一轮：别和首屏渲染、封面断点续传抢 IO。
const STARTUP_DELAY_SECS: u64 = 15;
/// 租约超过这么久没心跳就认为持有者已经死了（可能崩了或没优雅退出）。
const LEASE_STALE_SECS: i64 = 120;
/// 留痕副本放在 vault 根的这个子目录里。点开头目录在 Obsidian 中默认不显示，
/// 不会污染用户的笔记树。
const CONFLICT_DIR: &str = ".collector-conflicts";

/// 一轮同步的结果。前端据此决定是否弹提示。
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    /// 文件 → app（用户在 Obsidian 里改了）
    pub pulled: usize,
    /// app → 文件（app 侧笔记更新了）
    pub pushed: usize,
    /// 两边都改了，按「Obsidian 优先」裁决
    pub conflicts: usize,
    /// 笔记文件在 vault 里被删了，已按约定取消关联
    pub unlinked: usize,
}

impl SyncReport {
    fn has_changes(&self) -> bool {
        self.pulled + self.pushed + self.conflicts + self.unlinked > 0
    }
}

/// 托管区正文的哈希值。刻意复用 `obsidian::hash_notes` —— 推送方与拉取方必须算出
/// 完全一致的 base，否则两边会永远互相认为对方改了。
fn hash_text(text: &str) -> String {
    obsidian::hash_notes(text)
}

fn unix_secs(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

/// 一轮完整同步。返回 `Ok(报告)`；单个文件出错只记日志，不影响其他条目继续同步。
///
/// 这是「尽力而为」的后台任务：任何意外都不应该让整个过程失败。
pub async fn sync_once(state: &AppState) -> Result<SyncReport, AppError> {
    let mut report = SyncReport::default();
    let settings = obsidian::load_settings(&state.data_dir);
    if !settings.enabled || settings.vault_path.is_empty() {
        return Ok(report);
    }
    let vault = PathBuf::from(&settings.vault_path);
    if !vault.is_dir() {
        return Ok(report);
    }

    let links = db::list_obsidian_links(&state.pool).await?;
    for link in links {
        let Some(rel) = link.obsidian_path.clone() else {
            continue;
        };
        match sync_one_link(state, &vault, &link, &rel).await {
            Ok(outcome) => count_outcome(&mut report, outcome),
            Err(error) => {
                eprintln!("[obsidian-sync] 条目 {} 同步失败：{error}", link.id);
            }
        }
    }
    Ok(report)
}

/// 单个条目的同步结果（内部用）。
enum Outcome {
    Skip,
    Push,
    Pull,
    Conflict,
    Unlink,
}

fn count_outcome(report: &mut SyncReport, outcome: Outcome) {
    match outcome {
        Outcome::Skip => {}
        Outcome::Push => report.pushed += 1,
        Outcome::Pull => report.pulled += 1,
        Outcome::Conflict => report.conflicts += 1,
        Outcome::Unlink => report.unlinked += 1,
    }
}

async fn sync_one_link(
    state: &AppState,
    vault: &Path,
    link: &db::ObsidianLinkRow,
    rel: &str,
) -> Result<Outcome, AppError> {
    let abs = vault.join(rel);

    // 用户在 Obsidian 里删了文件 / 移走了（改名）→ 按约定取消关联，绝不静默重建。
    if !abs.exists() {
        db::unlink_obsidian_note(&state.pool, link.id).await?;
        return Ok(Outcome::Unlink);
    }

    let meta = fs::metadata(&abs).map_err(AppError::Io)?;
    let file_mtime = meta.modified().ok().and_then(unix_secs);
    let file_size = Some(meta.len() as i64);

    let base = db::get_obsidian_sync_state(&state.pool, link.id).await?;

    // 库侧预先算好：和做什么动作无关，纯内存操作，最廉价。
    let db_notes = link.notes.trim().to_string();
    let db_hash = hash_text(&db_notes);
    let db_changed = base.as_ref().map_or(true, |b| db_hash != b.base_hash);

    // 廉价快筛：mtime 与大小都没动，文件内容不可能变，连读都不必读。
    //
    // ⚠️ 这只是「排除法」，绝不能反过来当成「文件一定变了」：
    // 编辑器还可能在内容没变的情况下重写文件（mtime 照样刷新），所以要挪到最后靠哈希定论。
    let file_untouched = base
        .as_ref()
        .map_or(false, |b| b.file_mtime == file_mtime && b.file_size == file_size);

    // ---- 没有快照：第一次见到这条关联 ----
    let Some(base) = base else {
        return first_sync(state, vault, link, rel, &db_notes, &db_hash, &abs).await;
    };

    // ---- 文件确定没动 ----
    if file_untouched {
        if !db_changed {
            return Ok(Outcome::Skip);
        }
        // 只有库变了 → 推送。但「空笔记」不得覆盖用户在 Obsidian 里写的内容。
        if db_notes.is_empty() {
            // 整篇接管下这里读到的是文件全文；托管模式下是托管区正文。
            let file_notes = read_file_body(&abs)?.trim().to_string();
            // 库空、文件非空：分不清是「用户在 app 里清空了」还是「压根没写过」，
            // 保险起见以文件为准拉回来 —— 宁可多一次同步，也不能丢内容。
            if !file_notes.is_empty() {
                pull_to_db(state, link, &file_notes).await?;
                db::upsert_obsidian_sync_state(
                    &state.pool,
                    link.id,
                    rel,
                    &hash_text(&file_notes),
                    file_mtime,
                    file_size,
                )
                .await?;
                return Ok(Outcome::Pull);
            }
        }
        push_to_file(state, vault, link, rel, &db_notes).await?;
        db::upsert_obsidian_sync_state(&state.pool, link.id, rel, &db_hash, file_mtime, file_size)
            .await?;
        return Ok(Outcome::Push);
    }

    // ---- 文件可能变了：读出真内容，用哈希做最终判定 ----
    //
    // ⚠️ 这里不再有「缺托管标记 → 停止跟踪」的分支：整篇接管后，用户关联的笔记
    // 本来就没有标记，缺标记是最常见的合法状态。取消同步请走 `/obsidian/unlink`。
    let file_notes = read_file_body(&abs)?.trim().to_string();
    let file_hash = hash_text(&file_notes);

    if !db_changed && file_hash == base.base_hash {
        // 文件被重写过（mtime 变了）但内容没变 —— 补一次快照即可，避免下次重复。
        db::upsert_obsidian_sync_state(
            &state.pool,
            link.id,
            rel,
            &base.base_hash,
            file_mtime,
            file_size,
        )
        .await?;
        return Ok(Outcome::Skip);
    }

    if !db_changed {
        // 只有文件变了 → 拉取
        if file_hash == base.base_hash {
            return Ok(Outcome::Skip);
        }
        pull_to_db(state, link, &file_notes).await?;
        db::upsert_obsidian_sync_state(&state.pool, link.id, rel, &file_hash, file_mtime, file_size)
            .await?;
        return Ok(Outcome::Pull);
    }

    if file_hash == base.base_hash {
        // 只有库变了 → 推送
        push_to_file(state, vault, link, rel, &db_notes).await?;
        db::upsert_obsidian_sync_state(&state.pool, link.id, rel, &db_hash, file_mtime, file_size)
            .await?;
        return Ok(Outcome::Push);
    }

    // ---- 两边都改了：按约定「Obsidian 优先 + 留痕」 ----
    write_conflict_copy(vault, link, &db_notes).await?;
    pull_to_db(state, link, &file_notes).await?;
    db::upsert_obsidian_sync_state(&state.pool, link.id, rel, &file_hash, file_mtime, file_size)
        .await?;
    Ok(Outcome::Conflict)
}

/// 首次见到一条关联时的处理 —— **按接管模式分派**，判错了就是丢内容。
///
/// 三方比对在这里是失效的：没有 base，就无法判断「库和文件不一样」究竟是谁改的。
/// 老实现选择「一律以库为准推送」，那在只有托管区模式的年代是安全的 —— 推送只替换
/// 两个标记之间的一小段。但整篇接管后，库里的正文只是「关联那一刻」抓的**整篇快照**：
/// 拿它去推，等于把用户之后在 Obsidian 里写的内容整篇回退。后台循环一开跑，
/// 所有还没建过快照的老关联会被一口气推一遍，损失不可撤销。
///
/// 所以整篇接管走另一套：
/// - 两边一样 → 建快照了事
/// - 有一边是空的 → 往非空的那边对齐（没有内容可丢）
/// - 两边都非空且不同 → **无法判断方向**，按既定冲突策略：Obsidian 优先 + app 侧留痕
async fn first_sync(
    state: &AppState,
    vault: &Path,
    link: &db::ObsidianLinkRow,
    rel: &str,
    db_notes: &str,
    db_hash: &str,
    abs: &Path,
) -> Result<Outcome, AppError> {
    let content = fs::read_to_string(abs).map_err(AppError::Io)?;
    let file_notes = obsidian::read_note_body(&content).trim().to_string();
    let file_hash = hash_text(&file_notes);
    let (file_mtime, file_size) = file_meta(abs);
    let managed = obsidian::has_managed_zone(&content);

    match decide_first_sync(managed, db_notes, &file_notes) {
        FirstDecision::Skip => {
            save_snapshot(state, link.id, rel, &file_hash, file_mtime, file_size).await?;
            Ok(Outcome::Skip)
        }
        FirstDecision::Push => {
            push_to_file(state, vault, link, rel, db_notes).await?;
            // 刚写过文件，mtime / size 已经变了，快照必须取写**之后**的值
            let (mtime, size) = file_meta(abs);
            save_snapshot(state, link.id, rel, db_hash, mtime, size).await?;
            Ok(Outcome::Push)
        }
        FirstDecision::Pull => {
            pull_to_db(state, link, &file_notes).await?;
            save_snapshot(state, link.id, rel, &file_hash, file_mtime, file_size).await?;
            Ok(Outcome::Pull)
        }
        FirstDecision::Conflict => {
            write_conflict_copy(vault, link, db_notes).await?;
            pull_to_db(state, link, &file_notes).await?;
            save_snapshot(state, link.id, rel, &file_hash, file_mtime, file_size).await?;
            Ok(Outcome::Conflict)
        }
    }
}

/// 写入一轮同步后的快照（三方比对的 base）。
async fn save_snapshot(
    state: &AppState,
    item_id: i64,
    rel: &str,
    hash: &str,
    mtime: Option<i64>,
    size: Option<i64>,
) -> Result<(), AppError> {
    db::upsert_obsidian_sync_state(&state.pool, item_id, rel, hash, mtime, size).await
}

/// 首次同步方向的决策（**纯函数** —— 判错就是丢用户内容，必须能单测钉死）。
///
/// `managed` = 文件里有 collector 托管标记。
fn decide_first_sync(managed: bool, db_notes: &str, file_notes: &str) -> FirstDecision {
    // 托管区模式：库里的是 collector 自己写的笔记，而推送只替换两个标记之间的一小段，
    // 用户在标记之外写的东西碰不到 —— 久经检验的老行为，保持不变。
    if managed {
        return FirstDecision::Push;
    }
    // 整篇接管：库里的正文是「关联那一刻」抓的快照，与文件谁更新无从判断。
    if file_notes == db_notes {
        return FirstDecision::Skip;
    }
    if db_notes.is_empty() {
        // 库里没内容：拉回来不会丢任何东西
        return FirstDecision::Pull;
    }
    if file_notes.is_empty() {
        // 文件是空的：用户在 Obsidian 里没写东西，推进去无损失
        return FirstDecision::Push;
    }
    // 两边都有内容且不同 → 无从判断方向，按约定「Obsidian 优先 + 留痕」裁决
    FirstDecision::Conflict
}

/// 首次同步的四种走向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirstDecision {
    /// 两边一致，建快照即可
    Skip,
    /// 以库为准推送
    Push,
    /// 以文件为准拉回
    Pull,
    /// 两边都有内容且不同：Obsidian 优先 + app 侧留痕
    Conflict,
}

/// 取文件的 mtime 与大小（同步快照的廉价快筛用，取不到就记 None）。
fn file_meta(abs: &Path) -> (Option<i64>, Option<i64>) {
    match fs::metadata(abs) {
        Ok(meta) => (
            meta.modified().ok().and_then(unix_secs),
            Some(meta.len() as i64),
        ),
        Err(_) => (None, None),
    }
}

/// 读文件里 app 侧负责的正文：托管模式取托管区，整篇接管取全文。
///
/// 与推送方向 `obsidian::apply_note_body` 严格互逆 —— 两边口径必须一致，
/// 否则每轮都会判定「有变化」，双向同步进入死循环。
fn read_file_body(abs: &Path) -> Result<String, AppError> {
    let content = fs::read_to_string(abs).map_err(AppError::Io)?;
    Ok(obsidian::read_note_body(&content))
}

/// app → 文件。复用既有的 `write_or_update_note`：托管模式只替换托管区、保留用户区，
/// 整篇接管则整篇覆盖（用户关联的就是整篇，编辑的也是整篇）。
async fn push_to_file(
    state: &AppState,
    vault: &Path,
    link: &db::ObsidianLinkRow,
    rel: &str,
    notes: &str,
) -> Result<(), AppError> {
    let settings = obsidian::load_settings(&state.data_dir);
    let mut item = db::get_item(&state.pool, link.id).await?;
    // 用 trim 后的文本写入：这样文件读回来的内容与算 base_hash 时用的内容完全一致，
    // 否则每次都会平白判定「文件变了」。
    item.notes = notes.trim().to_string();
    let _vault = vault;
    let new_rel = obsidian::write_or_update_note(&settings, &item)?;
    if new_rel != rel {
        db::set_item_obsidian_path(&state.pool, link.id, &new_rel).await?;
    }
    Ok(())
}

/// 文件 → app。
///
/// ⚠️ 必须走 `set_item_notes_without_push` 这条不触发 Obsidian 推送的路径：
/// 若用普通的 `notes::save_notes`，它写库后又会推回文件，形成无限往返。
async fn pull_to_db(state: &AppState, link: &db::ObsidianLinkRow, notes: &str) -> Result<(), AppError> {
    db::set_item_notes_without_push(&state.pool, link.id, notes.trim()).await?;
    Ok(())
}

/// 冲突留痕：把 app 侧的笔记版本存一份到 `.collector-conflicts/`。
///
/// 用户在 Obsidian 里的版本胜出（他的主编辑器），但 app 侧的内容绝不能就此蒸发。
async fn write_conflict_copy(
    vault: &Path,
    link: &db::ObsidianLinkRow,
    app_side: &str,
) -> Result<PathBuf, AppError> {
    let dir = vault.join(CONFLICT_DIR);
    fs::create_dir_all(&dir).map_err(AppError::Io)?;
    let stamp = unix_secs(SystemTime::now()).unwrap_or(0);
    let safe_title: String = link
        .title
        .chars()
        .filter(|c| !"\\/:*?\"<>|".contains(*c))
        .take(60)
        .collect();
    let file_name = format!("{safe_title}-{stamp}.md");
    let target = dir.join(file_name);
    let body = format!(
        "# collector 同步冲突留痕\n\n\
         - 收藏：{}\n\
         - 条目 id：{}\n\
         - 时间：{stamp}\n\n\
         两边同时改动，已**以 Obsidian 中的版本为准**回写到 app；\n\
         下面是当时 app 里的版本，原样留档，确认无用后可删除本文件。\n\n\
         ---\n\n{}\n",
        link.title, link.id, app_side
    );
    fs::write(&target, body.as_bytes()).map_err(AppError::Io)?;
    Ok(target)
}

/// 拿到租约才同步，拿不到说明别的进程正在做。
///
/// 一个 `try_acquire` 覆盖三种情形：别人已佔且未过期 → false；没人占 / 就是自己 / 已过期 → true 并顺带完成续约。
async fn sync_if_holds_lease(
    state: &AppState,
    holder: &str,
) -> Result<Option<SyncReport>, AppError> {
    if db::try_acquire_obsidian_lease(&state.pool, holder, LEASE_STALE_SECS).await? {
        return Ok(Some(sync_once(state).await?));
    }
    Ok(None)
}

/// 拉起后台轮询循环（进程生命周期内常驻）。
///
/// GUI 主进程与 `--bridge-only` 后台桥都会调用它 —— 靠数据库租约保证同一时刻
/// 只有一个进程真正在干活，谁活着谁接管。这样不开着窗口也照样双向同步。
pub fn start_obsidian_sync_loop(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(STARTUP_DELAY_SECS)).await;
        let holder = format!("pid-{}", std::process::id());
        loop {
            let interval = {
                let guard = app.state::<AppState>();
                let state: &AppState = &guard;
                match sync_if_holds_lease(state, &holder).await {
                    Ok(Some(report)) => {
                        // 只在真有变化时广播 —— 否则每 30 秒弹一次提示会烦死人。
                        if report.has_changes() {
                            let _ = app.emit("obsidian://sync", &report);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("[obsidian-sync] 本轮同步失败：{error}"),
                }
                load_interval_secs(&state.data_dir)
            };
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    });
}

/// 从设置文件读轮询间隔，没配就用默认值。每次轮次都重读，改了不用重启应用。
fn load_interval_secs(data_dir: &Path) -> u64 {
    let path = data_dir.join("obsidian_sync_settings.json");
    let Ok(content) = fs::read_to_string(&path) else {
        return DEFAULT_INTERVAL_SECS;
    };
    #[derive(serde::Deserialize)]
    struct IntervalSettings {
        #[serde(default)]
        interval_secs: Option<u64>,
    }
    serde_json::from_str::<IntervalSettings>(&content)
        .ok()
        .and_then(|s| s.interval_secs)
        .map(|v| v.max(MIN_INTERVAL_SECS))
        .unwrap_or(DEFAULT_INTERVAL_SECS)
}

/// 轮询间隔下限：设成 0 会让后台变成忙循环，把磁盘和连接池打满。
pub const MIN_INTERVAL_SECS: u64 = 10;

#[cfg(test)]
mod tests {
    use super::*;

    // 首次同步的方向判断错了就是丢用户内容，逐条钉死。
    // 最关键的是最后一条：整篇接管下两边都有内容且不同时**绝不能**判成 Push。

    #[test]
    fn first_sync_managed_always_pushes() {
        // 托管区模式沿用老行为：库为准，且只替换标记之间的一小段，安全
        assert_eq!(
            decide_first_sync(true, "app 笔记", "文件里别的内容"),
            FirstDecision::Push
        );
        assert_eq!(decide_first_sync(true, "app 笔记", ""), FirstDecision::Push);
    }

    #[test]
    fn first_sync_full_skips_when_identical() {
        assert_eq!(decide_first_sync(false, "同一份", "同一份"), FirstDecision::Skip);
    }

    #[test]
    fn first_sync_full_pulls_when_db_empty() {
        // 库里没内容，拉回来不会丢任何东西
        assert_eq!(
            decide_first_sync(false, "", "用户在 Obsidian 写的"),
            FirstDecision::Pull
        );
    }

    #[test]
    fn first_sync_full_pushes_when_file_empty() {
        // 文件空 = Obsidian 里没写过东西，推进去无损失
        assert_eq!(
            decide_first_sync(false, "app 侧笔记", ""),
            FirstDecision::Push
        );
    }

    #[test]
    fn first_sync_full_conflicts_when_both_have_content() {
        // ⚠️ 核心回归点：整篇接管 + 两边都有内容且不同 → 必须走冲突（Obsidian 优先 + 留痕）。
        // 判成 Push 的话，后台循环一开跑就会拿「关联那一刻」的旧快照，
        // 整篇覆盖掉用户之后在 Obsidian 里写的内容 —— 且不可撤销。
        assert_eq!(
            decide_first_sync(false, "关联那刻的快照", "用户之后新写的"),
            FirstDecision::Conflict
        );
    }
}
