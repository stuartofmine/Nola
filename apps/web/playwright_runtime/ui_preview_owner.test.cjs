const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { verifyPreviewOwner } = require('./ui_preview_owner.cjs');
const database = '/tmp/nola-web-preview.fixture/state.db';
const executable = path.resolve(__dirname, '../../../target/debug/nola');
function fixture(overrides = {}) {
    let listeners = 0;
    const command = `${executable} web --port 5001`;
    return {
        canonical: value => value.startsWith('/tmp') ? value.replace('/tmp', '/private/tmp') : value,
        run(file, args) {
            if (file.endsWith('lsof')) {
                if (args.includes('cwd')) return `p123\nn${path.resolve(__dirname, '../../..')}\n`;
                return overrides.listener?.(++listeners) ?? 'p123\n';
            }
            const field = args.at(-1);
            if (field === 'comm=') return overrides.executable ?? executable;
            if (field === 'lstart=') return 'Mon Oct 19 10:00:00 2026';
            if (field === 'command=') return args[0] === 'eww'
                ? `${command} ${overrides.environment ?? `NOLA_DB_PATH=${database} OTHER_SECRET=never_print_me`}`
                : command;
            throw new Error('unexpected fixture read');
        },
    };
}
test('only exact unique listener executable and temporary database binding is accepted', () => {
    assert.deepEqual(verifyPreviewOwner(database, fixture()), { pid: 123, database: '/private/tmp/nola-web-preview.fixture/state.db' });
    assert.equal(verifyPreviewOwner(database, fixture({ executable: 'target/debug/nola' })).pid, 123);
});
test('wrong database, missing or ambiguous bindings, foreign binary and listener replacement fail closed', () => {
    for (const overrides of [
        { environment: 'NOLA_DB_PATH=/Users/person/real.db SECRET=never_print_me' },
        { environment: 'OTHER_SECRET=never_print_me' },
        { environment: `NOLA_DB_PATH=${database} NOLA_DB_PATH=${database}` },
        { executable: '/tmp/nola' },
        { listener: () => 'p123\np456\n' },
        { listener: count => count === 1 ? 'p123\n' : 'p456\n' },
    ]) assert.throws(() => verifyPreviewOwner(database, fixture(overrides)), { message: 'preview_listener_owner_unverified' });
});
test('diagnostics and symlink escapes never leak or authorize writes', () => {
    assert.throws(() => verifyPreviewOwner(database, { run() { throw new Error('SECRET never_print_me'); }, canonical: value => value }), { message: 'preview_listener_owner_unverified' });
    const deps = fixture();
    deps.canonical = value => value === database ? '/Users/person/real.db' : value;
    assert.throws(() => verifyPreviewOwner(database, deps), { message: 'preview_listener_owner_unverified' });
});
