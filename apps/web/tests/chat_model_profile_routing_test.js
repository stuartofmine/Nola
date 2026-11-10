const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const template = fs.readFileSync(path.join(__dirname, '../app/templates/chat.html'), 'utf8');
const section = (start, end) => {
    const from = template.indexOf(start);
    const to = template.indexOf(end, from);
    assert.ok(from >= 0 && to > from, `production section: ${start}`);
    return template.slice(from, to).replace(/\{\{[\s\S]*?\}\}/g, 'message');
};
const errors = [];
let panelsOpened = 0;
const context = {
    PROFILE_MODEL_PREFIX: '__nola_model_profile__:', SAVED_API_OPTION_VALUE: '__saved__', MANAGED_LOCAL_PREFIX: 'local:',
    modelSelect: { value: '' }, cloudModelsCache: ['cloud-model'],
    managedLocalModelsCache: [{ id: 'local-model', running: true }],
    remoteModelProfilesCache: [{ id: 'profile/中文', enabled: true, endpoint: { enabled: true }, capabilities: ['chat'] }],
    getCustomConfig: () => null, hasUsableCustomConfig: value => !!value,
    showError: message => errors.push(message), openLocalLlmConfigPanel: () => panelsOpened++,
    resolveAgentMaxIterations: () => 10, currentMessages: [], currentResponseStyle: () => 'balanced',
    document: { documentElement: { lang: 'zh' } }, navigator: { language: 'zh' },
    crypto: { randomUUID: () => 'submission' },
};
vm.createContext(context);
vm.runInContext(section('function selectedModelSettingFromValue(', 'function applyConversationModelSelection('), context);
vm.runInContext(section('function resolveSelectedModelChoice()', '// ---- State ----'), context);

context.remoteModelProfilesCache[0].selected = true;
assert.equal(context.resolveSelectedModelChoice().modelProfileId, 'profile/中文', 'Auto resolves the canonical default, not singleton settings');
context.remoteModelProfilesCache[0].enabled = false;
assert.equal(context.validateSelectedModelChoice(context.resolveSelectedModelChoice()), false, 'Unavailable default must not silently fall back');
context.remoteModelProfilesCache[0].enabled = true;
context.modelSelect.value = 'cloud-model';
assert.equal(context.resolveSelectedModelChoice().validModel, 'cloud-model', 'Explicit conversation model outranks global default');

context.modelSelect.value = 'local:local-model';
assert.equal(context.validateSelectedModelChoice(context.resolveSelectedModelChoice()), true);
assert.equal(panelsOpened, 0, 'running local models must not require an API endpoint');
context.managedLocalModelsCache[0].running = false;
assert.equal(context.validateSelectedModelChoice(context.resolveSelectedModelChoice()), false);
assert.equal(panelsOpened, 0, 'stopped local models need Start, not an API settings panel');

context.modelSelect.value = '__saved__';
assert.equal(context.validateSelectedModelChoice(context.resolveSelectedModelChoice()), false);
assert.equal(panelsOpened, 1);

context.modelSelect.value = context.PROFILE_MODEL_PREFIX + 'profile/中文';
const selected = context.resolveSelectedModelChoice();
assert.equal(selected.modelProfileId, 'profile/中文');
assert.equal(context.validateSelectedModelChoice(selected), true);
const persisted = context.selectedModelSettingFromValue();
assert.equal(context.conversationModelSelectValue(persisted), context.modelSelect.value);
context.remoteModelProfilesCache[0].enabled = false;
assert.equal(context.validateSelectedModelChoice(selected), false);
context.remoteModelProfilesCache = [];
assert.equal(context.validateSelectedModelChoice(selected), false, 'missing profiles cannot fall back to another endpoint');

// Execute the real request-builder prefix, stopping before streaming/DOM work.
vm.runInContext(section('async function runAgentV2Stream(', '    const shell = ensureAssistantStreamShell(bubbleEl);') + '\nreturn body;\n}', context);
(async () => {
    const body = await context.runAgentV2Stream({ modelProfileId: 'profile/中文', conv: { id: 'conversation' }, userTz: 'UTC' });
    assert.equal(body.modelProfileId, 'profile/中文');
    assert.equal(body.model, undefined);
    assert.equal(body.customApiKey, undefined, 'canonical selection does not copy singleton credentials into browser requests');
    console.log('chat model profile routing: passed');
})().catch(error => { console.error(error); process.exitCode = 1; });
