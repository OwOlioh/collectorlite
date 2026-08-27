DROP TABLE IF EXISTS items_fts;

CREATE VIRTUAL TABLE items_fts USING fts5(
    title,
    description,
    author_name,
    partition_name,
    tags,
    content='',
    contentless_delete=1
);

INSERT INTO items_fts(rowid, title, description, author_name, partition_name, tags)
SELECT
    i.id,
    i.title,
    i.description,
    COALESCE(i.author_name, ''),
    COALESCE(i.partition_name, ''),
    COALESCE(
        (
            SELECT group_concat(t.name, ' ')
            FROM item_tags it
            JOIN tags t ON t.id = it.tag_id
            WHERE it.item_id = i.id
        ),
        ''
    )
FROM items i;
