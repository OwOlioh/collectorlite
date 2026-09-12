//! 封面本地缓存的后台任务。
//!
//! ## 设计要点：数据库本身就是任务队列
//!
//! 一条收藏只要 `cover_url` 有值、而 `cover_local_path` 为空，就代表「待缓存」。
//! 由此得到三个好处：
//!
//! 1. **导入不必等封面**：数据先落库并立刻返回结果，封面交给后台补，UI 不再卡在导入中。
//! 2. **天然断点续传**：中途关掉应用不会丢任务，下次启动扫到同样的行接着缓存即可。
//! 3. **失败可重试**：下载失败的行仍然留在队列里，下次启动会再试一次。
//!
//! ## 中途关掉会发生什么
//!
//! - **已落库的数据**：安全，与封面无关。
//! - **已下载但没来得及回写路径的封面**：文件留在 `covers/`，路径没写进数据库。
//!   下次启动会重新下载并**覆盖同名文件**（文件名由 `source + external_id` 决定），
//!   不会造成垃圾堆积；只有扩展名变化时才可能留下一个孤立文件，代价可忽略。
//! - **没下载到的**：留在队列，下次继续。
//!
//! 所以「关闭应用」不会损坏数据，最坏情况只是浪费一点已下载的流量。

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager};

use crate::commands::save_cover_file;
use crate::db;
use crate::error::AppError;
use crate::state::AppState;

/// 与实时导入 `cache_item_covers` 保持一致的有界并发度。
const COVER_CONCURRENCY: usize = 8;
/// 每处理多少条就回写一次数据库并广播进度。太频繁会让 SQLite 反复提交、前端反复重渲染。
const FLUSH_EVERY: usize = COVER_CONCURRENCY * 4;

pub const COVER_CACHE_PROGRESS_EVENT: &str = "cover-cache://progress";

/// 广播给前端的缓存进度。`running = false` 表示本轮结束。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverCacheProgress {
    pub total: i64,
    pub done: i64,
    pub cached: i64,
    pub failed: i64,
    pub running: bool,
}

/// 扫描「待缓存」的行，并发下载并批量回写路径。
///
/// 一次调用 = 一轮。已完成的会写回 `cover_local_path` 从而自动出队；
/// 失败的原样留在队列里，留给下一轮（或下次启动）重试。
pub async fn run_pass(app: &AppHandle) -> Result<CoverCacheProgress, AppError> {
    // 取一次状态并立刻转成普通引用：`State<'_, AppState>` 不是 Copy，
    // 直接进 `async move` 会被搬走，导致外层循环第二轮起借不到。
    let state = app.state::<AppState>();
    let state: &AppState = &state;
    let items = db::fetch_items_needing_cover_cache(&state.pool).await?;
    let total = items.len() as i64;
    if total == 0 {
        return Ok(CoverCacheProgress {
            total: 0,
            done: 0,
            cached: 0,
            failed: 0,
            running: false,
        });
    }

    let mut cached: i64 = 0;
    let mut failed: i64 = 0;
    let mut done: i64 = 0;
    let mut paths: Vec<(String, String, String)> = Vec::new();

    for chunk_start in (0..items.len()).step_by(COVER_CONCURRENCY) {
        let end = (chunk_start + COVER_CONCURRENCY).min(items.len());
        let futures: Vec<_> = (chunk_start..end)
            .map(|i| {
                let item = &items[i];
                async move {
                    let url = match item.cover_url.as_deref().filter(|v| !v.is_empty()) {
                        Some(v) => v,
                        None => return None,
                    };
                    let download = match item.source.as_str() {
                        "bilibili" => state.bili.download_cover(url).await,
                        "csdn" => state.csdn.download_cover(url).await,
                        // 其余来源（知乎 / GitHub / 浏览器）的远程封面 / favicon 复用
                        // capture.rs 里的 download_cover_for 分发（已带系统代理 + UA + timeout）。
                        // 之前这里直接 return None、注释「WebView 能直接加载」是基于过期假设——
                        // 实际 WebView 默认不继承 app 代理，远程 favicon 在很多网络下加载失败，
                        // 这些源的封面永远进不了本地缓存队列，只能指望导入时一次性成功。
                        _ => match crate::capture::download_cover_for(state, &item.source, url).await {
                            Some(v) => Ok(v),
                            None => Err(AppError::Other("通用封面下载返回 None".into())),
                        },
                    };
                    let (bytes, extension) = download.ok()?;
                    let path =
                        save_cover_file(state, &item.source, &item.external_id, &bytes, &extension)
                            .ok()?;
                    Some((item.source.clone(), item.external_id.clone(), path))
                }
            })
            .collect();

        for outcome in futures::future::join_all(futures).await {
            match outcome {
                Some(row) => {
                    paths.push(row);
                    cached += 1;
                }
                None => failed += 1,
            }
            done += 1;
        }

        if chunk_start % FLUSH_EVERY == 0 || end == items.len() {
            flush(state, &mut paths).await;
            emit_progress(app, total, done, cached, failed, true);
        }
    }

    flush(state, &mut paths).await;
    let progress = CoverCacheProgress {
        total,
        done,
        cached,
        failed,
        running: false,
    };
    let _ = app.emit(COVER_CACHE_PROGRESS_EVENT, progress.clone());
    Ok(progress)
}

/// 后台跑封面缓存。已有任务在跑时只打一个「再来一轮」的标记，
/// 让正在跑的任务结束后顺带覆盖新导入的项，避免两个任务重复下载同一批封面。
pub fn spawn_cover_cache(app: &AppHandle) {
    {
        let state = app.state::<AppState>();
        if state.cover_cache_busy.swap(true, Ordering::SeqCst) {
            state.cover_cache_rerun.store(true, Ordering::SeqCst);
            return;
        }
    }

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last = CoverCacheProgress {
            total: 0,
            done: 0,
            cached: 0,
            failed: 0,
            running: false,
        };
        loop {
            match run_pass(&handle).await {
                Ok(progress) => {
                    if progress.total > 0 {
                        eprintln!(
                            "[cover-cache] 本轮完成 {}/{}（失败 {}）",
                            progress.cached, progress.total, progress.failed
                        );
                        last = progress;
                    }
                }
                Err(error) => eprintln!("[cover-cache] 本轮失败：{error}"),
            }

            let state = handle.state::<AppState>();
            let rerun = state.cover_cache_rerun.swap(false, Ordering::SeqCst);
            drop(state);
            if !rerun {
                break;
            }
        }

        let state = handle.state::<AppState>();
        state.cover_cache_busy.store(false, Ordering::SeqCst);
        drop(state);

        // 收尾广播：前端据此关掉进度提示。total = 0 表示本轮其实没活干，不必打扰用户。
        if last.total > 0 {
            let _ = handle.emit(
                COVER_CACHE_PROGRESS_EVENT,
                CoverCacheProgress {
                    running: false,
                    ..last
                },
            );
        }
    });
}

async fn flush(state: &AppState, paths: &mut Vec<(String, String, String)>) {
    if paths.is_empty() {
        return;
    }
    // 写回失败不阻断：路径没写进去的行下次还会被扫到，等于自动重试。
    if let Err(error) = db::set_items_cover_local_paths(&state.pool, paths).await {
        eprintln!("[cover-cache] 回写封面路径失败：{error}");
    }
    paths.clear();
}

fn emit_progress(app: &AppHandle, total: i64, done: i64, cached: i64, failed: i64, running: bool) {
    let _ = app.emit(
        COVER_CACHE_PROGRESS_EVENT,
        CoverCacheProgress {
            total,
            done,
            cached,
            failed,
            running,
        },
    );
}
