import assert from 'node:assert/strict';
import { test } from 'node:test';
import { commandQuery, parseComposerCommand } from './composerCommands.ts';
test('only explicit first-position slash input is a local command', () => {
    for (const text of ['搜索 current', 'please /stop', 'https://example.org', 'model permission architecture', '/opt/tools/notes.md', '//comment']) assert.equal(parseComposerCommand(text), null);
    assert.equal(commandQuery('/mo'), 'mo');
    assert.equal(commandQuery('/model '), null);
    assert.equal(commandQuery('/opt/tools'), null);
    assert.equal(parseComposerCommand('/goal')?.command?.name, 'goal');
    assert.equal(parseComposerCommand('/goal ship v1')?.valid, true);
    assert.equal(parseComposerCommand('/goal ship v1')?.argument, 'ship v1');
    assert.equal(parseComposerCommand('/search 最近变化')?.argument, '最近变化');
    assert.equal(parseComposerCommand('/stop extra')?.valid, false);
    assert.equal(parseComposerCommand('/unknown')?.valid, false);
    assert.equal(parseComposerCommand('/stop ')?.valid, true);
});
