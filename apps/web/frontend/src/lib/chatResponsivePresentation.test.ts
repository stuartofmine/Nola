import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// Source guardrails only. Final browser acceptance must separately exercise
// focus trapping/restoration, keyboard opening, resize, IME and large text.
const chat = readFileSync(new URL('../pages/Chat.svelte', import.meta.url), 'utf8');
const inspector = readFileSync(new URL('../components/ConversationInspector.svelte', import.meta.url), 'utf8');
const roomManager = readFileSync(new URL('../components/ExpertRoomManager.svelte', import.meta.url), 'utf8');
const textReader = readFileSync(new URL('../components/ChatTextReaderDialog.svelte', import.meta.url), 'utf8');

test('compact presentation keeps a single conversation manager, inspector, and focus-managed dialogs', () => {
    const markup = chat.slice(chat.indexOf('</script>'));
    assert.equal((markup.match(/<ChatConversationManager\b/g) ?? []).length, 1);
    assert.equal((markup.match(/<ConversationInspector\b/g) ?? []).length, 1);
    assert.match(roomManager, /dialog\.showModal\(\)/);
    assert.match(roomManager, /oncancel=/);
    assert.match(textReader, /dialog\?\.showModal\(\)/);
    assert.match(textReader, /oncancel=/);
    assert.match(textReader, /target\?\.isConnected/);
});

test('composer keeps focus indication and inspector shares one collapsible tabbed drawer', () => {
    assert.match(chat, /\.composer-box:focus-within\s*\{[^}]*border-color: var\(--composer-focus-border\)/);
    assert.match(inspector, /role="tablist"/);
    assert.match(inspector, /class="inspector-scrim"/);
    assert.match(inspector, /event\.key === 'Escape' && !document\.querySelector\('dialog:modal'\)/);
    for (const tab of ['tasks', 'artifacts', 'recall', 'diff', 'browser', 'context', 'data-sources', 'knowledge']) {
        assert.ok(inspector.includes(`id: '${tab}'`), `inspector tab ${tab}`);
    }
    assert.doesNotMatch(chat, /inspector-page-picker/);
});

test('chat occupies available viewport instead of stacking navigation above the transcript', () => {
    assert.match(chat, /height: 100dvh/);
    assert.match(chat, /@media \(max-width: 760px\)[^]*?\.chat-sidebar\s*\{\s*display: none;/);
    assert.doesNotMatch(chat, /max-height: 42dvh|max-height: 58dvh|min-height: 560px/);
});
