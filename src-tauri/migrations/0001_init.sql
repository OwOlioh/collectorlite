CREATE TABLE IF NOT EXISTS items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    source_url TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    cover_url TEXT,
    author_name TEXT,
    author_id TEXT,
    partition_name TEXT,
    published_at INTEGER,
    duration INTEGER,
    favorite_time INTEGER,
    extra_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(source, external_id)
);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    normalized TEXT NOT NULL,
    color TEXT,
    description TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE(namespace, normalized)
);

CREATE TABLE IF NOT EXISTS item_tags (
    item_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (item_id, tag_id),
    FOREIGN KEY (item_id) REFERENCES items(id) ON DELETE CASCADE,
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS import_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    collection_id TEXT,
    collection_title TEXT,
    collection_url TEXT,
    status TEXT NOT NULL,
    total INTEGER NOT NULL DEFAULT 0,
    imported INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    failed INTEGER NOT NULL DEFAULT 0,
    cleanup_status TEXT,
    cleanup_eligible INTEGER NOT NULL DEFAULT 0,
    error_json TEXT,
    started_at INTEGER NOT NULL,
    finished_at INTEGER
);

CREATE TABLE IF NOT EXISTS import_run_items (
    run_id INTEGER NOT NULL,
    item_id INTEGER NOT NULL,
    PRIMARY KEY (run_id, item_id),
    FOREIGN KEY (run_id) REFERENCES import_runs(id) ON DELETE CASCADE,
    FOREIGN KEY (item_id) REFERENCES items(id) ON DELETE CASCADE
);

CREATE VIRTUAL TABLE IF NOT EXISTS items_fts USING fts5(
    title,
    description,
    author_name,
    partition_name,
    tags,
    content=''
);
