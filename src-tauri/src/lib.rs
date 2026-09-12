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
    get_item_obsidian_path,
    get_netease_sync_settings,
    // Obsidian 单向联动
    get_obsidian_settings,
    get_trash_count,
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
    update_item_tags,
    upsert_tag,
    zhihu_browser_login,
    zhihu_logout,
    zhihu_profile,
    zhihu_set_cookie,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
        .setup(|app| {
            let handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(state::AppState::new(&handle))?;
            app.manage(state);
            // 浏览器扩展「快速入库」的本地桥：独立线程运行，端口被占满时只告警不阻断启动。
            capture::start(handle.clone());
            // 断点续传：上次没缓存完的封面，这次启动接着补。
            // 队列就存在数据库里（cover_url 有值但 cover_local_path 为空），没有待办时会立刻空转退出。
            cover_cache::spawn_cover_cache(&handle);
            // 网易云自动同步：启动 20 s 后跑一轮（内部自带间隔判断，太近会跳过），之后按配置轮询。
            start_netease_sync_loop(handle.clone());
            // 全局快捷键：注册失败只告警、不阻断启动（很可能被别的程序占用了同一个组合）
            {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                let hotkey = nowplaying::load_hotkey(&handle.state::<state::AppState>().data_dir);
                if let Err(e) = handle.global_shortcut().register(hotkey.as_str()) {
                    eprintln!("[nowplaying] 全局快捷键 {hotkey} 注册失败（可能被其它程序占用）：{e}");
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
            regenerate_bridge_token,
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
            now_playing_resolve,
            now_playing_capture,
            nowplaying_enabled,
            nowplaying_set_enabled,
            nowplaying_open,
            nowplaying_close,
            nowplaying_hotkey,
            nowplaying_is_playing,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Bilibili Collector");
}
