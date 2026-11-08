-- Count live guidance without scanning/decoding accumulated message bodies.
CREATE INDEX IF NOT EXISTS runtime_task_guidance_outstanding
ON runtime_task_guidance(task_id, json_extract(record_json,'$.status'));
