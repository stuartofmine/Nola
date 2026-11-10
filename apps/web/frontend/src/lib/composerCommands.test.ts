import assert from 'node:assert/strict';
import { test } from 'node:test';
import { commandQuery, parseComposerCommand } from './composerCommands.ts';
test('only explicit first-position slash input is a local command', () => {
    for (const text of ['搜索 current', 'please /stop', 'https://example.org', 'model permission architecture']) assert.equal(parseComposerCommand(text), null);
    assert.equal(commandQuery('/mo'), 'mo');
    assert.equal(commandQuery('/model '), null);
    assert.equal(parseComposerCommand('/search 最近变化')?.argument, '最近变化');
    assert.equal(parseComposerCommand('/stop extra')?.valid, false);
    assert.equal(parseComposerCommand('/unknown')?.valid, false);
    assert.equal(parseComposerCommand('/stop ')?.valid, true);
});
