import { test } from 'node:test';
import assert from 'node:assert/strict';
import { TextReveal, TEXT_REVEAL_MS, TEXT_COMPLETION_MS, MAX_REVEAL_PIECES, waterRemaining } from './streamingText.ts';

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
    assert.equal(visible(append), 'before after');
    assert.ok(append.pieces.length > 0);
    const replacement = reveal.update('changed', 0, true, TEXT_REVEAL_MS + 2);
    assert.equal(replacement.pieces.length, 0);
    assert.equal(visible(reveal.finish(id)), 'changed');
    const next = reveal.update('changed more', 7, true, TEXT_REVEAL_MS + 3);
    const current = next.pieces[0].id;
    assert.equal(visible(reveal.finish(id)), 'changed more');
    assert.equal(visible(reveal.finish(current)), 'changed more');
    reveal.update('changed more tail', 12, true, 2000);
    const cancelled = reveal.update('changed more tail', 17, false, 2001);
    assert.ok(cancelled.pieces.length > 0);
    assert.equal(reveal.advance(2001 + TEXT_COMPLETION_MS).pieces.length, 0);
    assert.equal(visible(reveal.finish(current)), 'changed more tail');
});

test('restored Markdown text is settled and subsequent new text alone reveals', () => {
    const reveal = new TextReveal();
    const history = 'stable code\nprint(1)\ntable cell';
    assert.equal(reveal.update(history, history.length, true, 0).pieces.length, 0);
    const next = reveal.update(history + ' next', history.length, true, 1);
    assert.equal(next.settled, history);
    assert.equal(next.pieces.map(piece => piece.text).join(''), ' next');
});


test('water front follows source order and finishes within a bounded window', () => {
    for (const elapsed of [0, 40, 100, 260, 450, 520]) {
        const head = waterRemaining(elapsed, 0);
        const tail = waterRemaining(elapsed, 1);
        assert.ok(head <= tail);
        assert.ok(head >= 0 && tail <= 1);
        assert.ok(waterRemaining(elapsed, .4, TEXT_COMPLETION_MS) < 1e-10);
    }
    assert.ok(waterRemaining(200, 0, undefined, 250) >= waterRemaining(250, 1));
    const reveal = new TextReveal();
    const batch = reveal.update('one two three four', 0, true, 0);
    const second = reveal.update('one two three four five', 18, true, 40);
    assert.ok(reveal.remaining(second.pieces.at(-1)!, 200) >= reveal.remaining(batch.pieces.at(-1)!, 200));
    const final = reveal.update('one two three four five', 23, false, 50);
    assert.ok(final.pieces.length > 0);
    assert.equal(reveal.advance(50 + TEXT_COMPLETION_MS + 1).pieces.length, 0);
});

test('grapheme groups preserve emoji and accents, including a delta extending a previous cluster', () => {
    const reveal = new TextReveal();
    const cluster = '👨‍👩‍👧‍👦';
    const text = `a${cluster}é🇨🇳👍🏽`;
    const view = reveal.update(text, 0, true, 0);
    const boundaries = new Set([0, text.length, ...Array.from(new Intl.Segmenter(undefined,
        { granularity: 'grapheme' }).segment(text), part => part.index)]);
    let offset = view.settled.length;
    for (const piece of view.pieces) { assert.ok(boundaries.has(offset)); offset += piece.text.length; }
    assert.equal(offset, text.length);
    const split = new TextReveal();
    split.update('cafe', 0, true, 0);
    const joined = split.update('café encore', 4, true, 1);
    assert.equal(joined.settled, 'café');
    assert.equal(visible(joined), 'café encore');
});

test('reduced motion settles immediately and toggling it off never replays history', () => {
    const reveal = new TextReveal();
    reveal.update('new words', 0, true, 0);
    assert.equal(reveal.update('new words', 9, true, 10, true).pieces.length, 0);
    assert.equal(reveal.update('new words', 9, true, 11, false).pieces.length, 0);
    assert.equal(reveal.update('new words next', 9, true, 12).settled, 'new words');
});
