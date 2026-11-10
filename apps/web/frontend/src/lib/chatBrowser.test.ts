import assert from 'node:assert/strict';
import { test } from 'node:test';
import { applyManagedBrowserPages, chatBrowserActivityKey, chatBrowserSnapshot, localBrowserPreviewURL } from './chatBrowser.ts';
import { journalPart } from './journalTimeline.fixture.ts';
import type { TimelineMessage } from './chatTimeline.ts';

function message(parts: TimelineMessage['parts']): TimelineMessage {
    return { id: 'assistant', role: 'assistant', state: 'completed', canonical: null, parts };
}

test('browser activity follows completed typed tool Parts and stop clears the snapshot', () => {
    const identity = {source_store_epoch:'epoch',conversation_id:'conv',run_id:'turn',assistant_message_id:'assistant'};
    const start = journalPart(identity, { kind: 'tool', execution_source: {kind: 'model'}, part_id: 'a',
        capability_id: 'browser_snapshot', state: 'completed', input: {}, output: {
            output_preview: JSON.stringify({ target: 'managed', running: true, pages: [{ index: 2, title: 'Example', url: 'https://example.invalid/', active: true }] }),
        },
    });
    const stopped = journalPart(identity, { kind: 'tool', execution_source: {kind: 'model'}, part_id: 'b',
        capability_id: 'browser_stop', state: 'completed', input: {}, output: {},
    });
    assert.deepEqual(chatBrowserSnapshot([message([start])]), { target: 'managed', connected: true,
        pages: [{ index: 2, title: 'Example', url: 'https://example.invalid/', active: true }], currentUrl: 'https://example.invalid/', previewUrl: '' });
    assert.equal(chatBrowserSnapshot([message([start, stopped])]), null);
    assert.equal(chatBrowserSnapshot([message([{ ...start, value: { ...start.value, state: 'running' } }])]), null);
});

test('canonical tool output is supported without rendering unrelated or malformed pages', () => {
    const part = { source: 'canonical', id: 'a', value: { kind: 'tool', execution_source: {kind: 'model'}, part_id: 'a', revision: 1,
        capability_id: 'chrome_native_tabs', state: 'completed', input: {}, output: { pages: [{ url: 'https://one.invalid', title: 'One' }, { url: '' }] },
    } } as const;
    assert.deepEqual(chatBrowserSnapshot([message([part])]), { target: 'attached', connected: true,
        pages: [{ index: 0, title: 'One', url: 'https://one.invalid', active: false }], currentUrl: 'https://one.invalid', previewUrl: '' });
});

test('an explicit empty page list clears a previously visible browser page', () => {
    const opened = { source: 'canonical', id: 'a', value: { kind: 'tool', execution_source: {kind: 'model'}, part_id: 'a', revision: 1, capability_id: 'browser_tabs', state: 'completed', input: {}, output: {
        running: true, pages: [{ index: 0, title: 'Old', url: 'https://old.invalid', active: true }],
    } } } as const;
    const closed = { source: 'canonical', id: 'b', value: { kind: 'tool', execution_source: {kind: 'model'}, part_id: 'b', revision: 1, capability_id: 'browser_tabs', state: 'completed', input: {}, output: {
        running: true, pages: [],
    } } } as const;
    assert.deepEqual(chatBrowserSnapshot([message([opened, closed])]), {
        target: 'managed', connected: true, pages: [], currentUrl: '', previewUrl: '',
    });
});

test('browser preview address follows the completed tool result and only embeds local host previews', () => {
    const preview = 'http://127.0.0.1:4321/view?token=test';
    const part = { source: 'canonical', id: 'a', value: { kind: 'tool', execution_source: {kind: 'model'}, part_id: 'a', revision: 1,
        capability_id: 'browser_open', state: 'completed', input: {}, output: { preview_url: preview, running: true },
    } } as const;
    assert.equal(chatBrowserSnapshot([message([part])])?.previewUrl, preview);
    assert.equal(localBrowserPreviewURL(preview, 'http://localhost:8080/chat'), preview);
    assert.equal(localBrowserPreviewURL(preview, 'https://nola.example/chat'), null);
    assert.equal(localBrowserPreviewURL('https://example.invalid/', 'http://localhost:8080'), null);
    assert.equal(localBrowserPreviewURL('http://127.0.0.1:4321/#secret', 'http://localhost:8080'), null);
});

test('page switching accepts only typed live browser receipts and replaces active page', () => {
    const original = { target: 'managed' as const, connected: true, pages: [{ index: 0, url: 'https://one.test', title: 'One', active: true }], currentUrl: 'https://one.test', previewUrl: 'http://127.0.0.1:4321/view?token=old' };
    const switched = applyManagedBrowserPages(original, { running: true, pages: [
        { index: 0, url: 'https://one.test', title: 'One', active: false },
        { index: 1, url: 'https://two.test', title: 'Two', active: true },
    ], currentUrl: '', previewUrl: '' });
    assert.equal(switched.currentUrl, 'https://two.test');
    assert.equal(switched.pages[1].active, true);
    assert.equal(switched.previewUrl, original.previewUrl);
    assert.throws(() => applyManagedBrowserPages(original, { running: true, pages: [{ index: -1, url: 'https://two.test', title: 'Two', active: true }], currentUrl: '', previewUrl: '' }));
});

test('unrelated transcript activity does not displace a selected browser page', () => {
    const browserPart = { source: 'canonical', id: 'browser', value: { kind: 'tool', execution_source: {kind: 'model'}, part_id: 'browser', revision: 2,
        capability_id: 'browser_tabs', state: 'completed', output: { pages: [] },
    } } as const;
    const original = [message([browserPart])];
    const unrelated = [...original, { ...message([]), id: 'next' }];
    assert.equal(chatBrowserActivityKey(original), chatBrowserActivityKey(unrelated));
    assert.notEqual(chatBrowserActivityKey(original), chatBrowserActivityKey([message([{ ...browserPart, value: { ...browserPart.value, revision: 3 } }])]));
});
