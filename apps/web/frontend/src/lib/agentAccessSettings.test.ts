import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agentAccessSnapshot, agentAccessReceipt, agentAccessDraft, agentAccessDirty, agentAccessPatch, knownPermissionProfile, requiresAllowAllConfirmation, permissionProfiles } from './agentAccessSettings.ts';
import { agentAccessMessages } from './locales/agentAccessSettings.ts';

const response = () => ({ permission_profile: 'cautious', custom_system_prompt: ' original\n', enable_local_dev_tools: false,
    allowed_dirs: ['/tmp/work'], bash_prefixes: ['git status'], temperature: 0.3, skills: [{ name: 'private' }], mcp_servers: [] });

test('access editor reads canonical shell prefixes and strips unrelated or sensitive fields', () => {
    const source = response();
    const snapshot = agentAccessSnapshot(source);
    source.allowed_dirs.push('/new');
    assert.deepEqual(snapshot.allowed_dirs, ['/tmp/work']);
    assert.equal(agentAccessDraft(snapshot).bashPrefixes, 'git status');
    assert.equal('temperature' in snapshot, false);
    assert.equal('skills' in snapshot, false);
    const { bash_prefixes, ...old } = response();
    assert.throws(() => agentAccessSnapshot({ ...old, bash_allowed_prefixes: bash_prefixes }));
    assert.throws(() => agentAccessSnapshot({ ...old, allowed_bash_prefixes: bash_prefixes }));
});

test('save receipt confirms every submitted field without accepting stale or partial application', () => {
    const initial = agentAccessSnapshot(response());
    const patch = { permission_profile: 'allow_all', custom_system_prompt: ' new\n', enable_local_dev_tools: true,
        allowed_dirs: ['/one', '/two'], bash_prefixes: ['git status', 'git diff'] };
    assert.deepEqual(agentAccessReceipt({ ...initial, ...patch }, patch), { ...initial, ...patch });
    for (const key of Object.keys(patch) as (keyof typeof patch)[]) {
        assert.throws(() => agentAccessReceipt({ ...initial, ...patch, [key]: initial[key] }, patch));
    }
    assert.throws(() => agentAccessReceipt({ ...initial, ...patch, custom_system_prompt: 'new' }, patch));
    assert.throws(() => agentAccessReceipt({ ...initial, ...patch, allowed_dirs: ['/two', '/one'] }, patch));
    // Concurrent unrelated values may be adopted, but never sent back in the sparse patch.
    assert.equal(agentAccessReceipt({ ...initial, custom_system_prompt: 'Other editor', bash_prefixes: [] }, { bash_prefixes: [] }).custom_system_prompt, 'Other editor');
});

test('saving only changed owned fields cannot rewrite model or MCP settings', () => {
    const snapshot = agentAccessSnapshot(response());
    const draft = agentAccessDraft(snapshot);
    assert.equal(agentAccessDirty(snapshot, draft), false);
    assert.deepEqual(agentAccessPatch(snapshot, draft), {});
    draft.customSystemPrompt = ' new instructions\n';
    assert.deepEqual(agentAccessPatch(snapshot, draft), { custom_system_prompt: ' new instructions\n' });
    draft.allowedDirs = '/tmp/new\n\n /tmp/second ';
    draft.bashPrefixes = '';
    draft.enableLocalDevTools = true;
    assert.deepEqual(agentAccessPatch(snapshot, draft), { custom_system_prompt: ' new instructions\n',
        allowed_dirs: ['/tmp/new', '/tmp/second'], bash_prefixes: [], enable_local_dev_tools: true });
    assert.deepEqual(snapshot.bash_prefixes, ['git status']);
    assert.equal(snapshot.custom_system_prompt, ' original\n');
});

test('unknown permission profiles remain explicit and cannot silently broaden access', () => {
    for (const profile of ['', 'moderate', 'ALLOW_ALL', 'allow-all', 'future_mode']) {
        const snapshot = agentAccessSnapshot({ ...response(), permission_profile: profile });
        const draft = agentAccessDraft(snapshot);
        assert.equal(draft.permissionProfile, profile);
        assert.equal(knownPermissionProfile(profile), false);
        assert.throws(() => agentAccessPatch(snapshot, draft), /agentAccess.unknownProfile/);
        draft.permissionProfile = 'cautious';
        assert.deepEqual(agentAccessPatch(snapshot, draft), { permission_profile: 'cautious' });
    }
});

test('only transitions into allow_all require destructive-operation confirmation', () => {
    const snapshot = agentAccessSnapshot(response());
    for (const permissionProfile of permissionProfiles) {
        const draft = { ...agentAccessDraft(snapshot), permissionProfile };
        assert.equal(requiresAllowAllConfirmation(snapshot, draft), permissionProfile === 'allow_all');
        assert.equal('network_policy' in agentAccessPatch(snapshot, draft), false);
    }
    const alreadyAllowed = { ...snapshot, permission_profile: 'allow_all' };
    assert.equal(requiresAllowAllConfirmation(alreadyAllowed, agentAccessDraft(alreadyAllowed)), false);
});

test('invalid server snapshots fail without mutating the existing draft', () => {
    const snapshot = agentAccessSnapshot(response());
    const draft = { ...agentAccessDraft(snapshot), customSystemPrompt: 'retain me' };
    for (const invalid of [null, {}, { ...response(), allowed_dirs: [1] }, { ...response(), bash_prefixes: null, bash_allowed_prefixes: 3 }, { ...response(), enable_local_dev_tools: 'false' }]) {
        assert.throws(() => agentAccessSnapshot(invalid), /agentAccess.invalidSnapshot/);
    }
    assert.equal(draft.customSystemPrompt, 'retain me');
});

test('access messages cover seven languages and permission details with matching placeholders', () => {
    const keys = Object.keys(agentAccessMessages.en).sort();
    assert.equal(Object.keys(agentAccessMessages).length, 7);
    for (const messages of Object.values(agentAccessMessages)) {
        assert.deepEqual(Object.keys(messages).sort(), keys);
        for (const key of keys) {
            assert.ok(messages[key]);
            assert.deepEqual(messages[key].match(/\{\w+\}/g) ?? [], agentAccessMessages.en[key].match(/\{\w+\}/g) ?? []);
        }
        for (const profile of permissionProfiles) assert.ok(messages[`agentAccess.detail.${profile}`]);
    }
});
