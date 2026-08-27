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
    delete_tag_category, execute_import, list_bilibili_favorites, list_tag_categories, list_tags,
    logout, merge_tags, open_url, parse_public_favorite_url, preview_import, rename_tag_category,
    search_items, update_item_notes, update_item_tags, upsert_tag,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            open_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running Bilibili Collector");
}
