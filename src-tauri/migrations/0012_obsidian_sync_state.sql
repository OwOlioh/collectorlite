-- Obsidian 双向同步：三方合并需要一个「上次同步时的快照」，另外用租约避免
-- GUI 进程与后台桥进程并发写同一批笔记。
--
-- 为什么必须单独建表而不是给 items 加列：base_hash 只在「已建立笔记本关联」的
-- 收藏上有意义，绝大多数收藏永远用不到它；放进 items 会让每一行的宽度与
-- ITEM_ROW_COLUMNS 的漏列风险一起变大（见 db.rs 里 ITEM_ROW_COLUMNS 的约定）。

-- 每条「已关联到 vault 笔记」的收藏一条记录。
CREATE TABLE IF NOT EXISTS obsidian_sync_state (
    item_id    INTEGER PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
    rel_path   TEXT    NOT NULL,
    -- 上次同步落盘时**托管区正文**的哈希。这是判断「谁改过」的唯一可靠依据，
    -- 不能用 mtime 代替：编辑器保存会改 mtime 但内容未必变。
    base_hash  TEXT    NOT NULL,
    -- 仅用于快速跳过未变文件的廉价判据，不作为最终结论。
    file_mtime INTEGER,
    file_size  INTEGER,
    synced_at  INTEGER NOT NULL
);

-- 单行租约表：同一时刻只允许一个进程执行双向同步轮询。
-- GUI 主进程与 `--bridge-only` 后台桥可能同时在跑，两边都扫同一批文件会互相覆盖。
CREATE TABLE IF NOT EXISTS obsidian_sync_lease (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    holder       TEXT    NOT NULL,
    heartbeat_at INTEGER NOT NULL
);
