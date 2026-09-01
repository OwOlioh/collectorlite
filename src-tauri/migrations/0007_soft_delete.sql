-- 回收站（软删除）支持：为 items 表增加 deleted_at 列。
-- NULL 表示正常在库；有值表示已移入回收站，值为删除时间戳（秒）。
ALTER TABLE items ADD COLUMN deleted_at INTEGER;
