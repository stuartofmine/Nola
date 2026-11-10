import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import { parseMarkdown, markdownHref, markdownPlainText } from './markdown.ts';
import { locale, supportedLocales, t } from './i18n.ts';
import type { Locale } from './i18n.ts';

// Render the actual component with Svelte's server renderer, not a second HTML
// implementation in the test. Absolute imports make the generated data module
// independent of temporary directories and leave no generated source files.
const streamingSource = readFileSync(new URL('../components/StreamingText.svelte', import.meta.url), 'utf8');
const streamingGenerated = compile(streamingSource, { generate: 'server', filename: 'StreamingText.svelte' }).js.code
    .replace("'svelte/internal/server'", JSON.stringify(import.meta.resolve('svelte/internal/server')))
    .replace("'svelte'", JSON.stringify(import.meta.resolve('svelte')))
    .replace("'../lib/streamingText'", JSON.stringify(new URL('./streamingText.ts', import.meta.url).href));
const streamingModule = `data:text/javascript;base64,${Buffer.from(streamingGenerated).toString('base64')}`;
const componentSource = readFileSync(new URL('../components/Markdown.svelte', import.meta.url), 'utf8');
const generated = compile(componentSource, { generate: 'server', filename: 'Markdown.svelte' }).js.code
    .replace("'svelte/internal/server'", JSON.stringify(import.meta.resolve('svelte/internal/server')))
    .replace("'svelte'", JSON.stringify(import.meta.resolve('svelte')))
    .replace("'./StreamingText.svelte'", JSON.stringify(streamingModule))
    .replace("'../lib/streamingText'", JSON.stringify(new URL('./streamingText.ts', import.meta.url).href))
    .replace("'../lib/markdown'", JSON.stringify(new URL('./markdown.ts', import.meta.url).href))
    .replace("'../lib/i18n'", JSON.stringify(new URL('./i18n.ts', import.meta.url).href));
const { default: Markdown } = await import(`data:text/javascript;base64,${Buffer.from(generated).toString('base64')}`);
const html = (source: string, language: Locale = 'zh-Hans') => {
    locale.set(language);
    return render(Markdown, { props: { source } }).body.replace(/<!--.*?-->/gs, '');
};

test('plain previews remove formatting while preserving code, escaped punctuation and link labels', () => {
    assert.equal(markdownPlainText('# **标题**\n\n[来源](https://example.test) 与 `a_b`、\\*原文\\*'),
        '标题\n\n来源 与 a_b、*原文*');
});

test('Markdown controls and accessible labels follow all seven locales without translating content', () => {
    for (const language of supportedLocales) {
        const value = html('```\noriginal_code\n```\n\n![original_alt](https://example.com/image.png)\n\n- [x] original_task', language);
        assert.ok(value.includes(t(language, 'shell.markdown.copy')), language);
        assert.ok(value.includes(`aria-label="${t(language, 'shell.markdown.code')}"`), language);
        assert.ok(value.includes(`aria-label="${t(language, 'shell.markdown.completed')}"`), language);
        assert.ok(value.includes(t(language, 'shell.markdown.image', { label: 'original_alt' })), language);
        assert.ok(value.includes('original_code') && value.includes('original_task'), language);
    }
});

test('Markdown tables and nested lists render as semantic Svelte elements', () => {
    const value = html('# 标题\n\n段落 **粗体** 与 *斜体*。\n\n| 字段 | 数值 |\n| :-- | --: |\n| `温度` | 0 |\n\n3. 第一项\n   - 子项\n\n- [x] 已完成');
    for (const tag of ['<h1', '<strong>', '<em>', '<table', '<thead', '<th ', '<tbody', '<td ', '<ol', '<ul', 'type="checkbox"']) assert.ok(value.includes(tag), tag);
    assert.ok(value.includes('start="3"'));
    assert.ok(value.includes('text-align: right'));
    assert.ok(!value.includes('| :--'));
});

test('entities decode as text but inline and fenced code preserve literal content', () => {
    const blocks = parseMarkdown('A &amp; B `&amp; <tag>`\n\n```html\n<img src=x onerror=evil()>\n```');
    assert.equal(blocks[0].kind, 'paragraph');
    if (blocks[0].kind === 'paragraph') assert.deepEqual(blocks[0].children, [{ kind: 'text', text: 'A & B ' }, { kind: 'code', text: '&amp; <tag>' }]);
    const value = html('A &amp; B `&amp; <tag>`\n\n```html\n<img src=x onerror=evil()>\n```');
    assert.ok(value.includes('A &amp; B'));
    assert.ok(value.includes('&amp;amp; &lt;tag>'));
    assert.ok(!value.includes('<img'));
    assert.ok(value.includes('复制'));
});

test('HTML and dangerous links cannot create active content or remote image requests', () => {
    const value = html('<script>alert(1)</script>\n\n[bad](javascript:alert%281%29) [encoded](javascript&#58;alert%281%29) ![track](https://example.com/pixel)\n\n<iframe src="https://example.com"></iframe>');
    assert.ok(!/<script|<iframe|<img|href="javascript:/i.test(value));
    assert.ok(value.includes('&lt;script>'));
    assert.ok(value.includes('图片：track'));
    assert.ok(value.includes('rel="noopener noreferrer"'));
});

test('link policy rejects credentials, local/app schemes and hidden protocol controls', () => {
    for (const value of ['javascript:alert(1)', 'data:text/html,evil', 'file:///tmp/main.pdf', '/api/private', '//evil.test', 'https://user:secret@example.com', 'java&#10;script:alert(1)', 'https://example.com/\npath']) {
        assert.equal(markdownHref(value), undefined, value);
    }
    assert.equal(markdownHref('https://example.com/?a=1&amp;b=2'), 'https://example.com/?a=1&b=2');
});

test('incomplete fences remain readable during streaming and ordinary soft wraps stay soft', () => {
    assert.deepEqual(parseMarkdown('```rust\nlet x = 1;'), [{ kind: 'code', language: 'rust', text: 'let x = 1;' }]);
    const value = html('同一段第一行\n第二行\n\n新段落  \n硬换行');
    assert.equal((value.match(/<p\b/g) ?? []).length, 2);
    assert.equal((value.match(/<br\b/g) ?? []).length, 1);
});


test('streaming text reveals new content without replaying a restored snapshot', () => {
    const source = 'Already **visible**\n\n```python\nprint(1)\n```';
    const restored = render(Markdown, { props: { source, streaming: true, initialText: source } }).body;
    const fresh = render(Markdown, { props: { source, streaming: true } }).body;
    assert.doesNotMatch(restored, /class="[^"]*\bfresh\b/);
    assert.match(fresh, /class="[^"]*\bfresh\b/);
    assert.ok(restored.includes('print(1)'));
});

test('long code and table history remains ordinary text without reveal pieces', () => {
    const source = '# Long history\n\n' + 'Paragraph with **stable text**.\n\n'.repeat(100)
        + '```python\n' + 'print("long code sample")\n'.repeat(1000) + '```\n\n'
        + '| Item | Value |\n|---|---|\n'
        + Array.from({ length: 1000 }, (_, index) => `| row-${index} | value-${index} |\n`).join('');
    const ordinary = html(source);
    const restored = render(Markdown, { props: { source, streaming: true, initialText: source } }).body;
    for (const value of [ordinary, restored]) {
        assert.doesNotMatch(value, /class="[^"]*\bfresh\b/);
        assert.ok(value.includes('row-999') && value.includes('value-999'));
        assert.equal((value.match(/<td\b/g) ?? []).length, 2000);
        assert.equal((value.match(/print\(/g) ?? []).length, 1000);
    }
});
