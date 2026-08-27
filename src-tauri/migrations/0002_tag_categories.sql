CREATE TABLE IF NOT EXISTS tag_categories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    normalized TEXT NOT NULL,
    color TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE(normalized)
);

ALTER TABLE tags ADD COLUMN category_id INTEGER REFERENCES tag_categories(id) ON DELETE SET NULL;
