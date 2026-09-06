-- 收藏星标：starred 标记 + 打星时间（用于置顶排序）
-- starred_at 为 unix 秒；NULL 表示从未打星。星标内容在列表任何排序下均置顶。
ALTER TABLE items ADD COLUMN starred INTEGER NOT NULL DEFAULT 0;
ALTER TABLE items ADD COLUMN starred_at INTEGER;
