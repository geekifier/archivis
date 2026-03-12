-- Re-parent existing grandchild tasks (resolve_book under scan_isbn)
-- to become direct children of the root task (import_directory/import_file).
-- No schema change — just data fixup for the 2-level hierarchy.
UPDATE tasks
SET parent_task_id = (
    SELECT t2.parent_task_id
    FROM tasks t2
    WHERE t2.id = tasks.parent_task_id
      AND t2.parent_task_id IS NOT NULL
)
WHERE parent_task_id IN (
    SELECT id FROM tasks
    WHERE task_type = 'scan_isbn'
      AND parent_task_id IS NOT NULL
);
