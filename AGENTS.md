# AGENTS.md

## Project Overview

This is a local Windows desktop app for collecting Bilibili favorite videos and organizing them with tags instead of Bilibili folders.

- Project path: `C:\Users\lioh\Desktop\software0`
- App type: Tauri 2 desktop application
- Frontend: React + TypeScript + Vite
- Backend: Rust + Tauri commands
- Database: local SQLite through `sqlx`
- Data scope: metadata and links only, no video download

## Main Structure

Frontend:

- `src/App.tsx`: keeps all views mounted, so Import page state survives navigation
- `src/components/LibraryPage.tsx`: video library, text/tag search, tag editing entry
- `src/components/ImportPage.tsx`: Bilibili login, favorite import, per-video tag assignment, pagination
- `src/components/TagManagerPanel.tsx`: tag pool, tag categories, drag/drop categorization
- `src/components/TagPoolInput.tsx`: shared tag pool input with inline selected tags
- `src/components/VideoTagEditorModal.tsx`: edit tags for one video
- `src/components/TagEditorModal.tsx`: edit tag name/color
- `src/lib/api.ts`: Tauri invoke wrapper and browser mock fallback
- `src/types.ts`: shared frontend types

Backend:

- `src-tauri/src/lib.rs`: Tauri setup and command registration
- `src-tauri/src/commands.rs`: all Tauri command handlers
- `src-tauri/src/db.rs`: SQLite queries, tag/category logic, import/run logic
- `src-tauri/src/models.rs`: backend request/response types
- `src-tauri/src/source/bilibili.rs`: Bilibili login and favorite-list client
- `src-tauri/src/source/mod.rs`: `SourceAdapter` abstraction
- `src-tauri/src/state.rs`: app state, cookie persistence
- `src-tauri/src/wbi.rs`: Bilibili WBI signing

Migrations:

- `src-tauri/migrations/0001_init.sql`
- `src-tauri/migrations/0002_tag_categories.sql`
- `src-tauri/migrations/0003_remove_up_tags.sql`

## Implemented Features

- Bilibili QR login, with cookies persisted to both Windows Credential Manager and a local file
- Public favorite-list import
- Logged-in favorite-list import and optional cleanup after successful import
- Unified source adapter abstraction for future GitHub/NetEase sources
- SQLite storage for items, tags, item-tag links, import runs, and tag categories
- Tag pool shared between library and import views
- Tag creation by pressing Space/Enter/comma
- Per-video tag assignment during import
- Tag categories with drag/drop from uncategorized tags
- Video tag editing from the library card
- Partial-match text search for title, description, UP name, and tags
- Paginated import tag editor

## Important Implementation Notes

- Bilibili QR poll returns top-level `code: 0`; the real status is in `data.code`.
- Search uses SQL `LIKE '%query%'` for partial matching.
- UP tags are removed by migration `0003`.
- Tags are colored deterministically; existing same-name tags are reused.
- `TagInput` uses camelCase serde field names.
- App data is stored under the Tauri app-data directory, named `bili_collector.sqlite3`.
- Login cookie backup file is named `bilibili_cookie.txt` in the same app-data directory.
- Tauri bundling is disabled (`bundle.active: false`), so this is currently run as a dev app.

## Run Commands

From `C:\Users\lioh\Desktop\software0`:

```powershell
npm.cmd run dev -- --host 127.0.0.1
```

From `C:\Users\lioh\Desktop\software0\src-tauri`, after the Vite server is running:

```powershell
cmd.exe /d /s /c "`"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat`" && cd /d C:\Users\lioh\Desktop\software0\src-tauri && C:\Users\lioh\.cargo\bin\cargo.exe run"
```

Checks:

```powershell
npm.cmd run build
npm.cmd run test
cargo fmt -- --check
cargo check
cargo test --lib
```

## Known Areas To Reverify

- Drag/drop from uncategorized tags into categories after the latest WebView2 fixes
- Saving tags for a single video after the latest stale-tag-id guard
- Real Bilibili login persistence across app restarts
- Bilibili API availability and possible rate limits

## Next Steps

- Re-test import with a real Bilibili favorite list or public favorite link
- Confirm tag category drag/drop in the actual desktop window
- Decide whether to enable Tauri bundling and generate an installer
- Consider error surfacing/logging for Bilibili API failures
