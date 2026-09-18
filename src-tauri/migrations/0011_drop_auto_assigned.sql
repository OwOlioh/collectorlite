-- 历史上曾试图 DROP item_tags.auto_assigned（回退实验性自动打标功能），
-- 但该列从未被任何迁移创建过；在全新库上 `ALTER TABLE ... DROP COLUMN`
-- 会失败，导致应用全新安装起不来。此迁移已退化为 no-op，仅保留版本号
-- 占位，使 sqlx 在全新库上能顺利越过第 11 步。真实库里对应的
-- _sqlx_migrations 记录已通过 DELETE + wal_checkpoint 清除。
SELECT 1;
