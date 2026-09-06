// 抑制新版 Rust 对 tauri 命令宏的 never type fallback 兼容性 deny lint（宏生成代码触发，不影响逻辑）
#![allow(dependency_on_unit_never_type_fallback)]

mod capture;
mod commands;
mod db;
mod error;
mod models;
mod obsidian;
mod source;
mod state;
mod wbi;

use tauri::Manager;

use commands::{
    assign_tag_category, auto_purge_trash, backup_now, bilibili_poll_qr_login, bilibili_profile,
    bilibili_start_qr_login, create_tag_category, delete_item, delete_items, delete_items_by_tag,
    delete_tag, delete_tag_category, empty_trash, execute_csdn_import, execute_github_import,
    execute_import, get_bridge_info, get_trash_count, list_trash, pick_backup_folder,
    purge_item, purge_items, restore_item, restore_items,
    execute_zhihu_import, export_collection, import_browser_bookmarks, import_collection,
    list_bilibili_favorites, list_bilibili_opus_favorite, list_csdn_collections,
    list_github_stars, list_tag_categories,
    list_tags, list_zhihu_collections, logout, merge_tags, open_url, parse_csdn_collection_url,
    parse_public_favorite_url, parse_zhihu_collection_url, preview_csdn_import,
    preview_github_import, preview_import, preview_zhihu_import, recache_covers,
    regenerate_bridge_token, rename_tag_category, reorder_tag_categories,
    save_export_file, search_items,     update_item_notes,
    update_item_tags,
    // Obsidian 单向联动
    get_obsidian_settings,
    set_obsidian_settings,
    get_item_obsidian_path,
    open_note_in_obsidian,
    export_items_to_obsidian,
    pick_obsidian_vault,
    upsert_tag, zhihu_browser_login, zhihu_logout, zhihu_profile, zhihu_set_cookie,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(state::AppState::new(&handle))?;
            app.manage(state);
            // 浏览器扩展「快速入库」的本地桥：独立线程运行，端口被占满时只告警不阻断启动。
            capture::start(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bilibili_start_qr_login,
            bilibili_poll_qr_login,
            bilibili_profile,
            logout,
            list_bilibili_favorites,
            list_bilibili_opus_favorite,
            parse_public_favorite_url,
            preview_import,
            execute_import,
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
            update_item_notes,
            update_item_tags,
            import_browser_bookmarks,
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
            pick_obsidian_vault,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Bilibili Collector");
}
