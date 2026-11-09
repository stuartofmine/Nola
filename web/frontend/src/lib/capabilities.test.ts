import { test } from 'node:test';
import assert from 'node:assert/strict';
import { builtin, capabilitySnapshot, capabilityCatalog, capabilityDraft, capabilityMutation, updatedCapability, capabilitySections, restoredSkill, CapabilityValidationError, capabilityName } from './capabilities.ts';
import { capabilityMessages } from './locales/capabilities.ts';

test('capability display labels and validation errors resolve in all seven languages', () => {
    const groups = capabilitySections([{ item: { name: 'fixture' }, index: 0 }], 'skills');
    const catalog = capabilityCatalog({});
    const keys = Object.keys(capabilityMessages.en).sort();
    assert.equal(Object.keys(capabilityMessages).length, 7);
    for (const messages of Object.values(capabilityMessages)) {
        assert.deepEqual(Object.keys(messages).sort(), keys);
        for (const key of keys) {
            assert.ok(messages[key]);
            assert.deepEqual(messages[key].match(/\{\w+\}/g) ?? [], capabilityMessages.en[key].match(/\{\w+\}/g) ?? []);
        }
        for (const group of [...groups, ...catalog]) assert.ok(messages[group.titleKey]);
    }
    assert.throws(() => updatedCapability({}, { name: ' ', description: '', primary: '', enabled: true }, 'skills'),
        (error: unknown) => error instanceof CapabilityValidationError && error.key === 'capability.nameRequired');
    for (const primary of ['invalid', 'file:///tmp/file', 'https://user:secret@example.com']) {
        assert.throws(() => updatedCapability({}, { name: 'MCP', description: '', primary, enabled: true }, 'mcp'),
            (error: unknown) => error instanceof CapabilityValidationError && error.key === 'capability.invalidEndpoint');
    }
    assert.throws(() => restoredSkill({}),
        (error: unknown) => error instanceof CapabilityValidationError && error.key === 'capability.noDefaultContent');
});

test('translations remain display fallbacks and do not replace user content or MCP metadata', () => {
    const item = { name: '用户 Skill', description: 'Description utilisateur', content: '\nKeep this exactly\n' };
    assert.equal(capabilityName(item, 'Translated fallback'), item.name);
    assert.equal(capabilityName({}, 'Translated fallback'), 'Translated fallback');
    assert.equal(capabilityDraft(item).description, item.description);
    assert.equal(capabilityDraft(item).primary, item.content);
    assert.equal(capabilityDraft({}).name, '');
    const descriptor = { name: 'user_tool', description: '説明はそのまま' };
    assert.deepEqual(capabilityCatalog({ tools: { key: descriptor } })[0].entries,
        [{ id: 'key', name: descriptor.name, description: descriptor.description }]);
});

test('canonical built_in flags protect built-in skills and restoration preserves disabled state', () => {
    const original = { id: 'document', built_in: true, source: 'built_in', content: 'custom', default_content: '\noriginal\n', enabled: false };
    assert.equal(builtin(original), true);
    assert.equal(builtin({ source: 'built_in' }), true);
    assert.deepEqual(restoredSkill(original), { ...original, content: '\noriginal\n' });
    assert.equal(original.content, 'custom');
    assert.throws(() => restoredSkill({ source: 'custom', default_content: 'not built in' }));
    assert.throws(() => restoredSkill({ built_in: true }));
});

test('category order matches Mac and sorted rows retain original mutation indices', () => {
    const entries = [
        { index: 0, item: { name: 'Custom', category_id: 'documents' } },
        { index: 1, item: { name: 'Z', built_in: true, category_id: 'workspace' } },
        { index: 2, item: { name: 'B', built_in: true, category_id: 'documents' } },
        { index: 3, item: { name: 'A', built_in: true, category_id: 'documents' } },
    ];
    const groups = capabilitySections(entries, 'skills');
    assert.deepEqual(groups.map(group => group.id), ['documents', 'workspace', 'custom']);
    assert.deepEqual(groups.map(group => group.entries.map(entry => entry.index)), [[3, 2], [1], [0]]);
    assert.deepEqual(entries.map(entry => entry.index), [0, 1, 2, 3]);
    assert.equal(capabilitySections(entries, 'mcp').length, 1);
});

test('MCP edits preserve discovery metadata and send the original revision baseline only', () => {
    const server = { server_id: 'opaque', revision: 7, name: 'MCP', endpoint: 'https://fixture.invalid', tools: { read: { name: 'Read' } }, enabled: false };
    const settings = { skills: [{ name: 'unrelated' }], mcp_servers: [server] };
    const draft = capabilityDraft(server); draft.enabled = true;
    const replacement = updatedCapability(server, draft, 'mcp');
    const payload = capabilityMutation(settings, 'mcp', 0, replacement);
    assert.deepEqual(payload, { mcp_servers: [{ ...server, enabled: true, description: '', display_name: 'MCP' }], mcp_baseline: [{ server_id: 'opaque', revision: 7 }] });
    assert.equal(server.enabled, false);
    const local = { server_id: 'local', name: 'Local', command: ['fixture', '--stdio'], environment: { FIXTURE: 'private-value' }, enabled: false };
    const launch = capabilityDraft(local);
    assert.equal(launch.primary.includes('private-value'), false);
    launch.enabled = true;
    assert.deepEqual(updatedCapability(local, launch, 'mcp'), { ...local, enabled: true, display_name: 'Local', description: '' });
    launch.primary = '{"command":"fixture","args":["--stdio"],"cwd":"/tmp"}';
    assert.deepEqual(updatedCapability(local, launch, 'mcp').args, ['--stdio']);
    launch.primary = '{"command":["fixture"],"args":["ambiguous"]}';
    assert.throws(() => updatedCapability(local, launch, 'mcp'), (error: unknown) => error instanceof CapabilityValidationError && error.key === 'capability.invalidLaunch');

    assert.deepEqual(capabilityCatalog(replacement)[0].entries, [{ id: 'read', name: 'Read', description: '' }]);
});

test('skill create edit and delete never submit unrelated settings', () => {
    const settings = { skills: [{ id: 'first', revision: 4, name: 'first', content: 'unchanged', builtin: true }, { id: 'second', revision: 7, name: 'second' }], mcp_servers: [{ name: 'private' }] };
    assert.deepEqual(capabilityMutation(settings, 'skills', null, { name: 'new' }), { skills: [...settings.skills, { name: 'new' }], skill_baseline: [{ id: 'first', revision: 4 }, { id: 'second', revision: 7 }] });
    assert.deepEqual(capabilityMutation(settings, 'skills', 1), { skills: [settings.skills[0]], skill_baseline: [{ id: 'first', revision: 4 }, { id: 'second', revision: 7 }] });
    assert.equal(settings.skills.length, 2);
    assert.equal(builtin(settings.skills[0]), true);
});

test('invalid edits fail before transport and instruction whitespace is preserved', () => {
    assert.throws(() => updatedCapability({}, { name: ' ', description: '', primary: '', enabled: true }, 'skills'));
    for (const endpoint of ['file:///tmp/server', 'https://user:secret@fixture.invalid', 'not a URL']) {
        assert.throws(() => updatedCapability({}, { name: 'MCP', description: '', primary: endpoint, enabled: true }, 'mcp'));
    }
    const draft = { name: ' Skill ', description: ' Details ', primary: '\n  content\n', enabled: false };
    assert.deepEqual(updatedCapability({ id: 'stable' }, draft, 'skills'), { id: 'stable', name: 'Skill', description: 'Details', content: '\n  content\n', enabled: false });
});

test('MCP catalog rejects non-object groups and renders descriptors as text', () => {
    const groups = capabilityCatalog({ tools: [], resources: null, prompts: { a: { title: '<script>not markup</script>' } } });
    assert.deepEqual(groups.map(group => group.entries.length), [0, 0, 0, 1]);
    assert.equal(groups[3].entries[0].name, '<script>not markup</script>');
});


test('capability snapshot rejects incomplete catalogs atomically instead of crashing the renderer', () => {
    const snapshot = {skill_settings_backup_key: null, skills: [{name: 'Read'}], mcp_servers: [], execution_backends: [{id: 'nola', display_name: 'Nola', capabilities: {supports_permission_bridge: true}}]};
    assert.deepEqual(capabilitySnapshot(snapshot), snapshot);
    for (const value of [null, {}, {...snapshot, execution_backends: undefined}, {...snapshot, skills: [null]},
        {...snapshot, execution_backends: [{id: 'nola', display_name: 'Nola', capabilities: {}}]},
        {...snapshot, execution_backends: [...snapshot.execution_backends, ...snapshot.execution_backends]}]) {
        assert.throws(() => capabilitySnapshot(value), error => error instanceof CapabilityValidationError && error.key === 'capability.loadFailed');
    }
});
