-- Obsidian 单向联动：记录每条收藏在 vault 中的相对路径（相对 vault 根，便于换机迁移）。
ALTER TABLE items ADD COLUMN obsidian_path TEXT;
