DELETE FROM item_tags
WHERE tag_id IN (
    SELECT id FROM tags WHERE normalized LIKE '%up主%'
);

DELETE FROM tags
WHERE normalized LIKE '%up主%';
