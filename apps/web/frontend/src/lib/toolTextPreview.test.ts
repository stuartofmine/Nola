import test from 'node:test';
import assert from 'node:assert/strict';
import {toolTextPreview} from './toolTextPreview.ts';
test('tool preview preserves short values and bounds long lines, many rows and Unicode', () => {
    for (const text of ['', 'null', 'false', '0', 'a\n\tb\n']) assert.deepEqual(toolTextPreview(text), {text, truncated: false});
    assert.equal(toolTextPreview('x'.repeat(3000)).text.length, 2001);
    assert.equal(toolTextPreview(Array(50).fill('row').join('\n')).text.split('\n').length, 18);
    assert.equal(toolTextPreview('a'.repeat(1999) + '😀').text, 'a'.repeat(1999) + '…');
    assert.equal(toolTextPreview('a\n'.repeat(18)).truncated, true);
});
