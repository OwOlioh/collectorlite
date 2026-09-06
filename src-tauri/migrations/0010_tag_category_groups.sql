-- 分类「组」：tag_categories.group_id 自引用组内最靠前成员（leader）的 id。
-- NULL = 未分组。组内成员共享同一颜色；排序时整组作为连续块整体移动。
ALTER TABLE tag_categories ADD COLUMN group_id INTEGER;
