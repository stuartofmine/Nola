import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';

test('production Chat layout renders through its actual Svelte route bundle', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: Chat } = await server.ssrLoadModule('/src/pages/Chat.svelte');
        const { default: PdfResources } = await server.ssrLoadModule('/src/components/ChatPdfResources.svelte');
        const { default: DataSources } = await server.ssrLoadModule('/src/components/ChatDataSources.svelte');
        const { default: Overview } = await server.ssrLoadModule('/src/components/ChatOverview.svelte');
        const { default: InspectorTools } = await server.ssrLoadModule('/src/components/ChatInspectorTools.svelte');
        const { default: Browser } = await server.ssrLoadModule('/src/components/ChatBrowser.svelte');
        const { default: ListNavigation } = await server.ssrLoadModule('/src/components/WorkspaceListNavigation.svelte');
        const { default: ResourceNavigation } = await server.ssrLoadModule('/src/components/WorkspaceResourceNavigation.svelte');
        const { default: LanguagePreference } = await server.ssrLoadModule('/src/components/LanguagePreference.svelte');
        const { parseChatConversation } = await server.ssrLoadModule('/src/lib/chatConversations.ts');
        const { locale, localePreference, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        // Use the renderer from Vite's graph so the Svelte runtime and compiled
        // component share their SSR context rather than mixing module instances.
        const { render } = await server.ssrLoadModule('svelte/server');
        for (const language of supportedLocales) {
            localePreference.set('system'); locale.set(language);
            const html: string = render(Chat).body;
            assert.doesNotMatch(html, /id="chat-composer"/);
            assert.match(html, /class="transcript(?:\s|\")/);
            assert.ok(html.includes(t(language, 'chatPage.title')));
            assert.ok(html.includes(t(language, 'chatPage.loading')));
            assert.doesNotMatch(html, /id="chat-details"/);
            assert.ok(html.includes('href="/models"'));
            assert.ok(html.includes('href="/data-sources"'));
            assert.match(html, /class="workspace-identity(?:\s|")/);
            assert.ok(html.includes(t(language, 'shell.workspace')));
            assert.match(html, /class="global-navigation(?:\s|")/);
            assert.match(html, /class="compact-list-categories(?:\s|")/);
            assert.equal((html.match(/class="conversation-manager(?:\s|")/g) ?? []).length, 1);
            assert.equal((html.match(/class="new-conversation(?:\s|")/g) ?? []).length, 1, 'The current conversation list owns the only create control');
            assert.doesNotMatch(html, /class="workspace-sidebar/);
            assert.doesNotMatch(html, /id="chat-panel-(knowledge|browser|tools)"/);
            assert.ok(html.includes(t(language, 'chatPage.find')) === false, 'Search opens only for an active conversation');
            assert.ok(!html.includes('undefined'));
            const listHtml: string = render(ListNavigation, { props: { category: 'conversations', onchange: () => {} } }).body;
            for (const category of ['conversations', 'resources']) assert.ok(listHtml.includes(t(language, `chatInspector.list.${category}`)));
            assert.equal((listHtml.match(/aria-pressed="true"/g) ?? []).length, 1);
            const resourceHtml: string = render(ResourceNavigation, { props: { selectedPath: '/models', onopen: () => {} } }).body;
            for (const href of ['/data-sources', '/models', '/capabilities']) assert.ok(resourceHtml.includes(`href="${href}"`));
            assert.equal((resourceHtml.match(/aria-current="page"/g) ?? []).length, 1);
            const languageHtml: string = render(LanguagePreference).body;
            assert.ok(languageHtml.includes(t(language, 'shell.language')));
            assert.ok(languageHtml.includes(t(language, 'shell.language.system')));
            for (const choice of supportedLocales) assert.ok(languageHtml.includes(`value="${choice}"`));
            const pdfHtml: string = render(PdfResources, { props: { conversationId: 'conv-test', generation: 0 } }).body;
            assert.ok(pdfHtml.includes(t(language, 'chatResource.previewOnly')));
            assert.ok(pdfHtml.includes(t(language, 'chatResource.upload')));
            const sourcesHtml: string = render(DataSources, { props: { conversationId: 'conv-test', generation: 0 } }).body;
            assert.ok(sourcesHtml.includes(t(language, 'chatDataSources.title')));
            assert.ok(sourcesHtml.includes(t(language, 'chatDataSources.scope')));
            assert.ok(sourcesHtml.includes(t(language, 'chatDataSources.inherit')) === false, 'No controls render without a canonical source');
            const conversation = parseChatConversation({
                isPinned: false, conversationId: 'conv-test', title: 'Test', createdAt: '', updatedAt: '',
                selectedModelSource: 'auto', selectedModelId: '', executionBackend: 'nola', revision: 1, configurationRevision: 1,
            });
            const overviewHtml: string = render(Overview, { props: { conversation, generation: 0 } }).body;
            assert.ok(overviewHtml.includes(t(language, 'chatOverview.notRecorded')));
            const compact = render(Overview, {props: {conversation, compact: true, selectedModelName: 'MODEL-SENTINEL'}}).body;
            assert.ok(!compact.includes('class="status"') && !compact.includes('class="model"'));
            assert.ok(compact.includes(t(language, 'chatOverview.cacheUnknown')));
            assert.ok(overviewHtml.includes(t(language, 'chatOverview.cacheUnknown')));
            const cacheProps = { conversation, cache: { input_tokens: 2000, read_tokens: 900, write_tokens: null,
                hit_rate_bps: 4500, is_compaction: false, reported_requests: 3, unreported_requests: 2 } };
            const cumulativeHtml: string = render(Overview, { props: cacheProps }).body;
            assert.ok(cumulativeHtml.includes(t(language, 'chatOverview.cacheRate')));
            assert.ok(cumulativeHtml.includes(t(language, 'chatOverview.cacheReported')));
            assert.ok(cumulativeHtml.includes(t(language, 'chatOverview.cacheUnreported')));
            assert.ok(cumulativeHtml.includes(t(language, 'chatOverview.cacheCoverage')));
            assert.ok(cumulativeHtml.includes(new Intl.NumberFormat(language,
                { style: 'percent', minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(0.45)));
            const unknownHtml: string = render(Overview, { props: { ...cacheProps, cache: {
                ...cacheProps.cache, input_tokens: null, read_tokens: null, hit_rate_bps: null,
                reported_requests: 0, unreported_requests: 2,
            } } }).body;
            assert.ok(unknownHtml.includes(t(language, 'chatOverview.cacheUnknown')));
            assert.ok(unknownHtml.includes(t(language, 'chatOverview.cacheUnreported')));
            const failedHtml: string = render(Overview, { props: { ...cacheProps, cache: null, cacheFailed: true } }).body;
            assert.ok(failedHtml.includes(t(language, 'chatOverview.cacheUnavailable')));
            assert.ok(!overviewHtml.includes('0%'));
            const toolsHtml: string = render(InspectorTools).body;
            assert.ok(toolsHtml.includes(t(language, 'chatTools.title')));
            assert.ok(toolsHtml.includes(t(language, 'chatTools.catalogHint')));
            assert.ok(toolsHtml.includes('href="/settings"'));
            assert.ok(toolsHtml.includes('href="/capabilities"'));
            const browserHtml: string = render(Browser, { props: { messages: [], conversationId: 'conv-test', generation: 0 } }).body;
            assert.ok(browserHtml.includes(t(language, 'chatBrowser.title')));
            assert.ok(browserHtml.includes(t(language, 'chatBrowser.noPages')));
            assert.ok(browserHtml.includes(t(language, 'chatBrowser.previewPending')));
        }
    } finally {
        await server.close();
    }
});
