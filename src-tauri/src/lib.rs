// 抑制新版 Rust 对 tauri 命令宏的 never type fallback 兼容性 deny lint（宏生成代码触发，不影响逻辑）
#![allow(dependency_on_unit_never_type_fallback)]

mod commands;
mod db;
mod error;
mod models;
mod source;
mod state;
mod wbi;

use tauri::Manager;

use commands::{
    assign_tag_category, bilibili_poll_qr_login, bilibili_profile, bilibili_start_qr_login,
    create_tag_category, delete_item, delete_items, delete_items_by_tag, delete_tag,
    delete_tag_category, execute_csdn_import, execute_github_import, execute_import,
    execute_zhihu_import, export_collection, import_browser_bookmarks, import_collection,
    list_bilibili_favorites, list_csdn_collections, list_github_stars, list_tag_categories,
    list_tags, list_zhihu_collections, logout, merge_tags, open_url, parse_csdn_collection_url,
    parse_public_favorite_url, parse_zhihu_collection_url, preview_csdn_import,
    preview_github_import, preview_import, preview_zhihu_import, recache_covers,
    rename_tag_category, save_export_file, search_items, update_item_notes, update_item_tags,
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bilibili_start_qr_login,
            bilibili_poll_qr_login,
            bilibili_profile,
            logout,
            list_bilibili_favorites,
            parse_public_favorite_url,
            preview_import,
            execute_import,
            search_items,
            delete_item,
            delete_items,
            delete_items_by_tag,
            list_tags,
            list_tag_categories,
            upsert_tag,
            merge_tags,
            delete_tag,
            create_tag_category,
            rename_tag_category,
            delete_tag_category,
            assign_tag_category,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Bilibili Collector");
}
