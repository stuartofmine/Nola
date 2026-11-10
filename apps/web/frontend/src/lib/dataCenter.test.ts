import test from 'node:test';
import assert from 'node:assert/strict';
import {sourceOverview} from './dataSources.ts';
import type {DataSource} from './dataSources.ts';
import {createServer} from 'vite';
test('overview counts only verified default sources as ready, treating unknown policies and failures conservatively', () => {
    const source: DataSource = {id: 'one', name: 'one', kind: 'http', location: 'https://example.test', accessPolicy: 'defaultEnabled',
        validationContract: {sourceId: 'one', readerDescription: '', schemaFields: [], redactedSample: '', recordCount: 0, maximumAgeMinutes: 60, validationState: 'verifiedEmpty', validatedAt: ''}};
    const rows = [source, {...source, id: 'blocked', accessPolicy: 'blocked'}, {...source, id: 'unknown', accessPolicy: 'future'},
        {...source, id: 'failed', lastError: 'error'}, {...source, id: 'opt-in', accessPolicy: 'conversationOptIn'}, {...source, id: 'pending', validationContract: null}];
    const overview = sourceOverview(rows);
    assert.equal(overview.connected, 6);
    assert.equal(overview.ready, 1);
    assert.deepEqual(overview.attention.map(source => source.id), ['blocked', 'unknown', 'failed', 'pending']);
    assert.deepEqual(sourceOverview([]), {connected: 0, ready: 0, attention: []});
});
test('data center renders four real labelled panels and no fabricated initial source totals', async () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    Object.defineProperty(globalThis, 'window', { configurable: true, value: {
        innerWidth: 1280, location: { href: 'http://nola.invalid/data-sources' },
    } });
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/pages/DataSources.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of supportedLocales) {
            locale.set(language);
            const html = render(Component).body;
            assert.equal((html.match(/role="tab"/g) ?? []).length, 4);
            assert.equal((html.match(/role="tabpanel"/g) ?? []).length, 4);
            assert.ok(!html.includes('class="source-stats"'));
            for (const pane of ['sources', 'memory', 'plans', 'usage']) {
                assert.ok(html.includes(`aria-controls="data-panel-${pane}"`));
                assert.ok(html.includes(t(language, `data_sources.tab.${pane}`)));
            }
        }
    } finally {
        await server.close();
        if (previousWindow) Object.defineProperty(globalThis, 'window', previousWindow);
        else Reflect.deleteProperty(globalThis, 'window');
    }
});
