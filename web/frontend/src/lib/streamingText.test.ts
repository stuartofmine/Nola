import { test } from 'node:test';
import assert from 'node:assert/strict';
import { TextReveal, TEXT_REVEAL_MS, MAX_REVEAL_PIECES } from './streamingText.ts';

const visible = (view: ReturnType<TextReveal['update']>) => view.settled + view.pieces.map(piece => piece.text).join('');

test('long fast and paced streams keep complete text with bounded active pieces', () => {
    for (const interval of [0, 20]) {
        const reveal = new TextReveal();
        let text = '', peak = 0;
        for (let index = 0; index < 10_000; index++) {
            const prefix = text.length;
            text += '新增文字1234';
            const view = reveal.update(text, prefix, true, index * interval);
            peak = Math.max(peak, view.pieces.length);
            assert.ok(view.pieces.length <= MAX_REVEAL_PIECES);
            if (index % 1000 === 0) assert.equal(visible(view), text);
        }
        const final = reveal.update(text, text.length, false, 1_000_000);
        assert.equal(visible(final), text);
        assert.equal(final.pieces.length, 0);
        assert.equal(peak, MAX_REVEAL_PIECES);
    }
});

test('settled pieces disappear, old callbacks cannot affect replacement or cancelled text', () => {
    const reveal = new TextReveal();
    const first = reveal.update('before', 0, true, 0);
    const id = first.pieces[0].id;
    const append = reveal.update('before after', 6, true, TEXT_REVEAL_MS + 1);
    assert.equal(append.settled, 'before');
    assert.equal(append.pieces.length, 1);
    const replacement = reveal.update('changed', 0, true, TEXT_REVEAL_MS + 2);
    assert.equal(replacement.pieces.length, 0);
    assert.equal(visible(reveal.finish(id)), 'changed');
    const next = reveal.update('changed more', 7, true, TEXT_REVEAL_MS + 3);
    const current = next.pieces[0].id;
    assert.equal(visible(reveal.finish(id)), 'changed more');
    assert.equal(reveal.finish(current).pieces.length, 0);
    reveal.update('changed more tail', 12, true, 2000);
    const cancelled = reveal.update('changed more tail', 17, false, 2001);
    assert.equal(cancelled.pieces.length, 0);
    assert.equal(visible(reveal.finish(current)), 'changed more tail');
});

test('restored Markdown text is settled and subsequent new text alone reveals', () => {
    const reveal = new TextReveal();
    const history = 'stable code\nprint(1)\ntable cell';
    assert.equal(reveal.update(history, history.length, true, 0).pieces.length, 0);
    const next = reveal.update(history + ' next', history.length, true, 1);
    assert.equal(next.settled, history);
    assert.deepEqual(next.pieces.map(piece => piece.text), [' next']);
});
