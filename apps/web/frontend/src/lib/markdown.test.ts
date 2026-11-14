import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import {
    StreamingMarkdownCache,
    findMarkdownSearchMatches,
    indexMarkdownBlocks,
    markdownHref,
    markdownPlainText,
    nodeSearchMatches,
    parseMarkdown,
} from './markdown.ts';
import type { InlineNode, MarkdownBlock } from './markdown.ts';
import type { Locale } from './i18n.ts';

// Load the production component tree, including bundled diagram assets.
const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
after(async () => { await server.close(); });
const {default: Markdown} = await server.ssrLoadModule('/src/components/Markdown.svelte');
const {render} = await server.ssrLoadModule('svelte/server');
const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
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
    for (const tag of ['h1', 'strong', 'em', 'table', 'thead', 'th', 'tbody', 'td', 'ol', 'ul']) {
        assert.match(value, new RegExp(`<${tag}(?:\\s|>)`));
    }
    assert.ok(value.includes('type="checkbox"'));
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


test('completed Mermaid uses the actual sandbox component while a live fence remains source', () => {
    const source = '```mermaid\nflowchart BT\nA[Start] --> B[End]\n```';
    for (const language of supportedLocales) {
        const result = html(source, language);
        assert.match(result, /<iframe/);
        assert.match(result, /sandbox="allow-scripts"/);
        assert.ok(!result.includes('allow-same-origin'));
        assert.ok(result.includes(t(language, 'a2ui.diagramSource')));
        assert.ok(result.includes('flowchart BT'));
        const live = render(Markdown, {props: {source, streaming: true}}).body;
        assert.doesNotMatch(live, /<iframe/);
        assert.match(live.replace(/<[^>]*>/g, ''), /flowchart BT/);
    }
});

test('inline references inside paragraphs and table cells retain clickable targets', () => {
    const value = html('正文 [来源](https://example.com/paragraph)\n\n| 时间 | 说明 |\n| --- | --- |\n| 阶段一 | 内容 [Source](https://example.com/table) |');
    assert.equal((value.match(/class="[^"]*\binline-reference\b[^"]*"/g) || []).length, 2);
    assert.ok(value.includes('href="https://example.com/paragraph"'));
    assert.match(value, /<td[^>]*>.*?href="https:\/\/example.com\/table"/s);
});

function collectNodeOffsets(blocks: readonly MarkdownBlock[], offsetOf: (node: object) => number): number[] {
    const offsets: number[] = [];
    function visitInlines(nodes: readonly InlineNode[]) {
        for (const node of nodes) {
            offsets.push(offsetOf(node));
            if ('children' in node) visitInlines(node.children);
        }
    }
    function visitBlocks(nodes: readonly MarkdownBlock[]) {
        for (const node of nodes) {
            offsets.push(offsetOf(node));
            if (node.kind === 'paragraph' || node.kind === 'heading') visitInlines(node.children);
            else if (node.kind === 'quote') visitBlocks(node.children);
            else if (node.kind === 'list') for (const item of node.items) visitBlocks(item.children);
            else if (node.kind === 'table') {
                for (const cell of node.header) visitInlines(cell);
                for (const row of node.rows) for (const cell of row) visitInlines(cell);
            }
        }
    }
    visitBlocks(blocks);
    return offsets;
}

const STREAMING_CORPUS = [
    '# 流式 Markdown 增量解析验证',
    '',
    '第一段包含 **加粗文本**、*斜体*、~~删除线~~、`inline_code` 与 [外部引用](https://example.com/one) 以及硬换行  \n继续同一段落。',
    '',
    'Setext 主标题跨行\n第二行标题文本\n===',
    '',
    'Setext 副标题\n---',
    '',
    '- 紧凑列表项 A\n- [x] 任务已完成\n- [ ] 任务待处理\n  - 嵌套子项 `token_1`',
    '',
    '1. 松散有序列表第一项\n\n   第一项的续段，提及 Nola 架构。\n\n2. 松散有序列表第二项',
    '',
    '> 引用第一段：包含 **Nola** 关键词与 [预设链接][preset-ref]。\n>\n> > 嵌套引用第二段：`quote_code`。',
    '',
    '[preset-ref]: https://example.com/preset "Preset Title"',
    '',
    '| 模块 | 复杂度 | 说明 |',
    '| :--- | :---: | ---: |',
    '| `markdown.ts` | O(tail) | 冻结前缀块缓存 |',
    '| 搜索高亮 | O(tail) | 偏移量与 [文档][late-ref] 一致 |',
    '',
    '````markdown',
    '# 围栏内部标题不应触发外部切分',
    '',
    '```ts',
    'const nola = "nested backticks";',
    '',
    'export function demo() { return nola; }',
    '```',
    '',
    '围栏内部尾段',
    '````',
    '',
    '~~~python',
    'def compute_nola():',
    '    return "tilde fence"',
    '',
    'print(compute_nola())',
    '~~~',
    '',
    '    indented_line_1()',
    '',
    '',
    '    indented_line_2()',
    '',
    '尾声段落：再次检索 Nola 与中文关键词「增量解析」，验证 [延迟定义链接][late-ref] 与转义 \\*星号\\*。',
    '',
    '[late-ref]: https://example.com/late "Late Title"',
    '',
    '倒数第二段：推进延迟引用定义越过可变尾部窗口。',
    '',
    '最终确认段落：Nola 流式渲染完成。',
].join('\n');

test('character-by-character streaming cache matches parseMarkdown and search offsets at every code unit', () => {
    const cache = new StreamingMarkdownCache();
    const queries = ['nola', '增量解析', 'O(tail)'];
    for (let cursor = 0; cursor <= STREAMING_CORPUS.length; cursor++) {
        const partial = STREAMING_CORPUS.slice(0, cursor);
        const snapshot = cache.update(partial);
        const expectedBlocks = parseMarkdown(partial);
        assert.deepEqual(snapshot.blocks, expectedBlocks, `AST mismatch at cursor=${cursor}`);

        const expectedIndex = indexMarkdownBlocks(expectedBlocks);
        assert.equal(snapshot.text, expectedIndex.text, `Indexed text mismatch at cursor=${cursor}`);
        assert.deepEqual(
            collectNodeOffsets(snapshot.blocks, snapshot.offsetOf),
            collectNodeOffsets(expectedBlocks, node => expectedIndex.offsets.get(node) ?? 0),
            `Node offsets mismatch at cursor=${cursor}`,
        );

        for (const query of queries) {
            assert.deepEqual(
                cache.search(query),
                findMarkdownSearchMatches(expectedBlocks, query),
                `Search match mismatch for "${query}" at cursor=${cursor}`,
            );
        }
    }
    assert.ok(cache.cacheHits > 500, `Expected high prefix cache reuse, got ${cache.cacheHits}`);
    assert.ok(cache.stablePrefixSource.length > 0);
});

test('chunk-by-chunk streaming handles unclosed fences, CRLF splits, and tail replacements', () => {
    for (const chunkSize of [2, 5, 11, 23, 47]) {
        const cache = new StreamingMarkdownCache();
        for (let end = chunkSize; end < STREAMING_CORPUS.length + chunkSize; end += chunkSize) {
            const partial = STREAMING_CORPUS.slice(0, Math.min(STREAMING_CORPUS.length, end));
            const snapshot = cache.update(partial);
            assert.deepEqual(snapshot.blocks, parseMarkdown(partial), `Chunk size ${chunkSize} mismatch at end=${end}`);
            assert.deepEqual(cache.search('Nola'), findMarkdownSearchMatches(partial, 'Nola'));
        }
    }

    // Verify CRLF boundary split where '\r' arrives at the end of one chunk and '\n' starts the next.
    const crlfDoc = '# 标题\r\n\r\n第一段 **Nola**\r\n\r\n第二段 `code`\r\n\r\n```rust\r\nfn main() {}\r\n```\r\n\r\n第三段结尾';
    const crlfCache = new StreamingMarkdownCache();
    for (let index = 1; index <= crlfDoc.length; index++) {
        const partial = crlfDoc.slice(0, index);
        const snapshot = crlfCache.update(partial);
        assert.deepEqual(snapshot.blocks, parseMarkdown(partial), `CRLF mismatch at index=${index}`);
    }

    // Verify unclosed fenced code block mid-stream followed by closure and reconciliation replacement.
    const fenceCache = new StreamingMarkdownCache();
    const unclosed = '# Header\n\nIntro paragraph.\n\nParagraph two.\n\n```ts\nconst x = 1;\n\nconst y = 2;';
    assert.deepEqual(fenceCache.update(unclosed).blocks, parseMarkdown(unclosed));
    const closed = `${unclosed}\n\`\`\`\n\nOutro paragraph.`;
    assert.deepEqual(fenceCache.update(closed).blocks, parseMarkdown(closed));
    const tailReplaced = '# Header\n\nIntro paragraph.\n\nParagraph two.\n\nReplaced trailing paragraph.';
    assert.deepEqual(fenceCache.update(tailReplaced).blocks, parseMarkdown(tailReplaced));
    const fullReplaced = 'Completely different message';
    assert.deepEqual(fenceCache.update(fullReplaced).blocks, parseMarkdown(fullReplaced));
});

test('search match offsets span inline nodes without falsely crossing block or table-cell boundaries', () => {
    const cache = new StreamingMarkdownCache();
    const doc = [
        '# Hello **World** Heading',
        '',
        'Paragraph one with `alpha_beta` and **Beta Gamma** inline.',
        '',
        '| Name | Role |',
        '| --- | --- |',
        '| Alice | Engineer |',
        '',
        'Trailing tail mentioning Beta Gamma in mutable suffix.',
    ].join('\n');

    const snapshot = cache.update(doc);
    const matches = cache.search('beta gamma');
    assert.deepEqual(matches, findMarkdownSearchMatches(doc, 'beta gamma'));
    assert.equal(matches.length, 2);
    for (const match of matches) {
        assert.equal(snapshot.text.slice(match.start, match.end).toLowerCase(), 'beta gamma');
        const metric = snapshot.blockMetrics[match.blockIndex];
        assert.equal(metric.text.slice(match.blockStart, match.blockEnd).toLowerCase(), 'beta gamma');
        assert.equal(match.start, metric.offset + match.blockStart);
    }

    // Adjacent table cells ("Name" + "Role", "Alice" + "Engineer") and adjacent blocks must not glue together.
    assert.deepEqual(cache.search('NameRole'), []);
    assert.deepEqual(cache.search('AliceEngineer'), []);
    assert.deepEqual(cache.search('HeadingParagraph'), []);

    // Verify per-node local highlight slicing across formatted inline nodes (`Hello ` + `World`).
    const crossInline = cache.search('hello world');
    assert.equal(crossInline.length, 1);
    const heading = snapshot.blocks[0];
    assert.equal(heading.kind, 'heading');
    if (heading.kind === 'heading') {
        const firstText = heading.children[0];
        const strongNode = heading.children[1];
        assert.equal(firstText.kind, 'text');
        assert.equal(strongNode.kind, 'strong');
        if (firstText.kind === 'text' && strongNode.kind === 'strong') {
            const strongChild = strongNode.children[0];
            assert.equal(strongChild.kind, 'text');
            if (strongChild.kind === 'text') {
                assert.deepEqual(
                    nodeSearchMatches(crossInline, snapshot.offsetOf(firstText), firstText.text.length),
                    [{ start: 0, end: 6 }],
                );
                assert.deepEqual(
                    nodeSearchMatches(crossInline, snapshot.offsetOf(strongChild), strongChild.text.length),
                    [{ start: 0, end: 5 }],
                );
            }
        }
    }
});

test('streaming a 200-paragraph document reuses frozen prefix blocks in O(tail) without re-parsing prefix', () => {
    const paragraphs = Array.from(
        { length: 200 },
        (_, index) => `## Section ${index}\n\nParagraph ${index} discusses **streaming marker ${index}** and \`item_${index}\` in detail.`,
    );
    const fullDocument = paragraphs.join('\n\n');
    const cache = new StreamingMarkdownCache();

    let appended = '';
    let previousFrozenRefs: MarkdownBlock[] = [];
    for (let index = 0; index < paragraphs.length; index++) {
        appended += (index === 0 ? '' : '\n\n') + paragraphs[index];
        const snapshot = cache.update(appended);

        // Every previously frozen block object reference must stay strictly identical (`===`).
        for (let frozenIndex = 0; frozenIndex < previousFrozenRefs.length; frozenIndex++) {
            assert.equal(
                snapshot.blocks[frozenIndex],
                previousFrozenRefs[frozenIndex],
                `Frozen block ${frozenIndex} was re-allocated at step ${index}`,
            );
        }
        previousFrozenRefs = snapshot.blocks.slice(0, snapshot.stableBlockCount);
    }

    const finalSnapshot = cache.update(fullDocument);
    assert.equal(cache.lastParsedLength, 0, 'Unchanged completion snapshot must parse 0 characters');
    assert.deepEqual(finalSnapshot.blocks, parseMarkdown(fullDocument));
    assert.deepEqual(cache.search('streaming marker 0'), findMarkdownSearchMatches(fullDocument, 'streaming marker 0'));
    assert.deepEqual(cache.search('streaming marker 199'), findMarkdownSearchMatches(fullDocument, 'streaming marker 199'));

    assert.equal(cache.prefixResets, 0);
    assert.ok(cache.cacheHits >= 195, `Expected >= 195 prefix cache hits across 200 sections, got ${cache.cacheHits}`);
    assert.ok(
        finalSnapshot.stableBlockCount >= 390,
        `Expected >= 390 of 400 heading/paragraph blocks frozen, got ${finalSnapshot.stableBlockCount}`,
    );
    assert.ok(
        finalSnapshot.mutableTailSource.length < 400,
        `Expected O(tail) mutable suffix (< 400 chars), got ${finalSnapshot.mutableTailSource.length}`,
    );
    // Naive O(N^2) parsing across 200 steps would parse ~200 * fullDocument.length / 2 = ~100x fullDocument.length.
    // O(tail) incremental parsing must stay within a small constant multiple of fullDocument.length.
    assert.ok(
        cache.totalParsedChars < fullDocument.length * 4,
        `Expected linear total parsed chars (< ${fullDocument.length * 4}), got ${cache.totalParsedChars}`,
    );
});
