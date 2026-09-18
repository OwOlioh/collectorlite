// 抑制新版 Rust 对 tauri 命令宏的 never type fallback 兼容性 deny lint（宏生成代码触发，不影响逻辑）
#![allow(dependency_on_unit_never_type_fallback)]

mod capture;
mod commands;
mod cover_cache;
mod db;
mod error;
mod models;
mod notes;
mod nowplaying;
mod obsidian;
mod obsidian_sync;
mod open_prefs;
mod source;
mod state;
mod uri;
mod weapi;
mod wbi;

use tauri::Manager;

use commands::{
    assign_tag_category,
    auto_purge_trash,
    backup_now,
    bilibili_poll_qr_login,
    bilibili_profile,
    bilibili_start_qr_login,
    cover_cache_status,
    create_tag_category,
    delete_item,
    delete_items,
    delete_items_by_tag,
    delete_tag,
    delete_tag_category,
    empty_trash,
    execute_csdn_import,
    execute_github_import,
    execute_import,
    execute_netease_import,
    execute_zhihu_import,
    export_collection,
    export_items_to_obsidian,
    get_bridge_info,
    get_bridge_autostart,
    set_bridge_autostart,
    get_item_obsidian_path,
    get_netease_sync_settings,
    // Obsidian 单向联动
    get_obsidian_settings,
    get_trash_count,
    get_collection_stats,
    get_duplicate_groups,
    merge_duplicate_items,
    group_tag_categories,
    import_browser_bookmarks,
    import_collection,
    list_bilibili_favorites,
    list_bilibili_opus_favorite,
    list_csdn_collections,
    list_github_stars,
    list_netease_collections,
    list_tag_categories,
    list_tags,
    list_trash,
    list_zhihu_collections,
    logout,
    merge_tags,
    netease_logout,
    netease_profile,
    netease_set_cookie,
    now_playing_capture,
    now_playing_current,
    // now_playing_progress 已移除（手动时间轴）
    now_playing_resolve,
    nowplaying_close,
    nowplaying_enabled,
    nowplaying_hotkey,
    nowplaying_open,
    nowplaying_is_playing,
    nowplaying_set_enabled,
    get_open_prefs,
    open_in_netease,
    open_note_in_obsidian,
    open_url,
    parse_csdn_collection_url,
    parse_netease_collection_url,
    parse_public_favorite_url,
    parse_zhihu_collection_url,
    pick_backup_folder,
    pick_obsidian_vault,
    preview_csdn_import,
    preview_github_import,
    preview_import,
    preview_netease_import,
    preview_zhihu_import,
    purge_item,
    purge_items,
    recache_covers,
    regenerate_bridge_token,
    rename_tag_category,
    reorder_tag_categories,
    restore_item,
    restore_items,
    save_export_file,
    save_netease_sync_settings,
    search_items,
    set_item_star,
    set_obsidian_settings,
    set_open_target,
    start_netease_sync_loop,
    sync_netease,
    ungroup_tag_category,
    update_item_notes,
    get_item_annotation,
    update_item_annotation,
    update_item_tags,
    upsert_tag,
    zhihu_browser_login,
    zhihu_logout,
    zhihu_profile,
    zhihu_set_cookie,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 是否「后台桥」模式：只跑浏览器扩展本地桥、不建主窗口，让用户不打开 GUI 也能收藏。
    // 由 `--bridge-only` 启动参数控制，setup 与 run 闭包共用这一个判定。
    let bridge_only = std::env::args()
        .any(|arg| arg == "--bridge-only" || arg == "--bridge_only");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        // 速记面板的全局快捷键唤起。按一次开、再按一次关（关闭即销毁窗口）。
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    use tauri_plugin_global_shortcut::ShortcutState;

                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    // 尊重设置页的开关
                    if let Some(st) = app.try_state::<state::AppState>() {
                        if !nowplaying::load_enabled(&st.data_dir) {
                            return;
                        }
                    }
                    // 同样绕到后台线程：快捷键回调也在主线程上，内联建窗口会自锁。
                    nowplaying::toggle_window_async(app);
                })
                .build(),
        )
        .setup(move |app| {
            // bridge_only 取自 run() 顶部（bool，Copy 进入闭包），不再在此重复解析。
            let handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(state::AppState::new(&handle))?;
            app.manage(state);
            // 浏览器扩展「快速入库」的本地桥：独立线程运行，端口被占满时只告警不阻断启动。
            // 桥直接写同一个 SQLite 库，不依赖 GUI 窗口是否存在。
            capture::start(handle.clone());

            // Obsidian 双向同步（轮询 vault）后台循环。靠数据库租约保证 GUI 与
            // --bridge-only 两个进程同时只有一个真正在干活，谁活着谁接管。
            //
            // 启用前提（2026-09-18）：整篇接管落地后，原先「首次无快照一律以库为准推送」
            // 会拿关联时抓的整篇旧快照覆盖用户之后在 Obsidian 里写的内容 —— 已改成
            // 首次只建快照、方向交给下一轮三方合并裁定。**没有这个修复就不能开这个循环**。
            obsidian_sync::start_obsidian_sync_loop(handle.clone());

            if bridge_only {
                // 无窗口常驻：建系统托盘维持进程 + 提供「打开主程序 / 退出」入口。
                // 托盘创建失败只是体验降级（桥仍在跑），不阻断启动。
                if let Err(error) = build_bridge_tray(app) {
                    eprintln!("[bridge] 托盘创建失败（不影响桥运行）：{error}");
                }
            } else {
                // 正常 GUI：建主窗口 + 后台任务。
                create_main_window(app);
                // 断点续传：上次没缓存完的封面，这次启动接着补。
                // 队列就存在数据库里（cover_url 有值但 cover_local_path 为空），没有待办时会立刻空转退出。
                cover_cache::spawn_cover_cache(&handle);
                // 网易云自动同步：启动 20 s 后跑一轮（内部自带间隔判断，太近会跳过），之后按配置轮询。
                start_netease_sync_loop(handle.clone());
                // 全局快捷键：注册失败只告警、不阻断启动（很可能被别的程序占用了同一个组合）
                {
                    use tauri_plugin_global_shortcut::GlobalShortcutExt;
                    let hotkey =
                        nowplaying::load_hotkey(&handle.state::<state::AppState>().data_dir);
                    if let Err(e) = handle.global_shortcut().register(hotkey.as_str()) {
                        eprintln!("[nowplaying] 全局快捷键 {hotkey} 注册失败（可能被其它程序占用）：{e}");
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bilibili_start_qr_login,
            bilibili_poll_qr_login,
            bilibili_profile,
            logout,
            netease_logout,
            netease_profile,
            netease_set_cookie,
            get_netease_sync_settings,
            save_netease_sync_settings,
            sync_netease,
            get_open_prefs,
            list_bilibili_favorites,
            list_bilibili_opus_favorite,
            list_netease_collections,
            parse_netease_collection_url,
            parse_public_favorite_url,
            preview_import,
            preview_netease_import,
            execute_import,
            execute_netease_import,
            search_items,
            delete_item,
            delete_items,
            delete_items_by_tag,
            // 回收站
            list_trash,
            restore_item,
            restore_items,
            purge_item,
            purge_items,
            empty_trash,
            get_trash_count,
            get_collection_stats,
            get_duplicate_groups,
            merge_duplicate_items,
            auto_purge_trash,
            list_tags,
            list_tag_categories,
            upsert_tag,
            merge_tags,
            delete_tag,
            create_tag_category,
            rename_tag_category,
            delete_tag_category,
            assign_tag_category,
            reorder_tag_categories,
            group_tag_categories,
            ungroup_tag_category,
            update_item_notes,
            get_item_annotation,
            update_item_annotation,
            update_item_tags,
            set_item_star,
            import_browser_bookmarks,
            open_in_netease,
            open_url,
            // Zhihu
            zhihu_set_cookie,
            zhihu_browser_login,
            zhihu_logout,
            zhihu_profile,
            list_zhihu_collections,
            parse_zhihu_collection_url,
            preview_zhihu_import,
            execute_zhihu_import,
            // CSDN
            list_csdn_collections,
            parse_csdn_collection_url,
            preview_csdn_import,
            execute_csdn_import,
            // GitHub
            list_github_stars,
            preview_github_import,
            execute_github_import,
            export_collection,
            import_collection,
            cover_cache_status,
            recache_covers,
            save_export_file,
            backup_now,
            pick_backup_folder,
            // 浏览器扩展「快速入库」本地桥
            get_bridge_info,
            get_bridge_autostart,
            regenerate_bridge_token,
    set_bridge_autostart,
    // Obsidian 单向联动
    get_obsidian_settings,
            set_obsidian_settings,
            get_item_obsidian_path,
            open_note_in_obsidian,
            export_items_to_obsidian,
            set_open_target,
            pick_obsidian_vault,
            // 速记浮窗（P1）
            now_playing_current,
            // now_playing_progress 已移除（手动时间轴）
            now_playing_resolve,
            now_playing_capture,
            nowplaying_enabled,
            nowplaying_set_enabled,
            nowplaying_open,
            nowplaying_close,
            nowplaying_hotkey,
            nowplaying_is_playing,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Bilibili Collector")
        .run(move |_app, event| {
            // 后台桥模式没有主窗口，Tauri 启动时会因「无窗口」触发 ExitRequested 而直接退出；
            // 这里阻止默认退出，让进程靠系统托盘常驻，直到用户在托盘选「退出后台桥」(app.exit)。
            // 正常 GUI 模式（bridge_only=false）保持默认行为：最后一个窗口关闭即退出。
            if bridge_only {
                if let tauri::RunEvent::ExitRequested { api, .. } = event {
                    api.prevent_exit();
                }
            }
        });
}

/// 正常 GUI 模式下创建主窗口。窗口不再写在 `tauri.conf.json` 里，
/// 而是按 `--bridge-only` 标志在执行期决定是否创建——后台桥模式就不建窗口。
fn create_main_window(app: &tauri::App) {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let handle = app.handle();
    if let Err(error) = WebviewWindowBuilder::new(handle, "main", WebviewUrl::App("index.html".into()))
        .title("collectorlite")
        .inner_size(1280.0, 800.0)
        .min_inner_size(940.0, 620.0)
        .resizable(true)
        .build()
    {
        eprintln!("[setup] 主窗口创建失败：{error}");
    }
}

/// 后台桥模式下的系统托盘：维持进程存活（无窗口时 Tauri 不会自动退出），
/// 并提供「打开主程序 / 退出后台桥」两个入口。
/// 注意：托盘类型来自 `tauri::tray`，只有启用 `tauri` 的 `tray-icon` 特性时才存在；
/// 该特性已在 Cargo.toml 的 `tauri` dependency 上开启，这里只需按 desktop 平台门控即可。
#[cfg(desktop)]
fn build_bridge_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    let handle = app.handle();
    let open = MenuItem::with_id(handle, "open", "打开 collectorlite", true, None::<&str>)?;
    let quit = MenuItem::with_id(handle, "quit", "退出后台桥", true, None::<&str>)?;
    let menu = Menu::with_items(handle, &[&open, &quit])?;
    // 应用图标来自 tauri.conf.json 的 `bundle.icon`，与是否创建窗口无关，正常总有。
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("未找到应用图标，无法创建托盘")?;
    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("collectorlite 后台桥（浏览器扩展收藏服务）")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                // 不带 --bridge-only 地再起一个进程，就是正常 GUI。
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(handle)?;
    Ok(())
}

#[cfg(not(desktop))]
fn build_bridge_tray(_app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // 极少数没有托盘支持的平台：后台桥仍能跑，只是没有系统托盘入口（需自行结束进程）。
    Ok(())
}
