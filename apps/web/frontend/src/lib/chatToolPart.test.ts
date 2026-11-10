import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';

test('actual tool component shows title, summary, preview and structured null independently in every locale', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: ToolPart} = await server.ssrLoadModule('/src/components/ChatToolPart.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        const value = {kind: 'tool', execution_source: {kind: 'model'}, part_id: 'part', revision: 8, capability_id: 'capability', title: '独立标题',
            summary: '人类摘要', output_preview: '技术预览', output: null,
            full_output: {message_id: 'message', part_id: 'part', tool_call_id: 'call'}};
        for (const language of supportedLocales) {
            locale.set(language);
            const html: string = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value}}}).body;
            for (const text of ['独立标题', '人类摘要', '技术预览', '>null</pre>', t(language, 'chatPage.toolPreview'), t(language, 'chatPage.toolResult')]) assert.ok(html.includes(text), text);
            assert.ok(!html.includes('<button'), 'A result descriptor is not an executable permission or reader');
            for (const [state, key] of Object.entries({
                pending: 'toolPending', running: 'toolRunning', completed: 'toolCompleted',
                failed: 'toolFailed', cancelled: 'toolCancelled', sealed: 'toolSealed',
                waiting_for_permission: 'toolWaitingForPermission', waiting_for_input: 'toolWaitingForInput',
                future_protocol_state: 'toolUnknown',
            })) {
                const statusHtml = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value: {...value, state}}}}).body;
                assert.ok(statusHtml.includes(t(language, `chatPage.${key}`)), `${language}: ${state}`);
                assert.ok(!statusHtml.includes('>future_protocol_state<'));
                assert.ok(!statusHtml.includes('<button'), 'A waiting state is not a permission grant');
            }
        }
        const {output: _absentResult, ...missing} = value;
        const absentHtml: string = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value: missing}}}).body;
        assert.doesNotMatch(absentHtml, /class="tool-result(?:\s|")/);
        const emptyHtml: string = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value: {...value, summary: '', output: '', output_preview: ''}}}}).body;
        assert.match(emptyHtml, /class="tool-summary(?:\s|")/);
        assert.match(emptyHtml, /class="tool-preview(?:\s|")/);
        assert.match(emptyHtml, /class="tool-result(?:\s|")/);
        const unsafeHtml: string = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value: {...value, title: '<script>unsafe</script>'}}}}).body;
        assert.ok(!unsafeHtml.includes('<script>unsafe'));
        locale.set('en');
        const nested = render(ToolPart, {props: {part: {source: 'canonical', id: 'nested-write', value: {
            ...value, part_id: 'nested-write', tool_call_id: 'cell:child:1', capability_id: 'write_file',
            execution_source: {kind: 'code_cell', cell: {host_id: 'host', conversation_id: 'conversation',
                turn_id: 'turn', parent_call_id: 'exec-parent', cell_id: 'cell', generation: 'generation'}},
            title: 'Write report', state: 'waiting_for_permission', input: {path: '/tmp/report.txt'},
            permission: {request_id: 'cell:child:1', permission_domain: 'filesystem', rationale: 'Write report', decision: null},
        }}}}).body;
        for (const fact of ['Write report', '/tmp/report.txt', 'filesystem', t('en', 'chatPage.toolWaitingForPermission')]) assert.ok(nested.includes(fact), fact);
        assert.ok(nested.includes('data-tool-id="nested-write"'));
        assert.ok(!nested.includes('<button'), 'Nested provenance does not create independent permission controls');
        const complete = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value: {
            ...value, input: {command: 'echo safe'}, origin: {provider_title: 'Host', namespace: 'shell'},
            artifacts: [{name: 'Report', uri: 'https://example.org/report', mime_type: 'text/plain'}],
        }}}}).body;
        for (const label of ['Tool input', 'echo safe', 'Host · shell · capability', 'Report']) assert.ok(complete.includes(label), label);
        assert.ok(complete.includes('href="https://example.org/report"'));
        const nullInput = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value: {...value, input: null}}}}).body;
        assert.match(nullInput, /class="tool-input/);
        assert.match(nullInput, />null<\/pre>/);
        assert.doesNotMatch(absentHtml, /class="tool-input/);
        for (const language of supportedLocales) {
            locale.set(language);
            const records = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value: {
                ...value, permission: {request_id: 'request', permission_domain: 'files', rationale: '<img src=x> Write report', decision: 'allow_once'},
                affected_resources: [{resource_type: 'file', operation: 'write', uri: 'file:///private/report.txt'}, {resource_type: 'url', operation: 'read', uri: 'javascript:alert(1)'}],
            }}}}).body;
            assert.ok(records.includes(t(language, 'chatPage.toolPermissionRecord')));
            assert.ok(records.includes(t(language, 'chatPage.toolResources')));
            for (const fact of ['files', 'allow_once', 'file:///private/report.txt', 'javascript:alert(1)']) assert.ok(records.includes(fact));
            assert.ok(!records.includes('<img src=x>'));
            assert.ok(!records.includes('<a '), 'Resource facts do not grant URI or local-path access');
            assert.ok(!records.includes('<button'), 'Historical permission facts never become decision controls');
        }
        const invalid = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value: {
            ...value, permission: {permission_domain: true, rationale: 'Invalid'}, affected_resources: [{uri: '/private'}],
        }}}}).body;
        assert.ok(!invalid.includes('<section class="tool-permission'));
        assert.ok(!invalid.includes('<section class="tool-resources'));
        locale.set('en');
        for (const field of ['input', 'output', 'output_preview']) {
            const longText = '<img src=x onerror=alert(1)>' + 'x'.repeat(2500) + 'END_OF_OUTPUT';
            const longValue = {...missing, [field]: longText};
            const longHtml = render(ToolPart, {props: {part: {source: 'canonical', id: 'part', value: longValue}}}).body;
            assert.ok(longHtml.includes('Open in reader'));
            assert.ok(!longHtml.includes('END_OF_OUTPUT'), 'Large text is not duplicated into the closed dialog or inline preview');
            assert.ok(!longHtml.includes('<img src=x'), 'Long output stays escaped');
            assert.ok(!longHtml.includes('<dialog open'), 'Reader requires an explicit action');
        }
    } finally {
        await server.close();
    }
});
