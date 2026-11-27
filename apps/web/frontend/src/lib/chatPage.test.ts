import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';

test('production Chat layout renders through its actual Svelte route bundle', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: Chat } = await server.ssrLoadModule('/src/pages/Chat.svelte');
        const { default: Inspector } = await server.ssrLoadModule('/src/components/ConversationInspector.svelte');
        const { default: BrowserPdfPanel } = await server.ssrLoadModule('/src/components/InspectorBrowserPdfPanel.svelte');
        const { default: ArtifactsPanel } = await server.ssrLoadModule('/src/components/InspectorArtifactsPanel.svelte');
        const { default: RecallPanel } = await server.ssrLoadModule('/src/components/InspectorRecallPanel.svelte');
        const { default: ExpertRoomManager } = await server.ssrLoadModule('/src/components/ExpertRoomManager.svelte');
        const { default: ListNavigation } = await server.ssrLoadModule('/src/components/WorkspaceListNavigation.svelte');
        const { default: ResourceNavigation } = await server.ssrLoadModule('/src/components/WorkspaceResourceNavigation.svelte');
        const { default: LanguagePreference } = await server.ssrLoadModule('/src/components/LanguagePreference.svelte');
        const { locale, localePreference, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        // Use the renderer from Vite's graph so the Svelte runtime and compiled
        // component share their SSR context rather than mixing module instances.
        const { render } = await server.ssrLoadModule('svelte/server');
        const emptyUsage = { inputTokens: 0, outputTokens: 0, totalTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0, requests: 0 };

        for (const language of supportedLocales) {
            localePreference.set('system');
            locale.set(language);
            const html: string = render(Chat).body;
            assert.match(html, /class="chat-shell(?:\s|")/);
            assert.match(html, /class="chat-sidebar(?:\s|")/);
            assert.match(html, /class="transcript(?:\s|")/);
            assert.match(html, /id="chat-composer"/);
            assert.ok(html.includes(t(language, 'chatPage.welcome')));
            assert.ok(html.includes(t(language, 'chatPage.choose')));
            assert.ok(html.includes(t(language, 'chatPage.send')));
            assert.ok(html.includes(t(language, 'chatPage.details')));
            assert.ok(html.includes(t(language, 'shell.settings')));
            assert.equal((html.match(/class="conversation-manager(?:\s|")/g) ?? []).length, 1);
            assert.equal((html.match(/class="new-conversation(?:\s|")/g) ?? []).length, 1, 'The current conversation list owns the only create control');
            assert.doesNotMatch(html, /id="conversation-inspector-panel"/, 'Inspector starts collapsed by default');
            assert.ok(!html.includes('undefined'));

            const listHtml: string = render(ListNavigation, { props: { category: 'conversations', onchange: () => {} } }).body;
            for (const category of ['conversations', 'resources']) {
                assert.ok(listHtml.includes(t(language, `chatInspector.list.${category}`)));
            }
            assert.equal((listHtml.match(/aria-pressed="true"/g) ?? []).length, 1);

            const resourceHtml: string = render(ResourceNavigation, { props: { selectedPath: '/models', onopen: () => {} } }).body;
            for (const href of ['/today', '/activity', '/automations', '/data-sources', '/models', '/capabilities']) {
                assert.ok(resourceHtml.includes(`href="${href}"`), href);
            }
            assert.equal((resourceHtml.match(/aria-current="page"/g) ?? []).length, 1);

            const languageHtml: string = render(LanguagePreference).body;
            assert.ok(languageHtml.includes(t(language, 'shell.language')));
            assert.ok(languageHtml.includes(t(language, 'shell.language.system')));
            for (const choice of supportedLocales) assert.ok(languageHtml.includes(`value="${choice}"`));

            const inspectorHtml: string = render(Inspector, {
                props: { conversationId: 'conv-test', messages: [], usage: emptyUsage, onclose: () => {} },
            }).body;
            assert.match(inspectorHtml, /id="conversation-inspector-panel"/);
            assert.ok(inspectorHtml.includes(t(language, 'chatInspector.planMode')));
            assert.ok(inspectorHtml.includes(t(language, 'chatInspector.goal')));
            assert.ok(!inspectorHtml.includes('undefined'));

            const browserPdfHtml: string = render(BrowserPdfPanel, {
                props: { conversationId: 'conv-test', messages: [] },
            }).body;
            assert.ok(browserPdfHtml.includes(t(language, 'chatBrowser.title')));
            assert.ok(browserPdfHtml.includes(t(language, 'chatBrowser.noPages')));
            assert.ok(browserPdfHtml.includes(t(language, 'chatResource.title')));
            assert.ok(browserPdfHtml.includes(t(language, 'chatResource.empty')));
            assert.ok(!browserPdfHtml.includes('undefined'));

            const artifactsHtml: string = render(ArtifactsPanel, {
                props: { conversationId: 'conv-test', messages: [] },
            }).body;
            assert.ok(!artifactsHtml.includes('undefined'));

            const recallHtml: string = render(RecallPanel, {
                props: { conversationId: 'conv-test' },
            }).body;
            assert.ok(!recallHtml.includes('undefined'));

            const roomsHtml: string = render(ExpertRoomManager, {
                props: { standalone: true },
            }).body;
            assert.ok(roomsHtml.includes(t(language, 'rooms.manageExperts')));
            assert.ok(roomsHtml.includes(t(language, 'rooms.experts')));
            assert.ok(roomsHtml.includes(t(language, 'rooms.title')));
            assert.ok(!roomsHtml.includes('undefined'));
        }
    } finally {
        await server.close();
    }
});
