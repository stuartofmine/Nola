import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// Source guardrails only. Final browser acceptance must separately exercise
// focus trapping/restoration, keyboard opening, resize, IME and large text.
const chat = readFileSync(new URL('../pages/Chat.svelte', import.meta.url), 'utf8');
const panel = readFileSync(new URL('../components/ChatResponsivePanel.svelte', import.meta.url), 'utf8');

test('compact presentation keeps a single conversation manager and settings owner', () => {
    assert.equal((chat.match(/<ChatConversationManager\b/g) ?? []).length, 1);
    assert.equal((chat.match(/<ChatSessionSettings\s+bind:this=/g) ?? []).length, 1);
    assert.match(chat, /bind:this=\{navigationPanel\} breakpoint=\{700\}/);
    assert.match(chat, /bind:this=\{inspectorPanel\} breakpoint=\{1250\}/);
    assert.equal((panel.match(/\{@render children\(\)\}/g) ?? []).length, 1);
    assert.match(panel, /dialog\.showModal\(\)/);
    assert.match(panel, /oncancel=/);
    assert.match(panel, /opener\?\.isConnected/);
});

test('composer keeps focus indication and grouped work tools share one collapsible inspector', () => {
    assert.match(chat, /\.composer:focus-within\s*\{[^}]*outline: 2px solid var\(--primary\)/);
    for (const tabs of ["['overview', 'progress', 'goal']", "['review', 'artifacts']", "['resources', 'execution', 'participants', 'settings']"]) assert.ok(chat.includes(tabs));
    assert.ok(chat.includes('aria-controls="chat-details"'));
    assert.ok(chat.includes('aria-expanded={selected}'));
    assert.ok(chat.includes('inspectorTab = inspectorTab === tab ? null : tab'));
    for (const tab of ['overview', 'review', 'artifacts', 'resources', 'settings', 'participants']) {
        assert.ok(chat.includes(`data-inspector-section="${tab}"`));
    }
    assert.doesNotMatch(chat, /inspector-page-picker|hidden=\{inspectorTab !==/);
});

test('chat occupies available viewport instead of stacking navigation above the transcript', () => {
    assert.match(chat, /grid-template-rows: auto minmax\(0, 1fr\)/);
    assert.match(chat, /height: 100dvh/);
    assert.doesNotMatch(chat, /max-height: 42dvh|max-height: 58dvh|min-height: 560px/);
});
