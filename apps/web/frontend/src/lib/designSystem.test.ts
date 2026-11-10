import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');
const theme = source('../theme.css');
function luminance(hex: string) {
    const rgb = hex.match(/[0-9a-f]{2}/gi)!.map(value => {
        const channel = parseInt(value, 16) / 255;
        return channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
    });
    return rgb[0] * .2126 + rgb[1] * .7152 + rgb[2] * .0722;
}
test('neutral palette maintains text and actionable outline contrast in both appearances', () => {
    const palettes = [theme.split('@media')[0], theme.split('@media (prefers-color-scheme: dark)')[1].split('@media')[0]];
    for (const block of palettes) {
        const palette = Object.fromEntries([...block.matchAll(/--([\w-]+):\s*#([\da-f]{6})\b/gi)]
            .map(match => [match[1], match[2]]));
        for (const [foreground, background, minimum] of [
            ['content', 'canvas', 4.5], ['muted', 'panel', 4.5],
            ['on-primary', 'primary', 4.5], ['outline', 'surface', 3],
            ['accent', 'accent-subtle', 3],
        ] as const) {
            assert.ok(palette[foreground] && palette[background], `${foreground}/${background} must exist`);
            const a = luminance(palette[foreground]), b = luminance(palette[background]);
            assert.ok((Math.max(a, b) + .05) / (Math.min(a, b) + .05) >= minimum,
                `${foreground}/${background} contrast`);
        }
    }
});
test('shared controls preserve zoom, readable disabled states and optional motion', () => {
    assert.match(theme, /font-size: 100%/);
    assert.match(theme, /button:disabled \{ opacity: 1;/);
    assert.match(theme, /prefers-reduced-motion: reduce/);
    assert.match(theme, /forced-colors: active/);
    assert.match(theme, /\.tabs \{[^}]*flex-wrap: wrap/);
    assert.match(theme, /textarea[^}]*max-width: 100%/);
    assert.doesNotMatch(source('../pages/Settings.svelte'), /var\(--accent-text|var\(--text\)/);
    assert.doesNotMatch(source('../components/OpenCodeSourceLogin.svelte'), /var\(--border-color/);
});
test('chat and previews keep local reading widths without changing resource ownership', () => {
    const chat = source('../pages/Chat.svelte');
    assert.match(chat, /article\.user \{[^}]*max-width: 85%/);
    assert.match(chat, /\.composer \{[^}]*max-width: 720px/);
    assert.match(source('../components/Markdown.svelte'), /overflow-x: auto/);
    const pdf = source('../components/ChatPdfResources.svelte');
    assert.match(pdf, /height: min\(60dvh, 480px\)/);
    assert.match(pdf, /loadConversationPdf\(resource, next.signal\)/);
    assert.match(source('./chatPdfPreview.ts'), /conversationPdfUrl\(resource.conversationId, resource.resourceId\)/);
});
test('empty chat has one real creation entry and no inactive composer or inspector', () => {
    const chat = source('../pages/Chat.svelte');
    assert.doesNotMatch(chat, /onclick=\{createFirstConversation\}/);
    assert.match(source('../components/ChatConversationManager.svelte'), /class="new-conversation"[^\n]*mutate\('create'\)/);
    assert.doesNotMatch(chat, /conversationManager\.createConversation\(\)/);
    assert.match(chat, /conversationManager\?\.focusCreateEntry\(\)/);
    assert.match(chat, /\{#if active && canInteract\}\s*(?:\{#if selectedExperts[^]*?\{\/if\}\s*)?<form class="composer"/);
    assert.match(chat, /\{#if active\}<ChatResponsivePanel bind:this=\{inspectorPanel\}/);
    assert.match(chat, /!active && !initializationFailed/);
    assert.match(chat, /chatPage\.retryInitialization/);
});
test('phone navigation has its own list and desktop retains one focus-managed workspace', () => {
    const app = source('../App.svelte');
    const chat = source('../pages/Chat.svelte');
    const responsivePanel = source('../components/ChatResponsivePanel.svelte');
    assert.doesNotMatch(app, /<ChatResponsivePanel/);
    assert.match(app, /path === '\/conversations'/);
    assert.match(app, /initialWorkspaceLocation/);
    assert.match(chat, /ChatResponsivePanel bind:this=\{navigationPanel\} breakpoint=\{700\}/);
    assert.match(chat, /@media \(max-width: 700px\)/);
    assert.match(responsivePanel, /opener\?\.isConnected[^\n]*opener\.focus\(\)/);
});
