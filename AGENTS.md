# AGENTS.md

## Project Overview

Multi-platform local desktop app for collecting favorites (Bilibili, browser bookmarks, Zhihu, etc.) and organizing them with tags.

- **Project path**: `C:\Users\lioh\Documents\GitHub\bilibili_collector`
- App type: Tauri 2 desktop application
- Frontend: React + TypeScript + Vite
- Backend: Rust + Tauri commands
- Database: local SQLite through `sqlx`
- Data scope: metadata and links only, no content download

## Main Structure

### Frontend

| File | Purpose |
|------|---------|
| `src/App.tsx` | All views mounted, `ToastProvider` wrapper, theme applied on mount |
| `src/components/LibraryPage.tsx` | Item library: search, source filter, tag editing, **virtual scroll (`VirtuosoGrid`)**, batch multi-select |
| `src/components/ImportPage.tsx` | Multi-source import orchestrator (step state + flow); per-source UI lives in `src/components/import/` |
| `src/components/import/BilibiliForm.tsx` | B站 source card (QR login / URL) |
| `src/components/import/ZhihuForm.tsx` | Zhihu source card (cookie login / URL) |
| `src/components/import/CsdnForm.tsx` | CSDN source card (username) |
| `src/components/import/GithubForm.tsx` | GitHub Stars source card (token / username) |
| `src/components/import/BrowserForm.tsx` | Browser bookmark HTML drag/drop |
| `src/components/import/TagEditor.tsx` | Per-item tag assignment preview/execute during import (toast on execute error) |
| `src/components/import/ResultCard.tsx` | Per-item import result card |
| `src/components/VideoCard.tsx` | Unified library card (covers, select checkbox, hover actions) |
| `src/components/CoverImage.tsx` | Blur-up lazy cover: shimmer skeleton → fade-in on decode |
| `src/components/BatchTagEditorModal.tsx` | Batch tag editor for multiple selected items |
| `src/components/Toast.tsx` | `ToastProvider` + `useToast()` lightweight notification system |
| `src/components/TagManagerPanel.tsx` | Tag pool, categories, drag/drop |
| `src/components/TagPoolInput.tsx` | Shared tag input with inline selected tags |
| `src/components/VideoTagEditorModal.tsx` | Edit tags for one item |
| `src/components/VideoNoteEditorModal.tsx` | Edit notes for one item |
| `src/components/TagEditorModal.tsx` | Edit tag name/color |
| `src/components/SettingsPage.tsx` | Account status, privacy info, **appearance (theme) selector** |
| `src/components/Sidebar.tsx` | Navigation sidebar |
| `src/lib/api.ts` | Tauri invoke wrapper with mock fallback |
| `src/lib/format.ts` | Shared `formatDuration` / `formatDate` helpers |
| `src/lib/theme.ts` | Theme persistence (localStorage + system preference) and `applyTheme` |
| `src/lib/tagUtils.ts` | Tag match/merge helpers |
| `src/types.ts` | Shared TypeScript types |

### Backend

| File | Purpose |
|------|---------|
| `src-tauri/src/lib.rs` | Tauri setup and command registration |
| `src-tauri/src/commands.rs` | All Tauri command handlers (B站 / browser / Zhihu / CSDN / GitHub) |
| `src-tauri/src/db.rs` | SQLite queries, upsert, tag operations, FTS, search |
| `src-tauri/src/models.rs` | Backend request/response types, `ExternalItem`, `CollectionInfo` |
| `src-tauri/src/source/mod.rs` | `SourceAdapter` trait definition |
| `src-tauri/src/source/bilibili.rs` | Bilibili client: QR login, favorites API |
| `src-tauri/src/source/browser.rs` | Browser bookmark HTML parser |
| `src-tauri/src/source/zhihu.rs` | Zhihu client: cookie login, collections API |
| `src-tauri/src/source/csdn.rs` | CSDN client: username → collections, article covers via `og:image` |
| `src-tauri/src/source/github.rs` | GitHub client: Stars import, uses `native-tls` (system proxy) |
| `src-tauri/src/state.rs` | App state, cookie/token persistence (file + keyring) |
| `src-tauri/src/wbi.rs` | Bilibili WBI signing |
| `src-tauri/src/error.rs` | `AppError` enum |

### Migrations

- `src-tauri/migrations/0001_init.sql` — items, tags, item_tags, import_runs, items_fts
- `src-tauri/migrations/0002_tag_categories.sql`
- `src-tauri/migrations/0003_remove_up_tags.sql`
- `src-tauri/migrations/0004_rebuild_items_fts.sql`
- `src-tauri/migrations/0005_cover_local_path.sql`
- `src-tauri/migrations/0006_video_notes.sql`

## Implemented Features

- **Bilibili**: QR login, public URL import, logged-in favorites import, optional cleanup
- **Browser bookmarks**: HTML file drag/drop import, folder hierarchy → auto tags, favicon covers
- **Zhihu**: Cookie login, collection list, URL import, supports articles/answers/pins
- **Source filter**: Toggle B站/browser/Zhihu in library view
- Unified `SourceAdapter` trait for extensible sources
- SQLite storage for items, tags, item-tag links, import runs, tag categories
- Tag pool shared between library and import views
- Tag creation by Space/Enter/comma
- Per-item tag assignment during import (with `ItemTagAssignment` by `external_id`)
- Tag categories with drag/drop
- Item tag editing from library card
- Item notes editing
- Partial-match text search (title, description, author, tags)
- FTS5 full-text search index
- Website favicon service for browser bookmarks (`favicon.im`)

## Frontend Features (added later)

- **CSDN**: username-based favorites import; article covers fetched via `og:image` meta and stored locally (`cover_local_path`)
- **GitHub Stars**: public starred repos import via personal access token (or username); `native-tls` client uses system proxy for China network access
- **Appearance / theme**: Light / dark / system theme toggle persisted to localStorage (`src/lib/theme.ts`); `:root` holds light tokens, `[data-theme="dark"]` overrides; sidebar uses `--side-*` variables
- **Toast notifications**: lightweight `ToastProvider` + `useToast()` for success/error/info feedback (import, delete, tag save, execute errors) — see `src/components/Toast.tsx`
- **Batch operations**: multi-select cards → batch tag editor (`BatchTagEditorModal`) + batch delete + JSON export; `⌘A` select all, `Esc` clear (input focused = no-op)
- **Performance**: library list uses `VirtuosoGrid` virtual scrolling; covers use blur-up lazy loading (`CoverImage`)
- **Micro-animations**: card hover lift, tag scale, sidebar transitions, modal/toast entrance, shimmer skeletons — all gated by `prefers-reduced-motion`

## Key Implementation Notes

### Source Architecture

- All sources implement `SourceAdapter` trait: `list_collections`, `resolve_collection`, `fetch_collection`, `enrich_items`
- `ExternalItem` is the unified intermediate representation
- `source` field (e.g. `"bilibili"`, `"browser"`, `"zhihu"`) distinguishes origins
- `upsert_item` uses `(source, external_id)` as unique key
- **Critical**: `external_id` must be unique within source and consistent between frontend preview and backend fetch

### Bilibili Specifics

- QR poll returns top-level `code: 0`; real status in `data.code`
- Cookie: `bilibili_cookie.txt` + Windows Credential Manager
- WBI signing required for some API calls

### Browser Bookmarks Specifics

- Frontend and backend both compute `SHA256(URL)` → `bk_` + first 16 hex chars as `external_id`
- Uses `scraper` crate to parse Netscape Bookmark HTML format
- Folder hierarchy stored in `extra.folder_tags` as JSON array, auto-creates tags on import

### Zhihu Specifics

- API: `GET /api/v4/people/{url_token}/collections`, `GET /api/v4/collections/{id}/items`
- Requires `z_c0` + `d_c0` cookies (both needed, not just z_c0)
- Cookie: `zhihu_cookie.txt` + Windows Credential Manager
- Item JSON has `content.id` (not top-level `id`), may be number or string
- `json_value_to_string()` helper handles both types
- Items API always returns 401 without cookie (even for public collections)
- 403 errors often caused by missing request headers (`accept`, `x-requested-with`)
- Login state auto-checked on page load via `refreshZhihuProfile`

### CSDN Specifics

- Import by **username** (the English handle, not the Chinese display name). API returns an empty list for unknown/wrong username — surface a clear hint telling the user where to find their English handle.
- Cover images are not in the list API; `enrich_items` fetches each article page, extracts the `og:image` meta, then downloads to local `cover_local_path`.
- No login required for public collections.

### GitHub Specifics

- Import public Stars via **personal access token** (or username for public-only stars).
- **Network**: GitHub API is often blocked in China. The client is built with `native-tls` (not `rustls-tls`) so it uses the OS/system proxy TLS stack. If you see `error sending request for url (https://api.github.com/...)`, it is a proxy/TLS issue — confirm `Cargo.toml` enables `native-tls`, not `rustls-tls`.
- Avatar (`avatar_url`) is used as the cover.

### Frontend Structure

- `ImportPage` is a thin orchestrator owning import step state; each source's UI is a component in `src/components/import/` (one file per source) plus shared `TagEditor` / `ResultCard`. Keep per-source UI in those subcomponents.
- Library cards render via `VideoCard` (covers, select checkbox, hover actions); covers go through `CoverImage` for blur-up lazy loading. New sources automatically benefit once they appear in the library.
- Global notifications use `useToast()` from `src/components/Toast.tsx` — prefer it over `window.alert`.
- Theme is applied by setting `data-theme` on `<html>`; **new colors MUST be added as CSS variables** (never hardcode hex in component CSS) so both light and dark themes work.
- Performance: `LibraryPage` list uses `react-virtuoso` (`VirtuosoGrid`); ensure the scroll region is `height:100%` inside a flex column.

### Tag Assignment

- `execute_xxx_import` matches tags by `external_id` in `ItemTagAssignment`
- **DO NOT** fallback to global `tag_specs` when no match found — use empty array
- Frontend `buildTagSpecs` builds per-item `TagInput[]` from `PerVideoTagState`

### State Isolation

- Each source uses independent React state variables (e.g. `zhihuProfile`, `zhihuCollections`)
- Shared state causes cross-contamination bugs

### Database

- `search_items` supports `sources: Vec<String>` filter (AND logic)
- `camelCase` serde for all JSON models
- FTS index rebuilt after every tag change

## Run Commands

From project root:

```powershell
# Terminal 1: Vite dev server
npm.cmd run dev -- --host 127.0.0.1

# Terminal 2: Tauri
cd src-tauri
cargo run
```

Checks:

```powershell
npm.cmd run build
npm.cmd run test
cargo fmt -- --check
cargo check
cargo test --lib
```

## Adding New Sources

Read `DEVELOPMENT.md` for the complete guide with step-by-step instructions, architecture patterns, and all bugs/traps documented.

## Known Quirks

- `beforeDevCommand` in tauri.conf.json is set to empty — Vite must be started manually
- Database file is `bili_collector_v2.sqlite3` (renamed to avoid migration 7 panic)
- WebView2 cookie reading is unreliable; use `document.cookie` injection or manual paste instead
- `favicon.im` is used for browser bookmark favicons (Google favicon service blocked in China)