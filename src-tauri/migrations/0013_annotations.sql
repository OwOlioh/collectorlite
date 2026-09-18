-- 侧边栏「批注模式」的轻量批注，与应用内每条收藏下方的批注按钮共用、互相同步。
-- 刻意独立于 items.notes（Obsidian 笔记），因此批注绝不进 Obsidian。
CREATE TABLE annotations (
  item_id INTEGER PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
  body TEXT NOT NULL DEFAULT '',
  updated_at INTEGER
);
