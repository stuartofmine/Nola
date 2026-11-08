// Read-only canonical evidence for real-provider acceptance. Never infer success from `sealed`.
const {execFileSync} = require('node:child_process');
const assert = require('node:assert/strict');
function query(database, sql, parameters = []) {
    return JSON.parse(execFileSync('python3', ['-c',
        'import sqlite3,json,sys\nc=sqlite3.connect("file:"+sys.argv[1]+"?mode=ro",uri=True);c.row_factory=sqlite3.Row\nprint(json.dumps([dict(r) for r in c.execute(sys.argv[2],json.loads(sys.argv[3]))]))',
        database, sql, JSON.stringify(parameters)], {encoding: 'utf8'}));
}
function delegatedRuns(database, conversation) {
    const rows = query(database, `SELECT t.id AS task_id,t.owner_session_id,t.child_session_id,
        t.started_at_ms,t.ended_at_ms,b.run_id,r.status,json_extract(v.result_json,'$.value.kind') AS outcome
        FROM durable_tasks t JOIN runtime_task_run_bindings b ON b.task_id=t.id
        JOIN runs r ON r.id=b.run_id LEFT JOIN run_results v ON v.run_id=r.id
        WHERE t.owner_session_id=?`, [conversation]);
    assert.equal(rows.length, 2, 'two actual child tasks must be bound to this parent');
    assert.equal(new Set(rows.map(row => row.child_session_id)).size, 2);
    assert.equal(new Set(rows.map(row => row.run_id)).size, 2);
    for (const row of rows) {
        assert.equal(row.outcome, 'answered');
        assert.ok(row.started_at_ms && row.ended_at_ms >= row.started_at_ms);
    }
    assert.ok(Math.max(...rows.map(row => row.started_at_ms)) < Math.min(...rows.map(row => row.ended_at_ms)), 'parallel child execution must overlap');
    return rows;
}
function submissionHistory(database, conversation) {
    return query(database, "SELECT request_json FROM source_submission_receipts WHERE json_extract(identity_json,'$.conversation_id')=? AND request_json IS NOT NULL ORDER BY rowid", [conversation])
        .map(row => JSON.parse(row.request_json));
}
module.exports = {query, delegatedRuns, submissionHistory};
