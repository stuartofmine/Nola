import { Lexer, marked } from 'marked';
import type { Links, Token, Tokens } from 'marked';
import { decodeHTML } from 'entities';

export type InlineNode =
    | { kind: 'text' | 'code'; text: string }
    | { kind: 'strong' | 'em' | 'del'; children: InlineNode[] }
    | { kind: 'link'; children: InlineNode[]; href?: string; title?: string }
    | { kind: 'image'; text: string; href?: string }
    | { kind: 'br' };
export type MarkdownBlock =
    | { kind: 'paragraph'; children: InlineNode[] }
    | { kind: 'heading'; depth: number; children: InlineNode[] }
    | { kind: 'code'; text: string; language: string }
    | { kind: 'quote'; children: MarkdownBlock[] }
    | { kind: 'list'; ordered: boolean; start: number; items: { checked?: boolean; children: MarkdownBlock[] }[] }
    | { kind: 'table'; header: InlineNode[][]; rows: InlineNode[][][]; align: ('left' | 'center' | 'right' | null)[] }
    | { kind: 'hr' };

export type SearchMatch = {
    start: number;
    end: number;
    blockIndex: number;
    blockStart: number;
    blockEnd: number;
};

type TextSegment = {
    blockOffset: number;
    text: string;
};

export type MarkdownBlockMetric = {
    block: MarkdownBlock;
    text: string;
    offset: number;
    length: number;
    segments: readonly TextSegment[];
};

export type StreamingMarkdownSnapshot = {
    blocks: MarkdownBlock[];
    stablePrefixSource: string;
    mutableTailSource: string;
    stableBlockCount: number;
    text: string;
    blockMetrics: readonly MarkdownBlockMetric[];
    offsetOf(node: object): number;
};

const MARKED_OPTIONS = { gfm: true, breaks: false } as const;

// No model-generated URL can invoke a browser/app protocol or expose URL credentials.
// Local document links need a separate, authorized host-file contract.
export function markdownHref(value: string): string | undefined {
    const text = decodeHTML(value).trim();
    if (/[\u0000-\u0020\u007f]/.test(text)) return;
    try {
        const url = new URL(text);
        if (!['http:', 'https:', 'mailto:'].includes(url.protocol) || url.username || url.password) return;
        return url.href;
    } catch { return; }
}

function inline(tokens: Token[], depth = 0): InlineNode[] {
    return tokens.flatMap((token): InlineNode[] => {
        if (depth > 48) return [{ kind: 'text', text: token.raw }];
        switch (token.type) {
            case 'strong': case 'em': case 'del':
                return [{ kind: token.type, children: inline((token as Tokens.Strong).tokens, depth + 1) }];
            case 'codespan': return [{ kind: 'code', text: (token as Tokens.Codespan).text }];
            case 'br': return [{ kind: 'br' }];
            case 'link': {
                const link = token as Tokens.Link;
                return [{ kind: 'link', children: inline(link.tokens, depth + 1), href: markdownHref(link.href), title: link.title ?? undefined }];
            }
            case 'image': {
                const image = token as Tokens.Image;
                return [{ kind: 'image', text: decodeHTML(image.text), href: markdownHref(image.href) }];
            }
            case 'text': {
                const text = token as Tokens.Text;
                return text.tokens ? inline(text.tokens, depth + 1) : [{ kind: 'text', text: decodeHTML(text.text) }];
            }
            // HTML is deliberately text, never an innerHTML/{@html} sink.
            case 'escape': return [{ kind: 'text', text: (token as Tokens.Escape).text }];
            default: return [{ kind: 'text', text: token.raw }];
        }
    });
}

function blocks(tokens: Token[], depth = 0): MarkdownBlock[] {
    return tokens.flatMap((token): MarkdownBlock[] => {
        if (depth > 48) return [{ kind: 'paragraph', children: [{ kind: 'text', text: token.raw }] }];
        switch (token.type) {
            case 'space': case 'def': return [];
            case 'hr': return [{ kind: 'hr' }];
            case 'paragraph': case 'text': return [{ kind: 'paragraph', children: inline((token as Tokens.Paragraph).tokens ?? [token]) }];
            case 'heading': {
                const heading = token as Tokens.Heading;
                return [{ kind: 'heading', depth: heading.depth, children: inline(heading.tokens) }];
            }
            case 'code': {
                const code = token as Tokens.Code;
                return [{ kind: 'code', text: code.text, language: code.lang?.split(/\s/)[0] ?? '' }];
            }
            case 'blockquote': return [{ kind: 'quote', children: blocks((token as Tokens.Blockquote).tokens, depth + 1) }];
            case 'list': {
                const list = token as Tokens.List;
                return [{ kind: 'list', ordered: list.ordered, start: typeof list.start === 'number' ? list.start : 1,
                    items: list.items.map(item => ({ checked: item.task ? !!item.checked : undefined, children: blocks(item.tokens, depth + 1) })) }];
            }
            case 'table': {
                const table = token as Tokens.Table;
                return [{ kind: 'table', header: table.header.map(cell => inline(cell.tokens)),
                    rows: table.rows.map(row => row.map(cell => inline(cell.tokens))), align: table.align }];
            }
            default: return [{ kind: 'paragraph', children: [{ kind: 'text', text: token.raw }] }];
        }
    });
}

export function parseMarkdown(source: string): MarkdownBlock[] {
    try { return blocks(marked.lexer(source, MARKED_OPTIONS)); }
    catch { return [{ kind: 'paragraph', children: [{ kind: 'text', text: source }] }]; }
}

function indexSingleBlock(
    block: MarkdownBlock,
    baseOffset: number,
    offsets?: Map<object, number> | WeakMap<object, number>,
): MarkdownBlockMetric {
    let text = '';
    const segments: TextSegment[] = [];
    function visitInlines(nodes: InlineNode[]) {
        for (const node of nodes) {
            offsets?.set(node, baseOffset + text.length);
            if (node.kind === 'text' || node.kind === 'code') text += node.text;
            else if ('children' in node) visitInlines(node.children);
        }
    }
    function recordInlineSegment(nodes: InlineNode[]) {
        const start = text.length;
        visitInlines(nodes);
        if (text.length > start) segments.push({ blockOffset: start, text: text.slice(start) });
    }
    function visitBlock(node: MarkdownBlock) {
        offsets?.set(node, baseOffset + text.length);
        if (node.kind === 'code') {
            const start = text.length;
            text += node.text;
            if (node.text.length > 0) segments.push({ blockOffset: start, text: node.text });
        } else if (node.kind === 'paragraph' || node.kind === 'heading') {
            recordInlineSegment(node.children);
        } else if (node.kind === 'quote') {
            for (const child of node.children) visitBlock(child);
        } else if (node.kind === 'list') {
            for (const item of node.items) for (const child of item.children) visitBlock(child);
        } else if (node.kind === 'table') {
            for (const cell of node.header) recordInlineSegment(cell);
            for (const row of node.rows) for (const cell of row) recordInlineSegment(cell);
        }
    }
    visitBlock(block);
    return { block, text, offset: baseOffset, length: text.length, segments };
}

export function indexMarkdownBlocks(
    parsedBlocks: readonly MarkdownBlock[],
    baseOffset = 0,
): { text: string; offsets: Map<object, number>; blockMetrics: MarkdownBlockMetric[] } {
    const offsets = new Map<object, number>();
    const blockMetrics: MarkdownBlockMetric[] = [];
    let text = '';
    let cursor = baseOffset;
    for (const block of parsedBlocks) {
        const metric = indexSingleBlock(block, cursor, offsets);
        blockMetrics.push(metric);
        text += metric.text;
        cursor += metric.length;
    }
    return { text, offsets, blockMetrics };
}

function compileSearchPattern(query: string): RegExp | null {
    if (!query) return null;
    const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    return new RegExp(escaped, 'gi');
}

function collectBlockSearchMatches(
    metrics: readonly MarkdownBlockMetric[],
    startBlockIndex: number,
    pattern: RegExp,
    queryLength: number,
    out: SearchMatch[],
): void {
    for (let index = 0; index < metrics.length; index++) {
        const metric = metrics[index];
        if (metric.length < queryLength) continue;
        const blockIndex = startBlockIndex + index;
        for (const segment of metric.segments) {
            if (segment.text.length < queryLength) continue;
            pattern.lastIndex = 0;
            let match: RegExpExecArray | null;
            while ((match = pattern.exec(segment.text)) !== null) {
                const blockStart = segment.blockOffset + match.index;
                const blockEnd = blockStart + queryLength;
                out.push({
                    start: metric.offset + blockStart,
                    end: metric.offset + blockEnd,
                    blockIndex,
                    blockStart,
                    blockEnd,
                });
            }
        }
    }
}

export function findMarkdownSearchMatches(
    sourceOrBlocks: string | readonly MarkdownBlock[],
    query: string,
): SearchMatch[] {
    const pattern = compileSearchPattern(query);
    if (!pattern) return [];
    const parsed = typeof sourceOrBlocks === 'string' ? parseMarkdown(sourceOrBlocks) : sourceOrBlocks;
    const { blockMetrics } = indexMarkdownBlocks(parsed);
    const matches: SearchMatch[] = [];
    collectBlockSearchMatches(blockMetrics, 0, pattern, query.length, matches);
    return matches;
}

export function nodeSearchMatches(
    matches: readonly SearchMatch[],
    nodeOffset: number,
    nodeLength: number,
): { start: number; end: number }[] {
    if (!matches.length || nodeLength <= 0) return [];
    const nodeEnd = nodeOffset + nodeLength;
    let low = 0;
    let high = matches.length;
    while (low < high) {
        const mid = (low + high) >>> 1;
        if (matches[mid].end <= nodeOffset) low = mid + 1;
        else high = mid;
    }
    const local: { start: number; end: number }[] = [];
    for (let index = low; index < matches.length; index++) {
        const match = matches[index];
        if (match.start >= nodeEnd) break;
        local.push({
            start: Math.max(0, match.start - nodeOffset),
            end: Math.min(nodeLength, match.end - nodeOffset),
        });
    }
    return local;
}

function hasDefToken(tokens: readonly Token[]): boolean {
    for (const token of tokens) {
        if (token.type === 'def') return true;
        if (token.type === 'blockquote' && hasDefToken((token as Tokens.Blockquote).tokens)) return true;
        if (token.type === 'list') {
            for (const item of (token as Tokens.List).items) {
                if (hasDefToken(item.tokens)) return true;
            }
        }
    }
    return false;
}

/**
 * Finds the last blank-line `space` token before at least two trailing blocks
 * (with at least one blank-line boundary or a third block separating them),
 * ensuring lists, indented code, tables, setext headings, and open fences
 * in the active tail can never retroactively mutate frozen prefix blocks.
 */
function findSafeFreezeSpaceIndex(tokens: readonly Token[]): number {
    let nonSpaceAfter = 0;
    let spaceAfter = 0;
    let candidate = -1;
    for (let index = tokens.length - 1; index >= 0; index--) {
        if (tokens[index].type === 'space') {
            if (nonSpaceAfter >= 2 && (spaceAfter >= 1 || nonSpaceAfter >= 3)) {
                candidate = index;
                break;
            }
            spaceAfter++;
        } else {
            nonSpaceAfter++;
        }
    }
    if (candidate <= 0) return -1;
    for (let index = 0; index < candidate; index++) {
        if (tokens[index].type !== 'space') return candidate;
    }
    return -1;
}

/**
 * Incremental $O(\text{tail})$ Markdown block & text-offset prefix cache for SSE streaming.
 * Guarantees 100% AST and search-offset equivalence with `parseMarkdown(source)`.
 */
export class StreamingMarkdownCache {
    private rawSource = '';
    private rawEndedWithCR = false;
    private normalizedSource = '';
    private stablePrefix = '';
    private stableBlocks: MarkdownBlock[] = [];
    private stableBlockMetrics: MarkdownBlockMetric[] = [];
    private stableText = '';
    private stableTextLength = 0;
    private stableLinks: Links = Object.create(null) as Links;
    private stableLinkCount = 0;
    private stableMayHaveBracket = false;
    private nodeOffsets = new WeakMap<object, number>();
    private tailBlockMetrics: MarkdownBlockMetric[] = [];
    private lastSnapshot: StreamingMarkdownSnapshot | null = null;

    private cachedSearchQuery = '';
    private cachedSearchPattern: RegExp | null = null;
    private cachedSearchStableCount = 0;
    private cachedStableMatches: SearchMatch[] = [];

    public cacheHits = 0;
    public prefixResets = 0;
    public lastParsedLength = 0;
    public totalParsedChars = 0;

    get stablePrefixSource(): string {
        return this.stablePrefix;
    }

    get frozenBlocks(): readonly MarkdownBlock[] {
        return this.stableBlocks;
    }

    reset(): void {
        this.rawSource = '';
        this.rawEndedWithCR = false;
        this.normalizedSource = '';
        this.lastSnapshot = null;
        this.resetStableState();
    }

    private resetStableState(): void {
        this.stablePrefix = '';
        this.stableBlocks = [];
        this.stableBlockMetrics = [];
        this.stableText = '';
        this.stableTextLength = 0;
        this.stableLinks = Object.create(null) as Links;
        this.stableLinkCount = 0;
        this.stableMayHaveBracket = false;
        this.nodeOffsets = new WeakMap<object, number>();
        this.tailBlockMetrics = [];
        this.cachedSearchStableCount = 0;
        this.cachedStableMatches = [];
    }

    private normalizeInput(source: string): string {
        if (this.rawSource.length > 0 && source.startsWith(this.rawSource)) {
            let deltaStart = this.rawSource.length;
            if (this.rawEndedWithCR && source.charCodeAt(deltaStart) === 10) {
                deltaStart += 1;
            }
            const delta = source.slice(deltaStart);
            const normalizedDelta = delta.includes('\r') ? delta.replace(/\r\n|\r/g, '\n') : delta;
            this.normalizedSource += normalizedDelta;
        } else {
            this.normalizedSource = source.includes('\r') ? source.replace(/\r\n|\r/g, '\n') : source;
        }
        this.rawSource = source;
        this.rawEndedWithCR = source.length > 0 && source.charCodeAt(source.length - 1) === 13;
        return this.normalizedSource;
    }

    update(source: string): StreamingMarkdownSnapshot {
        if (source === this.rawSource && this.lastSnapshot) {
            this.lastParsedLength = 0;
            return this.lastSnapshot;
        }

        const normalized = this.normalizeInput(source);
        if (!normalized) {
            if (this.stablePrefix.length > 0) this.prefixResets++;
            this.resetStableState();
            this.lastParsedLength = 0;
            const offsets = this.nodeOffsets;
            this.lastSnapshot = {
                blocks: [],
                stablePrefixSource: '',
                mutableTailSource: '',
                stableBlockCount: 0,
                text: '',
                blockMetrics: [],
                offsetOf: (node: object) => offsets.get(node) ?? 0,
            };
            return this.lastSnapshot;
        }

        const reusedPrefix = this.stablePrefix.length > 0 && normalized.startsWith(this.stablePrefix);
        if (!reusedPrefix && this.stablePrefix.length > 0) {
            this.prefixResets++;
            this.resetStableState();
        } else if (reusedPrefix) {
            this.cacheHits++;
        }

        try {
            return this.lexAndCommit(normalized, reusedPrefix);
        } catch {
            this.resetStableState();
            const fallbackBlocks: MarkdownBlock[] = [{ kind: 'paragraph', children: [{ kind: 'text', text: source }] }];
            const metric = indexSingleBlock(fallbackBlocks[0], 0, this.nodeOffsets);
            this.tailBlockMetrics = [metric];
            const offsets = this.nodeOffsets;
            this.lastSnapshot = {
                blocks: fallbackBlocks,
                stablePrefixSource: '',
                mutableTailSource: normalized,
                stableBlockCount: 0,
                text: metric.text,
                blockMetrics: [metric],
                offsetOf: (node: object) => offsets.get(node) ?? 0,
            };
            return this.lastSnapshot;
        }
    }

    private lexAndCommit(normalized: string, reusedPrefix: boolean): StreamingMarkdownSnapshot {
        let mutableInput = normalized.slice(this.stablePrefix.length);
        this.lastParsedLength = mutableInput.length;
        this.totalParsedChars += mutableInput.length;

        let lexer = new Lexer(MARKED_OPTIONS);
        if (this.stableLinkCount > 0) {
            Object.assign(lexer.tokens.links, this.stableLinks);
        }
        let tailTokens: Token[] = lexer.lex(mutableInput);
        let discoveredNewDef = Object.keys(lexer.tokens.links).length > this.stableLinkCount;

        // Forward reference definition `[id]: url` arriving after `[id]` in frozen prefix:
        // re-lex full normalized source so earlier reference links resolve identically to `parseMarkdown(source)`.
        if (reusedPrefix && discoveredNewDef && this.stableMayHaveBracket) {
            this.prefixResets++;
            this.resetStableState();
            mutableInput = normalized;
            this.lastParsedLength = mutableInput.length;
            this.totalParsedChars += mutableInput.length;
            lexer = new Lexer(MARKED_OPTIONS);
            tailTokens = lexer.lex(mutableInput);
            discoveredNewDef = Object.keys(lexer.tokens.links).length > 0;
        }

        const cutSpaceIndex = findSafeFreezeSpaceIndex(tailTokens);
        if (cutSpaceIndex >= 0) {
            const remainingTokens = tailTokens.slice(cutSpaceIndex + 1);
            const holdForMutableDef = discoveredNewDef && hasDefToken(remainingTokens);
            if (!holdForMutableDef) {
                const tailRaw = remainingTokens.map(token => token.raw).join('');
                const cutOffset = mutableInput.length - tailRaw.length;
                if (
                    cutOffset > 0
                    && mutableInput.charCodeAt(cutOffset - 1) === 10
                    && mutableInput.endsWith(tailRaw)
                ) {
                    const frozenChunkSource = mutableInput.slice(0, cutOffset);
                    const newlyFrozenBlocks = blocks(tailTokens.slice(0, cutSpaceIndex + 1));
                    for (const block of newlyFrozenBlocks) {
                        const metric = indexSingleBlock(block, this.stableTextLength, this.nodeOffsets);
                        this.stableBlocks.push(block);
                        this.stableBlockMetrics.push(metric);
                        this.stableText += metric.text;
                        this.stableTextLength += metric.length;
                    }
                    this.stablePrefix += frozenChunkSource;
                    if (!this.stableMayHaveBracket && frozenChunkSource.includes('[')) {
                        this.stableMayHaveBracket = true;
                    }
                    if (discoveredNewDef) {
                        Object.assign(this.stableLinks, lexer.tokens.links);
                        this.stableLinkCount = Object.keys(this.stableLinks).length;
                    }
                    mutableInput = tailRaw;
                    tailTokens = remainingTokens;
                }
            }
        }

        const tailBlocks = blocks(tailTokens);
        const tailMetrics: MarkdownBlockMetric[] = [];
        let tailText = '';
        let cursor = this.stableTextLength;
        for (const block of tailBlocks) {
            const metric = indexSingleBlock(block, cursor, this.nodeOffsets);
            tailMetrics.push(metric);
            tailText += metric.text;
            cursor += metric.length;
        }
        this.tailBlockMetrics = tailMetrics;

        const combinedBlocks = this.stableBlocks.length > 0
            ? this.stableBlocks.concat(tailBlocks)
            : tailBlocks;
        const combinedMetrics = this.stableBlockMetrics.length > 0
            ? this.stableBlockMetrics.concat(tailMetrics)
            : tailMetrics;
        const offsets = this.nodeOffsets;
        this.lastSnapshot = {
            blocks: combinedBlocks,
            stablePrefixSource: this.stablePrefix,
            mutableTailSource: mutableInput,
            stableBlockCount: this.stableBlocks.length,
            text: this.stableText + tailText,
            blockMetrics: combinedMetrics,
            offsetOf: (node: object) => offsets.get(node) ?? 0,
        };
        return this.lastSnapshot;
    }

    search(query: string): SearchMatch[] {
        if (!query) return [];
        if (query !== this.cachedSearchQuery || !this.cachedSearchPattern) {
            this.cachedSearchQuery = query;
            this.cachedSearchPattern = compileSearchPattern(query);
            this.cachedSearchStableCount = 0;
            this.cachedStableMatches = [];
        }
        const pattern = this.cachedSearchPattern;
        if (!pattern) return [];

        if (this.cachedSearchStableCount < this.stableBlockMetrics.length) {
            const newlyFrozen = this.stableBlockMetrics.slice(this.cachedSearchStableCount);
            collectBlockSearchMatches(
                newlyFrozen,
                this.cachedSearchStableCount,
                pattern,
                query.length,
                this.cachedStableMatches,
            );
            this.cachedSearchStableCount = this.stableBlockMetrics.length;
        }

        if (this.tailBlockMetrics.length === 0) {
            return this.cachedStableMatches.slice();
        }
        const tailMatches: SearchMatch[] = [];
        collectBlockSearchMatches(
            this.tailBlockMetrics,
            this.stableBlockMetrics.length,
            pattern,
            query.length,
            tailMatches,
        );
        return this.cachedStableMatches.length > 0
            ? this.cachedStableMatches.concat(tailMatches)
            : tailMatches;
    }
}

function inlineText(nodes: InlineNode[]): string {
    return nodes.map(node => {
        if (node.kind === 'br') return '\n';
        if ('children' in node) return inlineText(node.children);
        return node.text;
    }).join('');
}
function blockText(blocks: MarkdownBlock[]): string {
    return blocks.map(block => {
        switch (block.kind) {
            case 'paragraph': case 'heading': return inlineText(block.children);
            case 'code': return block.text;
            case 'quote': return blockText(block.children);
            case 'list': return block.items.map((item, index) =>
                `${block.ordered ? `${block.start + index}.` : '•'} ${item.checked === undefined ? '' : item.checked ? '[x] ' : '[ ] '}${blockText(item.children)}`).join('\n');
            case 'table': return [block.header, ...block.rows].map(row => row.map(inlineText).join('\t')).join('\n');
            case 'hr': return '—';
        }
    }).join('\n\n');
}

/** Plain reading text shared by copy and conversation previews. */
export function markdownPlainText(source: string): string {
    return blockText(parseMarkdown(source));
}
