import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { timelineToolsNeedAttention } from './chatTimeline.ts';
import type { TimelinePart } from './chatTimeline.ts';
import { journalPart } from './journalTimeline.fixture.ts';

const part = (state: string): TimelinePart => ({source: 'canonical', id: 'tool', value: {
    kind: 'tool', execution_source: {kind: 'model'}, part_id: 'tool', revision: 1, state, title: 'Run command', output_preview: 'Missing dependency',
}});

test('typed failure and user waits request disclosure without changing tool facts', () => {
    for (const state of ['failed', 'waiting_for_permission', 'waiting_for_input']) {
        assert.equal(timelineToolsNeedAttention([part(state)]), true);
    }
    for (const state of ['running', 'completed', 'sealed', 'cancelled', 'unknown']) {
        assert.equal(timelineToolsNeedAttention([part(state)]), false);
    }
});

test('tool groups expose failures without automatically opening raw diagnostics', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatToolGroup.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const failed = render(Component, {props: {parts: [part('failed')]}}).body;
        assert.match(failed, /<details open/);
        assert.doesNotMatch(failed, /data-state="failed" open/);
        assert.match(failed, /class="tool-technical/);
        assert.ok(failed.includes('Missing dependency'));
        const completed = render(Component, {props: {parts: [part('completed')]}}).body;
        assert.doesNotMatch(completed, /<details[^>]*\sopen(?:=|\s|>)/);
    } finally { await server.close(); }
});

test('journal failure diagnostics are disclosed without treating ordinary steps as errors', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatJournalDetails.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        locale.set('en');
        const identity = {source_store_epoch: 'epoch', conversation_id: 'conversation', run_id: 'run', assistant_message_id: 'assistant'};
        for (const value of [{kind: 'error', failure: 'Transport failed'}, {kind: 'model_attempt', state: 'failed', failure: 'Transport failed'}]) {
            const html = render(Component, {props: {part: journalPart(identity, {part_id: 'part', ...value})}}).body;
            if (value.kind === 'error') assert.match(html, /<details[^>]*\sopen(?:=|\s|>)/);
            else assert.doesNotMatch(html, /<details[^>]*\sopen(?:=|\s|>)/, 'past provider attempts stay collapsed after a successful retry');
            assert.ok(html.includes('Transport failed'));
            assert.ok(html.includes(t('en', 'chatPage.toolFailed')), 'Failure must be labelled even without Activity.state');
        }
        const html = render(Component, {props: {part: journalPart(identity, {part_id: 'step', kind: 'step', state: 'completed'})}}).body;
        assert.doesNotMatch(html, /<details[^>]*\sopen(?:=|\s|>)/);
    } finally { await server.close(); }
});
