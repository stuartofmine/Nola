import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { supportedLocales, t } from './i18n.ts';

test('all inspector actions and empty plan states have seven-language labels', () => {
    for (const language of supportedLocales) {
        for (const action of ['progress', 'resources', 'settings', 'close', 'requirements', 'noPlan']) {
            const key = `chatInspector.${action}`;
            assert.notEqual(t(language, key), key);
            assert.ok(t(language, key).trim());
        }
    }
});


test('inspector renders localized tabs, empty conversation state, and plan approval actions', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Inspector} = await server.ssrLoadModule('/src/components/ConversationInspector.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        const emptyUsage = { inputTokens: 0, outputTokens: 0, totalTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0, requests: 0 };
        for (const language of supportedLocales) {
            locale.set(language);
            const idleHtml: string = render(Inspector, {
                props: { conversationId: null, messages: [], usage: emptyUsage, onclose() {} },
            }).body;
            assert.equal((idleHtml.match(/role="tab"/g) ?? []).length, 8);
            assert.equal((idleHtml.match(/aria-selected="true"/g) ?? []).length, 1);
            assert.ok(idleHtml.includes(t(language, 'chatInspector.noConversationSelected')));
            assert.ok(idleHtml.includes(t(language, 'chatInspector.close')));
            assert.ok(!idleHtml.includes('undefined'));

            const pendingPlanHtml: string = render(Inspector, {
                props: {
                    conversationId: 'conv-test',
                    messages: [],
                    usage: emptyUsage,
                    plan: {
                        enabled: true,
                        status: 'awaiting_approval',
                        summary: 'Refactor plan',
                        steps: [{ id: 's1', title: 'Audit endpoints', status: 'pending' }],
                    },
                    onclose() {},
                },
            }).body;
            assert.ok(pendingPlanHtml.includes(t(language, 'chatInspector.planPendingApproval')));
            assert.ok(pendingPlanHtml.includes(t(language, 'chatInspector.planApprove')));
            assert.ok(pendingPlanHtml.includes(t(language, 'chatInspector.planReject')));
            assert.ok(pendingPlanHtml.includes(t(language, 'chatInspector.planClear')));
            assert.ok(pendingPlanHtml.includes('Audit endpoints'));
            assert.ok(pendingPlanHtml.includes(t(language, 'chatInspector.goal')));
            assert.ok(!pendingPlanHtml.includes('undefined'));
        }
    } finally { await server.close(); }
});
