import { Lexer, Marked } from 'marked';
import type { Links, Token, Tokens, TokenizerExtension } from 'marked';
import { decodeHTML } from 'entities';

export type InlineNode =
    | { kind: 'text' | 'code' | 'math'; text: string }
    | { kind: 'strong' | 'em' | 'del'; children: InlineNode[] }
    | { kind: 'link'; children: InlineNode[]; href?: string; title?: string }
    | { kind: 'image'; text: string; href?: string }
    | { kind: 'br' };
export type MarkdownBlock =
    | { kind: 'paragraph'; children: InlineNode[] }
    | { kind: 'heading'; depth: number; children: InlineNode[] }
    | { kind: 'code'; text: string; language: string }
    | { kind: 'math'; text: string }
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

type MathToken = Tokens.Generic & {
    type: 'mathBlock' | 'mathInline';
    text: string;
};

function matchInlineMath(src: string, offset: number): { raw: string; text: string } | null {
    if (src[offset] !== '$' || src[offset + 1] === '$') return null;
    if (offset > 0 && (src[offset - 1] === '\\' || /[A-Za-z0-9]/.test(src[offset - 1]))) return null;
    const afterOpen = src[offset + 1];
    if (!afterOpen || afterOpen === ' ' || afterOpen === '\t' || afterOpen === '\n' || afterOpen === '\r') return null;

    let braceDepth = 0;
    for (let i = offset + 1; i < src.length; i++) {
        const ch = src[i];
        if (ch === '\n' || ch === '\r') return null;
        if (ch === '\\') {
            if (i + 1 < src.length && src[i + 1] !== '\n' && src[i + 1] !== '\r') {
                i++;
                continue;
            }
            return null;
        }
        if (ch === '{') {
            braceDepth++;
            continue;
        }
        if (ch === '}') {
            braceDepth--;
            if (braceDepth < 0) return null;
            continue;
        }
        if (ch === '$') {
            if (src[i + 1] === '$') return null;
            const beforeClose = src[i - 1];
            if (!beforeClose || beforeClose === ' ' || beforeClose === '\t') return null;
            const afterClose = src[i + 1];
            if (afterClose && afterClose >= '0' && afterClose <= '9') return null;
            if (braceDepth !== 0) return null;
            const tex = src.slice(offset + 1, i);
            if (!tex.trim()) return null;
            return { raw: src.slice(offset, i + 1), text: tex };
        }
    }
    return null;
}

const mathBlockExtension: TokenizerExtension = {
    name: 'mathBlock',
    level: 'block',
    start(src: string) {
        const match = src.match(/(?:^|\n)[ \t]{0,3}\$\$/);
        if (!match || match.index === undefined) return undefined;
        return match[0].startsWith('\n') ? match.index + 1 : match.index;
    },
    tokenizer(src: string) {
        const singleLine = /^[ \t]{0,3}\$\$(?!\$)([^\n]+?)\$\$[ \t]*(?:\n|$)/.exec(src);
        if (singleLine && !singleLine[1].includes('$$')) {
            const tex = singleLine[1].trim();
            if (tex.length > 0) {
                return { type: 'mathBlock', raw: singleLine[0], text: tex };
            }
        }
        const openLine = /^[ \t]{0,3}\$\$[ \t]*\n/.exec(src);
        if (openLine) {
            let cursor = openLine[0].length;
            const bodyStart = cursor;
            while (cursor < src.length) {
                const nl = src.indexOf('\n', cursor);
                const lineEnd = nl === -1 ? src.length : nl;
                const line = src.slice(cursor, lineEnd);
                if (/^[ \t]{0,3}\$\$[ \t]*$/.test(line)) {
                    const tex = src.slice(bodyStart, cursor).trim();
                    if (tex.length > 0) {
                        const rawEnd = nl === -1 ? src.length : nl + 1;
                        return { type: 'mathBlock', raw: src.slice(0, rawEnd), text: tex };
                    }
                    return undefined;
                }
                if (line.trim().length === 0) return undefined;
                if (nl === -1) break;
                cursor = nl + 1;
            }
        }
        return undefined;
    },
};

const mathInlineExtension: TokenizerExtension = {
    name: 'mathInline',
    level: 'inline',
    start(src: string) {
        let idx = src.indexOf('$');
        while (idx !== -1) {
            if (matchInlineMath(src, idx)) return idx;
            idx = src.indexOf('$', idx + 1);
        }
        return undefined;
    },
    tokenizer(src: string) {
        const matched = matchInlineMath(src, 0);
        if (!matched) return undefined;
        return { type: 'mathInline', raw: matched.raw, text: matched.text };
    },
};

const markdownParser = new Marked(MARKED_OPTIONS);
markdownParser.use({ extensions: [mathBlockExtension, mathInlineExtension] });

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

export function markdownImageHref(value: string): string | undefined {
    const text = decodeHTML(value).trim();
    if (!text || /[\u0000-\u0020\u007f]/.test(text)) return;
    if (text.startsWith('/api/chat/')) {
        if (text.startsWith('//') || text.includes('..') || text.includes('\\')) return;
        return text;
    }
    try {
        const url = new URL(text);
        if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) return;
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
            case 'mathInline': return [{ kind: 'math', text: (token as MathToken).text }];
            case 'br': return [{ kind: 'br' }];
            case 'link': {
                const link = token as Tokens.Link;
                return [{ kind: 'link', children: inline(link.tokens, depth + 1), href: markdownHref(link.href), title: link.title ?? undefined }];
            }
            case 'image': {
                const image = token as Tokens.Image;
                return [{ kind: 'image', text: decodeHTML(image.text), href: markdownImageHref(image.href) }];
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
            case 'mathBlock': return [{ kind: 'math', text: (token as MathToken).text }];
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
    try { return blocks(markdownParser.lexer(source)); }
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
            if (node.kind === 'text' || node.kind === 'code' || node.kind === 'math') text += node.text;
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
        if (node.kind === 'code' || node.kind === 'math') {
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

        let lexer = new Lexer(markdownParser.defaults);
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
            lexer = new Lexer(markdownParser.defaults);
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
            case 'code': case 'math': return block.text;
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

function escapeHtml(text: string): string {
    return text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;');
}

function tok(cls: string, raw: string): string {
    return `<span class="tok-${cls}">${escapeHtml(raw)}</span>`;
}

type CodeLangSpec = {
    keywords: ReadonlySet<string>;
    types: ReadonlySet<string>;
    lineComment?: string;
    blockComment?: boolean;
    nestedBlockComment?: boolean;
    tripleStrings?: boolean;
    backtickStrings?: boolean;
    singleQuoteStrings?: boolean;
    stringPrefixes?: boolean;
    rustRawStrings?: boolean;
    rustLifetimes?: boolean;
    rustMacros?: boolean;
    rustAttrs?: boolean;
    atDecorators?: boolean;
    hashDirectives?: boolean;
    caseInsensitive?: boolean;
    pascalTypes?: boolean;
};

const RUST_SPEC: CodeLangSpec = {
    keywords: new Set([
        'as', 'async', 'await', 'break', 'const', 'continue', 'crate', 'dyn', 'else', 'enum',
        'extern', 'false', 'fn', 'for', 'if', 'impl', 'in', 'let', 'loop', 'match', 'mod',
        'move', 'mut', 'pub', 'ref', 'return', 'self', 'static', 'struct', 'super', 'trait',
        'true', 'type', 'unsafe', 'use', 'where', 'while',
    ]),
    types: new Set([
        'bool', 'char', 'str', 'i8', 'i16', 'i32', 'i64', 'i128', 'isize',
        'u8', 'u16', 'u32', 'u64', 'u128', 'usize', 'f32', 'f64', 'Self',
        'String', 'Vec', 'Option', 'Result', 'Box', 'Arc', 'Rc', 'HashMap', 'BTreeMap', 'HashSet', 'BTreeSet',
    ]),
    lineComment: '//',
    blockComment: true,
    nestedBlockComment: true,
    rustRawStrings: true,
    rustLifetimes: true,
    rustMacros: true,
    rustAttrs: true,
    pascalTypes: true,
};

const TS_JS_SPEC: CodeLangSpec = {
    keywords: new Set([
        'as', 'async', 'await', 'break', 'case', 'catch', 'class', 'const', 'continue', 'debugger',
        'declare', 'default', 'delete', 'do', 'else', 'enum', 'export', 'extends', 'false', 'finally',
        'for', 'from', 'function', 'get', 'if', 'implements', 'import', 'in', 'infer', 'instanceof',
        'interface', 'is', 'keyof', 'let', 'module', 'namespace', 'new', 'null', 'of', 'override',
        'private', 'protected', 'public', 'readonly', 'return', 'satisfies', 'set', 'static', 'super',
        'switch', 'this', 'throw', 'true', 'try', 'type', 'typeof', 'undefined', 'var', 'void',
        'while', 'with', 'yield',
    ]),
    types: new Set([
        'any', 'bigint', 'boolean', 'never', 'number', 'object', 'string', 'symbol', 'unknown',
        'Promise', 'Array', 'Record', 'Map', 'Set', 'WeakMap', 'WeakSet', 'Partial', 'Required', 'Readonly',
    ]),
    lineComment: '//',
    blockComment: true,
    singleQuoteStrings: true,
    backtickStrings: true,
    atDecorators: true,
    pascalTypes: true,
};

const PYTHON_SPEC: CodeLangSpec = {
    keywords: new Set([
        'False', 'None', 'True', 'and', 'as', 'assert', 'async', 'await', 'break', 'case',
        'class', 'continue', 'def', 'del', 'elif', 'else', 'except', 'finally', 'for', 'from',
        'global', 'if', 'import', 'in', 'is', 'lambda', 'match', 'nonlocal', 'not', 'or',
        'pass', 'raise', 'return', 'try', 'while', 'with', 'yield',
    ]),
    types: new Set([
        'int', 'float', 'str', 'bool', 'bytes', 'list', 'dict', 'set', 'tuple', 'object',
        'Any', 'Optional', 'Union', 'Callable', 'Sequence', 'Mapping',
    ]),
    lineComment: '#',
    tripleStrings: true,
    singleQuoteStrings: true,
    stringPrefixes: true,
    atDecorators: true,
    pascalTypes: true,
};

const SWIFT_SPEC: CodeLangSpec = {
    keywords: new Set([
        'actor', 'any', 'as', 'associatedtype', 'async', 'await', 'break', 'case', 'catch', 'class',
        'continue', 'default', 'defer', 'deinit', 'didSet', 'do', 'else', 'enum', 'extension',
        'fallthrough', 'false', 'fileprivate', 'for', 'func', 'get', 'guard', 'if', 'import', 'in',
        'indirect', 'infix', 'init', 'inout', 'internal', 'is', 'isolated', 'lazy', 'let', 'mutating',
        'nil', 'nonisolated', 'nonmutating', 'open', 'operator', 'optional', 'override', 'postfix',
        'prefix', 'private', 'protocol', 'public', 'repeat', 'rethrows', 'return', 'self', 'set',
        'some', 'static', 'struct', 'subscript', 'super', 'switch', 'throw', 'throws', 'true', 'try',
        'typealias', 'unowned', 'var', 'weak', 'where', 'while', 'willSet',
    ]),
    types: new Set([
        'Int', 'Int8', 'Int16', 'Int32', 'Int64', 'UInt', 'UInt8', 'UInt16', 'UInt32', 'UInt64',
        'Float', 'Double', 'Bool', 'String', 'Character', 'Array', 'Dictionary', 'Set', 'Optional',
        'Any', 'AnyObject', 'Void', 'Never', 'Self',
    ]),
    lineComment: '//',
    blockComment: true,
    nestedBlockComment: true,
    tripleStrings: true,
    atDecorators: true,
    hashDirectives: true,
    pascalTypes: true,
};

const KOTLIN_SPEC: CodeLangSpec = {
    keywords: new Set([
        'abstract', 'actual', 'annotation', 'as', 'break', 'by', 'catch', 'class', 'companion',
        'const', 'constructor', 'continue', 'crossinline', 'data', 'do', 'else', 'enum', 'expect',
        'external', 'false', 'final', 'finally', 'for', 'fun', 'get', 'if', 'import', 'in', 'infix',
        'init', 'inline', 'inner', 'interface', 'internal', 'is', 'lateinit', 'noinline', 'null',
        'object', 'open', 'operator', 'out', 'override', 'package', 'private', 'protected', 'public',
        'reified', 'return', 'sealed', 'set', 'super', 'suspend', 'tailrec', 'this', 'throw', 'true',
        'try', 'typealias', 'val', 'value', 'var', 'vararg', 'when', 'where', 'while',
    ]),
    types: new Set([
        'Int', 'Long', 'Short', 'Byte', 'Float', 'Double', 'Boolean', 'Char', 'String', 'Unit',
        'Nothing', 'Any', 'List', 'MutableList', 'Map', 'MutableMap', 'Set', 'MutableSet',
    ]),
    lineComment: '//',
    blockComment: true,
    tripleStrings: true,
    singleQuoteStrings: true,
    atDecorators: true,
    pascalTypes: true,
};

const GO_SPEC: CodeLangSpec = {
    keywords: new Set([
        'break', 'case', 'chan', 'const', 'continue', 'default', 'defer', 'else', 'fallthrough',
        'false', 'for', 'func', 'go', 'goto', 'if', 'import', 'interface', 'iota', 'map', 'nil',
        'package', 'range', 'return', 'select', 'struct', 'switch', 'true', 'type', 'var',
    ]),
    types: new Set([
        'any', 'bool', 'byte', 'comparable', 'complex64', 'complex128', 'error', 'float32', 'float64',
        'int', 'int8', 'int16', 'int32', 'int64', 'rune', 'string', 'uint', 'uint8', 'uint16',
        'uint32', 'uint64', 'uintptr',
    ]),
    lineComment: '//',
    blockComment: true,
    singleQuoteStrings: true,
    backtickStrings: true,
    pascalTypes: true,
};

const SQL_SPEC: CodeLangSpec = {
    keywords: new Set([
        'all', 'alter', 'and', 'as', 'asc', 'begin', 'between', 'by', 'case', 'cast', 'check',
        'coalesce', 'commit', 'conflict', 'create', 'cross', 'default', 'delete', 'desc', 'distinct',
        'do', 'drop', 'else', 'end', 'exists', 'explain', 'false', 'foreign', 'from', 'full',
        'group', 'having', 'in', 'index', 'inner', 'insert', 'into', 'is', 'join', 'key', 'left',
        'like', 'limit', 'not', 'nothing', 'null', 'offset', 'on', 'or', 'order', 'outer', 'pragma',
        'primary', 'recursive', 'references', 'returning', 'right', 'rollback', 'select', 'set',
        'table', 'then', 'transaction', 'true', 'union', 'unique', 'update', 'vacuum', 'values',
        'view', 'when', 'where', 'with',
    ]),
    types: new Set([
        'bigint', 'blob', 'bool', 'boolean', 'char', 'date', 'decimal', 'double', 'float', 'int',
        'integer', 'json', 'jsonb', 'numeric', 'real', 'smallint', 'text', 'time', 'timestamp',
        'uuid', 'varchar',
    ]),
    lineComment: '--',
    blockComment: true,
    singleQuoteStrings: true,
    backtickStrings: true,
    caseInsensitive: true,
};

const BASH_KEYWORDS = new Set([
    'break', 'case', 'cd', 'continue', 'declare', 'do', 'done', 'echo', 'elif', 'else', 'esac',
    'eval', 'exec', 'exit', 'export', 'false', 'fi', 'for', 'function', 'if', 'in', 'local',
    'printf', 'pwd', 'readonly', 'return', 'set', 'shift', 'source', 'test', 'then', 'trap',
    'true', 'typeset', 'unset', 'until', 'while',
]);

const YAML_KEYWORDS = new Set(['true', 'false', 'null', 'yes', 'no', 'on', 'off', '~']);

function isIdentStart(ch: string): boolean {
    return (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || ch === '_' || ch === '$';
}

function isIdentPart(ch: string): boolean {
    return isIdentStart(ch) || (ch >= '0' && ch <= '9');
}

function isDigit(ch: string): boolean {
    return ch >= '0' && ch <= '9';
}

function isPascalType(word: string): boolean {
    if (word.length < 2) return false;
    const first = word[0];
    if (first < 'A' || first > 'Z') return false;
    let hasLower = false;
    for (let i = 1; i < word.length; i++) {
        const ch = word[i];
        if (ch >= 'a' && ch <= 'z') {
            hasLower = true;
            break;
        }
    }
    return hasLower;
}

function scanQuotedString(code: string, start: number, quote: string): number {
    let i = start + 1;
    while (i < code.length) {
        const ch = code[i];
        if (ch === '\\' && quote !== '`') {
            i += 2;
            continue;
        }
        if (ch === quote) {
            // SQL '' escaped quote inside single-quoted string is handled naturally on next pass if adjacent
            return i + 1;
        }
        if (ch === '\n' && quote !== '`') {
            return i;
        }
        i++;
    }
    return i;
}

function scanNumber(code: string, start: number): number {
    let i = start;
    if (code[i] === '0' && i + 1 < code.length) {
        const next = code[i + 1];
        if (next === 'x' || next === 'X') {
            i += 2;
            while (i < code.length && /[0-9a-fA-F_]/.test(code[i])) i++;
            return i;
        }
        if (next === 'b' || next === 'B') {
            i += 2;
            while (i < code.length && /[01_]/.test(code[i])) i++;
            return i;
        }
        if (next === 'o' || next === 'O') {
            i += 2;
            while (i < code.length && /[0-7_]/.test(code[i])) i++;
            return i;
        }
    }
    while (i < code.length && (isDigit(code[i]) || code[i] === '_')) i++;
    if (code[i] === '.' && i + 1 < code.length && isDigit(code[i + 1])) {
        i++;
        while (i < code.length && (isDigit(code[i]) || code[i] === '_')) i++;
    }
    if ((code[i] === 'e' || code[i] === 'E') && i + 1 < code.length) {
        let j = i + 1;
        if (code[j] === '+' || code[j] === '-') j++;
        if (j < code.length && isDigit(code[j])) {
            i = j;
            while (i < code.length && (isDigit(code[i]) || code[i] === '_')) i++;
        }
    }
    // Optional numeric suffixes (e.g. u64, usize, f32, n, L)
    if (i < code.length && /[uifnLFl]/.test(code[i])) {
        let j = i;
        while (j < code.length && /[A-Za-z0-9]/.test(code[j])) j++;
        const suffix = code.slice(i, j);
        if (/^(?:[uif](?:8|16|32|64|128)|usize|isize|n|L|UL|f|F|l)$/.test(suffix)) {
            i = j;
        }
    }
    return i;
}

function highlightCodeSpec(code: string, spec: CodeLangSpec): string {
    const out: string[] = [];
    let i = 0;
    const len = code.length;

    while (i < len) {
        const ch = code[i];

        // Line comment
        if (spec.lineComment && code.startsWith(spec.lineComment, i)) {
            let end = code.indexOf('\n', i);
            if (end === -1) end = len;
            out.push(tok('cmt', code.slice(i, end)));
            i = end;
            continue;
        }

        // Block comment
        if (spec.blockComment && ch === '/' && code[i + 1] === '*') {
            let end = i + 2;
            let depth = 1;
            while (end < len && depth > 0) {
                if (spec.nestedBlockComment && code[end] === '/' && code[end + 1] === '*') {
                    depth++;
                    end += 2;
                } else if (code[end] === '*' && code[end + 1] === '/') {
                    depth--;
                    end += 2;
                } else {
                    end++;
                }
            }
            out.push(tok('cmt', code.slice(i, end)));
            i = end;
            continue;
        }

        // Rust attributes #[...] / #![...]
        if (spec.rustAttrs && ch === '#' && (code[i + 1] === '[' || (code[i + 1] === '!' && code[i + 2] === '['))) {
            let end = code[i + 1] === '!' ? i + 3 : i + 2;
            let depth = 1;
            while (end < len && depth > 0) {
                if (code[end] === '[') depth++;
                else if (code[end] === ']') depth--;
                else if (code[end] === '"') {
                    end = scanQuotedString(code, end, '"');
                    continue;
                }
                end++;
            }
            out.push(tok('meta', code.slice(i, end)));
            i = end;
            continue;
        }

        // Swift compiler directives (#if, #elseif, #else, #endif, #available, #selector)
        if (spec.hashDirectives && ch === '#' && i + 1 < len && isIdentStart(code[i + 1])) {
            let end = i + 2;
            while (end < len && isIdentPart(code[end])) end++;
            out.push(tok('meta', code.slice(i, end)));
            i = end;
            continue;
        }

        // @decorators / @annotations / @MainActor
        if (spec.atDecorators && ch === '@' && i + 1 < len && isIdentStart(code[i + 1])) {
            let end = i + 2;
            while (end < len && (isIdentPart(code[end]) || code[end] === '.')) end++;
            out.push(tok('meta', code.slice(i, end)));
            i = end;
            continue;
        }

        // Rust raw strings: r"...", r#"..."#, br#"..."#
        if (spec.rustRawStrings && (ch === 'r' || (ch === 'b' && code[i + 1] === 'r'))) {
            const rPos = ch === 'b' ? i + 1 : i;
            let hashes = 0;
            let p = rPos + 1;
            while (p < len && code[p] === '#') {
                hashes++;
                p++;
            }
            if (p < len && code[p] === '"') {
                const closeSeq = '"' + '#'.repeat(hashes);
                const closeIdx = code.indexOf(closeSeq, p + 1);
                const end = closeIdx === -1 ? len : closeIdx + closeSeq.length;
                out.push(tok('str', code.slice(i, end)));
                i = end;
                continue;
            }
        }

        // Python string prefixes (r"...", f'...', rb"""...""")
        if (spec.stringPrefixes && /[rfbuRFBU]/.test(ch)) {
            let p = i + 1;
            if (p < len && /[rfbuRFBU]/.test(code[p]) && code[p].toLowerCase() !== ch.toLowerCase()) p++;
            if (p < len && (code[p] === '"' || code[p] === '\'')) {
                const quote = code[p];
                if (spec.tripleStrings && code.startsWith(quote.repeat(3), p)) {
                    let end = p + 3;
                    while (end < len) {
                        if (code[end] === '\\') { end += 2; continue; }
                        if (code.startsWith(quote.repeat(3), end)) { end += 3; break; }
                        end++;
                    }
                    out.push(tok('str', code.slice(i, end)));
                    i = end;
                    continue;
                }
                const end = scanQuotedString(code, p, quote);
                out.push(tok('str', code.slice(i, end)));
                i = end;
                continue;
            }
        }

        // Triple-quoted strings
        if (spec.tripleStrings && ((ch === '"' && code[i + 1] === '"' && code[i + 2] === '"')
            || (spec.singleQuoteStrings && ch === '\'' && code[i + 1] === '\'' && code[i + 2] === '\''))) {
            const delim = ch.repeat(3);
            let end = i + 3;
            while (end < len) {
                if (code[end] === '\\') { end += 2; continue; }
                if (code.startsWith(delim, end)) { end += 3; break; }
                end++;
            }
            out.push(tok('str', code.slice(i, end)));
            i = end;
            continue;
        }

        // Double-quoted strings
        if (ch === '"') {
            const end = scanQuotedString(code, i, '"');
            out.push(tok('str', code.slice(i, end)));
            i = end;
            continue;
        }

        // Backtick strings (TS template / Go raw / SQL identifier)
        if (spec.backtickStrings && ch === '`') {
            let end = i + 1;
            while (end < len) {
                if (code[end] === '\\' && spec === TS_JS_SPEC) { end += 2; continue; }
                if (code[end] === '`') { end++; break; }
                end++;
            }
            out.push(tok('str', code.slice(i, end)));
            i = end;
            continue;
        }

        // Rust char literals vs lifetimes ('a, 'static vs 'x', '\n')
        if (spec.rustLifetimes && ch === '\'') {
            if (code[i + 1] === '\\') {
                const close = code.indexOf('\'', i + 2);
                if (close !== -1 && close - i <= 12) {
                    out.push(tok('str', code.slice(i, close + 1)));
                    i = close + 1;
                    continue;
                }
            } else if (i + 2 < len && code[i + 2] === '\'' && code[i + 1] !== '\n') {
                out.push(tok('str', code.slice(i, i + 3)));
                i += 3;
                continue;
            } else if (i + 1 < len && isIdentStart(code[i + 1])) {
                let end = i + 2;
                while (end < len && isIdentPart(code[end])) end++;
                out.push(tok('meta', code.slice(i, end)));
                i = end;
                continue;
            }
        }

        // Single-quoted strings
        if (spec.singleQuoteStrings && ch === '\'') {
            const end = scanQuotedString(code, i, '\'');
            out.push(tok('str', code.slice(i, end)));
            i = end;
            continue;
        }

        // Numbers
        if (isDigit(ch) && (i === 0 || !isIdentPart(code[i - 1]))) {
            const end = scanNumber(code, i);
            out.push(tok('num', code.slice(i, end)));
            i = end;
            continue;
        }

        // Identifiers, keywords, types, Rust macros
        if (isIdentStart(ch)) {
            let end = i + 1;
            while (end < len && isIdentPart(code[end])) end++;
            const word = code.slice(i, end);
            if (spec.rustMacros && code[end] === '!' && code[end + 1] !== '=') {
                out.push(tok('meta', word + '!'));
                i = end + 1;
                continue;
            }
            const lookup = spec.caseInsensitive ? word.toLowerCase() : word;
            if (spec.keywords.has(lookup)) {
                out.push(tok('kw', word));
            } else if (spec.types.has(lookup) || (spec.pascalTypes && isPascalType(word))) {
                out.push(tok('type', word));
            } else {
                out.push(escapeHtml(word));
            }
            i = end;
            continue;
        }

        // Plain character
        out.push(escapeHtml(ch));
        i++;
    }

    return out.join('');
}

function highlightJson(code: string): string {
    const out: string[] = [];
    let i = 0;
    const len = code.length;
    while (i < len) {
        const ch = code[i];
        if (ch === '/' && code[i + 1] === '/') {
            let end = code.indexOf('\n', i);
            if (end === -1) end = len;
            out.push(tok('cmt', code.slice(i, end)));
            i = end;
            continue;
        }
        if (ch === '/' && code[i + 1] === '*') {
            const close = code.indexOf('*/', i + 2);
            const end = close === -1 ? len : close + 2;
            out.push(tok('cmt', code.slice(i, end)));
            i = end;
            continue;
        }
        if (ch === '"') {
            const end = scanQuotedString(code, i, '"');
            let after = end;
            while (after < len && (code[after] === ' ' || code[after] === '\t')) after++;
            out.push(tok(code[after] === ':' ? 'key' : 'str', code.slice(i, end)));
            i = end;
            continue;
        }
        if ((ch === '-' && isDigit(code[i + 1] ?? '')) || isDigit(ch)) {
            const start = i;
            if (ch === '-') i++;
            const end = scanNumber(code, i);
            out.push(tok('num', code.slice(start, end)));
            i = end;
            continue;
        }
        if (isIdentStart(ch)) {
            let end = i + 1;
            while (end < len && isIdentPart(code[end])) end++;
            const word = code.slice(i, end);
            if (word === 'true' || word === 'false' || word === 'null') out.push(tok('kw', word));
            else out.push(escapeHtml(word));
            i = end;
            continue;
        }
        out.push(escapeHtml(ch));
        i++;
    }
    return out.join('');
}

function highlightYaml(code: string): string {
    return code.split('\n').map(line => {
        const trimmed = line.trim();
        if (trimmed === '---' || trimmed === '...') return tok('meta', line);
        if (trimmed.startsWith('#')) return tok('cmt', line);

        // Split inline comment outside quotes
        let commentIdx = -1;
        let quote: string | null = null;
        for (let i = 0; i < line.length; i++) {
            const ch = line[i];
            if (quote) {
                if (ch === '\\' && quote === '"') { i++; continue; }
                if (ch === quote) quote = null;
            } else if (ch === '"' || ch === '\'') {
                quote = ch;
            } else if (ch === '#' && (i === 0 || line[i - 1] === ' ' || line[i - 1] === '\t')) {
                commentIdx = i;
                break;
            }
        }
        const body = commentIdx >= 0 ? line.slice(0, commentIdx) : line;
        const comment = commentIdx >= 0 ? tok('cmt', line.slice(commentIdx)) : '';

        const keyMatch = /^(\s*(?:-\s+)?)("(?:\\.|[^"])*"|'(?:''|[^'])*'|[A-Za-z0-9_./-]+)(\s*:(?:\s|$))(.*)$/.exec(body);
        if (keyMatch) {
            const [, indent, key, colon, rest] = keyMatch;
            return `${escapeHtml(indent)}${tok('key', key)}${escapeHtml(colon)}${highlightYamlValue(rest)}${comment}`;
        }
        const listMatch = /^(\s*-\s+)(.*)$/.exec(body);
        if (listMatch) {
            return `${escapeHtml(listMatch[1])}${highlightYamlValue(listMatch[2])}${comment}`;
        }
        return `${highlightYamlValue(body)}${comment}`;
    }).join('\n');
}

function highlightYamlValue(value: string): string {
    const trimmed = value.trim();
    if (!trimmed) return escapeHtml(value);
    const leadLen = value.indexOf(trimmed);
    const lead = escapeHtml(value.slice(0, leadLen));
    const trail = escapeHtml(value.slice(leadLen + trimmed.length));
    if ((trimmed.startsWith('"') && trimmed.endsWith('"')) || (trimmed.startsWith('\'') && trimmed.endsWith('\''))) {
        return `${lead}${tok('str', trimmed)}${trail}`;
    }
    if (trimmed.startsWith('&') || trimmed.startsWith('*') || trimmed.startsWith('!')) {
        return `${lead}${tok('meta', trimmed)}${trail}`;
    }
    if (YAML_KEYWORDS.has(trimmed.toLowerCase())) {
        return `${lead}${tok('kw', trimmed)}${trail}`;
    }
    if (/^-?(?:0x[0-9a-fA-F_]+|0o[0-7_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)$/.test(trimmed)) {
        return `${lead}${tok('num', trimmed)}${trail}`;
    }
    return escapeHtml(value);
}

function highlightBash(code: string): string {
    const out: string[] = [];
    let i = 0;
    const len = code.length;
    while (i < len) {
        const ch = code[i];
        if (ch === '#' && (i === 0 || /[\s;|&(]/.test(code[i - 1]))) {
            let end = code.indexOf('\n', i);
            if (end === -1) end = len;
            out.push(tok('cmt', code.slice(i, end)));
            i = end;
            continue;
        }
        if (ch === '"' || ch === '\'') {
            const end = scanQuotedString(code, i, ch);
            out.push(tok('str', code.slice(i, end)));
            i = end;
            continue;
        }
        if (ch === '$') {
            if (code[i + 1] === '{') {
                const close = code.indexOf('}', i + 2);
                const end = close === -1 ? len : close + 1;
                out.push(tok('var', code.slice(i, end)));
                i = end;
                continue;
            }
            if (i + 1 < len && /[A-Za-z0-9_?@#*$!-]/.test(code[i + 1])) {
                let end = i + 2;
                if (isIdentStart(code[i + 1])) {
                    while (end < len && isIdentPart(code[end])) end++;
                }
                out.push(tok('var', code.slice(i, end)));
                i = end;
                continue;
            }
        }
        if (isDigit(ch) && (i === 0 || /[\s=:(,[]/.test(code[i - 1]))) {
            let end = i + 1;
            while (end < len && isDigit(code[end])) end++;
            if (end === len || /[\s;)|&\],]/.test(code[end])) {
                out.push(tok('num', code.slice(i, end)));
                i = end;
                continue;
            }
        }
        if (isIdentStart(ch) && (i === 0 || !isIdentPart(code[i - 1]) && code[i - 1] !== '-')) {
            let end = i + 1;
            while (end < len && (isIdentPart(code[end]) || code[end] === '-')) end++;
            const word = code.slice(i, end);
            if (BASH_KEYWORDS.has(word) && code[end] !== '=') {
                out.push(tok('kw', word));
            } else if (code[end] === '=' && /^[A-Za-z_][A-Za-z0-9_]*$/.test(word)) {
                out.push(tok('var', word));
            } else {
                out.push(escapeHtml(word));
            }
            i = end;
            continue;
        }
        out.push(escapeHtml(ch));
        i++;
    }
    return out.join('');
}

function highlightHtml(code: string): string {
    const out: string[] = [];
    let i = 0;
    const len = code.length;
    while (i < len) {
        if (code.startsWith('<!--', i)) {
            const close = code.indexOf('-->', i + 4);
            const end = close === -1 ? len : close + 3;
            out.push(tok('cmt', code.slice(i, end)));
            i = end;
            continue;
        }
        if (code.startsWith('<!', i)) {
            const close = code.indexOf('>', i + 2);
            const end = close === -1 ? len : close + 1;
            out.push(tok('meta', code.slice(i, end)));
            i = end;
            continue;
        }
        if (code[i] === '<' && i + 1 < len && (code[i + 1] === '/' || /[A-Za-z]/.test(code[i + 1]))) {
            let p = i + 1;
            if (code[p] === '/') p++;
            while (p < len && /[A-Za-z0-9:_-]/.test(code[p])) p++;
            out.push(tok('tag', code.slice(i, p)));
            i = p;
            while (i < len && code[i] !== '>') {
                if (code[i] === '/' && code[i + 1] === '>') {
                    out.push(tok('tag', '/>'));
                    i += 2;
                    break;
                }
                if (/\s/.test(code[i])) {
                    out.push(escapeHtml(code[i]));
                    i++;
                    continue;
                }
                if (code[i] === '"' || code[i] === '\'') {
                    const end = scanQuotedString(code, i, code[i]);
                    out.push(tok('str', code.slice(i, end)));
                    i = end;
                    continue;
                }
                if (/[A-Za-z0-9:@_.-]/.test(code[i])) {
                    let end = i + 1;
                    while (end < len && /[A-Za-z0-9:@_.-]/.test(code[end])) end++;
                    out.push(tok('attr', code.slice(i, end)));
                    i = end;
                    if (code[i] === '=') {
                        out.push('=');
                        i++;
                        if (i < len && (code[i] === '"' || code[i] === '\'')) {
                            const strEnd = scanQuotedString(code, i, code[i]);
                            out.push(tok('str', code.slice(i, strEnd)));
                            i = strEnd;
                        } else if (i < len && !/[\s>]/.test(code[i])) {
                            let valEnd = i;
                            while (valEnd < len && !/[\s>]/.test(code[valEnd])) valEnd++;
                            out.push(tok('str', code.slice(i, valEnd)));
                            i = valEnd;
                        }
                    }
                    continue;
                }
                out.push(escapeHtml(code[i]));
                i++;
            }
            if (i < len && code[i] === '>') {
                out.push(tok('tag', '>'));
                i++;
            }
            continue;
        }
        out.push(escapeHtml(code[i]));
        i++;
    }
    return out.join('');
}

function highlightCss(code: string): string {
    const out: string[] = [];
    let i = 0;
    const len = code.length;
    while (i < len) {
        const ch = code[i];
        if (ch === '/' && code[i + 1] === '*') {
            const close = code.indexOf('*/', i + 2);
            const end = close === -1 ? len : close + 2;
            out.push(tok('cmt', code.slice(i, end)));
            i = end;
            continue;
        }
        if (ch === '/' && code[i + 1] === '/') {
            let end = code.indexOf('\n', i);
            if (end === -1) end = len;
            out.push(tok('cmt', code.slice(i, end)));
            i = end;
            continue;
        }
        if (ch === '"' || ch === '\'') {
            const end = scanQuotedString(code, i, ch);
            out.push(tok('str', code.slice(i, end)));
            i = end;
            continue;
        }
        if (ch === '@' && i + 1 < len && /[A-Za-z-]/.test(code[i + 1])) {
            let end = i + 2;
            while (end < len && /[A-Za-z0-9_-]/.test(code[end])) end++;
            out.push(tok('kw', code.slice(i, end)));
            i = end;
            continue;
        }
        if (ch === '-' && code[i + 1] === '-' && i + 2 < len && /[A-Za-z0-9_-]/.test(code[i + 2])) {
            let end = i + 2;
            while (end < len && /[A-Za-z0-9_-]/.test(code[end])) end++;
            out.push(tok('var', code.slice(i, end)));
            i = end;
            continue;
        }
        if (ch === '#' && i + 1 < len && /[0-9a-fA-F]/.test(code[i + 1])) {
            let end = i + 1;
            while (end < len && /[0-9a-fA-F]/.test(code[end])) end++;
            const hexLen = end - i - 1;
            if (hexLen === 3 || hexLen === 4 || hexLen === 6 || hexLen === 8) {
                out.push(tok('num', code.slice(i, end)));
                i = end;
                continue;
            }
        }
        if (isDigit(ch) || (ch === '.' && isDigit(code[i + 1] ?? ''))) {
            let end = i + 1;
            while (end < len && (isDigit(code[end]) || code[end] === '.')) end++;
            while (end < len && (/[A-Za-z%]/.test(code[end]))) end++;
            out.push(tok('num', code.slice(i, end)));
            i = end;
            continue;
        }
        if (/[A-Za-z-]/.test(ch)) {
            let end = i + 1;
            while (end < len && /[A-Za-z0-9_-]/.test(code[end])) end++;
            let after = end;
            while (after < len && (code[after] === ' ' || code[after] === '\t')) after++;
            const word = code.slice(i, end);
            if (code[after] === ':' && code[after + 1] !== ':') {
                out.push(tok('attr', word));
            } else {
                out.push(escapeHtml(word));
            }
            i = end;
            continue;
        }
        out.push(escapeHtml(ch));
        i++;
    }
    return out.join('');
}

function highlightDiff(code: string): string {
    return code.split('\n').map(line => {
        if (
            line.startsWith('diff --git ')
            || line.startsWith('index ')
            || line.startsWith('--- ')
            || line.startsWith('+++ ')
            || line.startsWith('new file')
            || line.startsWith('deleted file')
            || line.startsWith('rename from ')
            || line.startsWith('rename to ')
        ) {
            return tok('meta', line);
        }
        if (line.startsWith('@@')) return tok('hunk', line);
        if (line.startsWith('+')) return tok('add', line);
        if (line.startsWith('-')) return tok('del', line);
        return escapeHtml(line);
    }).join('\n');
}

const LANGUAGE_ALIASES: Readonly<Record<string, string>> = {
    rs: 'rust', rust: 'rust',
    ts: 'ts', tsx: 'ts', typescript: 'ts', js: 'ts', jsx: 'ts', mjs: 'ts', cjs: 'ts', javascript: 'ts',
    py: 'python', py3: 'python', python: 'python',
    swift: 'swift',
    kt: 'kotlin', kts: 'kotlin', kotlin: 'kotlin',
    go: 'go', golang: 'go',
    json: 'json', jsonc: 'json',
    yaml: 'yaml', yml: 'yaml',
    sh: 'bash', bash: 'bash', zsh: 'bash', shell: 'bash', fish: 'bash',
    sql: 'sql', sqlite: 'sql', postgres: 'sql', postgresql: 'sql', mysql: 'sql',
    html: 'html', htm: 'html', xml: 'html', svg: 'html', svelte: 'html', vue: 'html',
    css: 'css', scss: 'css', less: 'css',
    diff: 'diff', patch: 'diff',
};

/** Infers a syntax-highlighter language identifier from a repository file path. */
export function languageFromPath(filePath?: string | null): string {
    if (!filePath) return '';
    const base = filePath.split('/').pop()?.toLowerCase() ?? '';
    const dot = base.lastIndexOf('.');
    if (dot <= 0) return '';
    const ext = base.slice(dot + 1);
    return LANGUAGE_ALIASES[ext] ?? '';
}

/**
 * XSS-safe, zero-dependency syntax highlighter for fenced code blocks and diffs.
 * Every input character is HTML-escaped before wrapping in `<span class="tok-...">`.
 */
export function highlightCode(code: string, lang?: string): string {
    if (!code) return '';
    const normalized = LANGUAGE_ALIASES[(lang ?? '').trim().toLowerCase()] ?? '';
    switch (normalized) {
        case 'rust': return highlightCodeSpec(code, RUST_SPEC);
        case 'ts': return highlightCodeSpec(code, TS_JS_SPEC);
        case 'python': return highlightCodeSpec(code, PYTHON_SPEC);
        case 'swift': return highlightCodeSpec(code, SWIFT_SPEC);
        case 'kotlin': return highlightCodeSpec(code, KOTLIN_SPEC);
        case 'go': return highlightCodeSpec(code, GO_SPEC);
        case 'sql': return highlightCodeSpec(code, SQL_SPEC);
        case 'json': return highlightJson(code);
        case 'yaml': return highlightYaml(code);
        case 'bash': return highlightBash(code);
        case 'html': return highlightHtml(code);
        case 'css': return highlightCss(code);
        case 'diff': return highlightDiff(code);
        default: return escapeHtml(code);
    }
}

const MATH_GREEK: Readonly<Record<string, string>> = {
    alpha: 'α', beta: 'β', gamma: 'γ', delta: 'δ', epsilon: 'ε', varepsilon: 'ε',
    zeta: 'ζ', eta: 'η', theta: 'θ', vartheta: 'ϑ', iota: 'ι', kappa: 'κ',
    lambda: 'λ', mu: 'μ', nu: 'ν', xi: 'ξ', omicron: 'ο', pi: 'π', varpi: 'ϖ',
    rho: 'ρ', varrho: 'ϱ', sigma: 'σ', varsigma: 'ς', tau: 'τ', upsilon: 'υ',
    phi: 'φ', varphi: 'ϕ', chi: 'χ', psi: 'ψ', omega: 'ω',
    Gamma: 'Γ', Delta: 'Δ', Theta: 'Θ', Lambda: 'Λ', Xi: 'Ξ', Pi: 'Π',
    Sigma: 'Σ', Upsilon: 'Υ', Phi: 'Φ', Psi: 'Ψ', Omega: 'Ω',
};

const MATH_SYMBOLS: Readonly<Record<string, string>> = {
    sum: '∑', prod: '∏', int: '∫', iint: '∬', iiint: '∭', oint: '∮', infty: '∞',
    pm: '±', mp: '∓', times: '×', div: '÷', cdot: '·', cdots: '⋯', ldots: '…', dots: '…', vdots: '⋮', ddots: '⋱',
    le: '≤', leq: '≤', ge: '≥', geq: '≥', ne: '≠', neq: '≠', approx: '≈', equiv: '≡',
    sim: '∼', simeq: '≃', cong: '≅', propto: '∝', ll: '≪', gg: '≫',
    to: '→', rightarrow: '→', leftarrow: '←', leftrightarrow: '↔',
    Rightarrow: '⇒', Leftarrow: '⇐', Leftrightarrow: '⇔', implies: '⇒', iff: '⇔', mapsto: '↦',
    in: '∈', notin: '∉', ni: '∋', subset: '⊂', supset: '⊃', subseteq: '⊆', supseteq: '⊇',
    cup: '∪', cap: '∩', emptyset: '∅', varnothing: '∅',
    forall: '∀', exists: '∃', nexists: '∄', neg: '¬', lnot: '¬',
    land: '∧', lor: '∨', wedge: '∧', vee: '∨',
    partial: '∂', nabla: '∇', degree: '°', angle: '∠', top: '⊤', bot: '⊥', perp: '⊥', parallel: '∥',
    prime: '′', dagger: '†', ddagger: '‡', ell: 'ℓ', hbar: 'ℏ', Re: 'ℜ', Im: 'ℑ', aleph: 'ℵ',
    circ: '∘', bullet: '•', star: '⋆', ast: '∗', setminus: '∖', oplus: '⊕', otimes: '⊗',
    langle: '⟨', rangle: '⟩', lfloor: '⌊', rfloor: '⌋', lceil: '⌈', rceil: '⌉',
    quad: '\u2001', qquad: '\u2001\u2001',
};

const MATH_OPERATORS = new Set([
    'sin', 'cos', 'tan', 'cot', 'sec', 'csc', 'arcsin', 'arccos', 'arctan',
    'sinh', 'cosh', 'tanh', 'log', 'ln', 'exp', 'lim', 'max', 'min', 'sup', 'inf',
    'det', 'dim', 'ker', 'deg', 'gcd', 'arg', 'Pr', 'mod',
]);

const MATH_BLACKBOARD: Readonly<Record<string, string>> = {
    C: 'ℂ', H: 'ℍ', N: 'ℕ', P: 'ℙ', Q: 'ℚ', R: 'ℝ', Z: 'ℤ',
};

function skipMathSpaces(src: string, pos: number, end: number): number {
    while (pos < end && (src[pos] === ' ' || src[pos] === '\t')) pos++;
    return pos;
}

function readMathGroup(src: string, start: number, end: number): { content: string; next: number } {
    const pos = skipMathSpaces(src, start, end);
    if (pos >= end) return { content: '', next: pos };
    if (src[pos] === '{') {
        let depth = 1;
        let i = pos + 1;
        while (i < end && depth > 0) {
            if (src[i] === '\\' && i + 1 < end) {
                i += 2;
                continue;
            }
            if (src[i] === '{') depth++;
            else if (src[i] === '}') depth--;
            if (depth === 0) {
                return { content: src.slice(pos + 1, i), next: i + 1 };
            }
            i++;
        }
        return { content: src.slice(pos + 1, end), next: end };
    }
    if (src[pos] === '\\' && pos + 1 < end) {
        if (/[A-Za-z]/.test(src[pos + 1])) {
            let i = pos + 2;
            while (i < end && /[A-Za-z]/.test(src[i])) i++;
            return { content: src.slice(pos, i), next: i };
        }
        return { content: src.slice(pos, pos + 2), next: pos + 2 };
    }
    return { content: src[pos], next: pos + 1 };
}

function formatMathRange(src: string, start: number, end: number, depth: number): string {
    if (depth > 24) return escapeHtml(src.slice(start, end));
    const out: string[] = [];
    let i = start;

    while (i < end) {
        const ch = src[i];

        if (ch === '\\' && i + 1 < end) {
            const next = src[i + 1];
            if (next === '\\') {
                out.push('<br />');
                i += 2;
                continue;
            }
            if (next === ',') {
                out.push('\u2009');
                i += 2;
                continue;
            }
            if (next === ';' || next === ':') {
                out.push('\u2005');
                i += 2;
                continue;
            }
            if (next === '!') {
                i += 2;
                continue;
            }
            if (next === ' ') {
                out.push('&nbsp;');
                i += 2;
                continue;
            }
            if (next === '|') {
                out.push('‖');
                i += 2;
                continue;
            }
            if ('{}%$_&#'.includes(next)) {
                out.push(escapeHtml(next));
                i += 2;
                continue;
            }
            if (/[A-Za-z]/.test(next)) {
                let cmdEnd = i + 2;
                while (cmdEnd < end && /[A-Za-z]/.test(src[cmdEnd])) cmdEnd++;
                const cmd = src.slice(i + 1, cmdEnd);

                if (cmd === 'frac' || cmd === 'dfrac' || cmd === 'tfrac' || cmd === 'cfrac') {
                    const num = readMathGroup(src, cmdEnd, end);
                    const den = readMathGroup(src, num.next, end);
                    out.push(
                        `<span class="math-frac"><span class="math-frac-num">${formatMathRange(num.content, 0, num.content.length, depth + 1)}</span><span class="math-frac-den">${formatMathRange(den.content, 0, den.content.length, depth + 1)}</span></span>`,
                    );
                    i = den.next;
                    continue;
                }
                if (cmd === 'sqrt') {
                    let p = skipMathSpaces(src, cmdEnd, end);
                    let rootHtml = '';
                    if (p < end && src[p] === '[') {
                        let r = p + 1;
                        while (r < end && src[r] !== ']') {
                            if (src[r] === '\\' && r + 1 < end) r += 2;
                            else r++;
                        }
                        if (r < end && src[r] === ']') {
                            const rootTex = src.slice(p + 1, r);
                            rootHtml = `<sup class="math-root">${formatMathRange(rootTex, 0, rootTex.length, depth + 1)}</sup>`;
                            p = r + 1;
                        }
                    }
                    const body = readMathGroup(src, p, end);
                    out.push(
                        `<span class="math-sqrt">${rootHtml}<span class="math-sqrt-sign" aria-hidden="true">√</span><span class="math-sqrt-body">${formatMathRange(body.content, 0, body.content.length, depth + 1)}</span></span>`,
                    );
                    i = body.next;
                    continue;
                }
                if (cmd === 'text' || cmd === 'mathrm' || cmd === 'operatorname') {
                    const body = readMathGroup(src, cmdEnd, end);
                    out.push(`<span class="math-text">${escapeHtml(body.content)}</span>`);
                    i = body.next;
                    continue;
                }
                if (cmd === 'mathbf' || cmd === 'boldsymbol' || cmd === 'bm') {
                    const body = readMathGroup(src, cmdEnd, end);
                    out.push(`<span class="math-bf">${formatMathRange(body.content, 0, body.content.length, depth + 1)}</span>`);
                    i = body.next;
                    continue;
                }
                if (cmd === 'mathbb') {
                    const body = readMathGroup(src, cmdEnd, end);
                    const trimmed = body.content.trim();
                    if (trimmed.length === 1 && Object.hasOwn(MATH_BLACKBOARD, trimmed)) {
                        out.push(MATH_BLACKBOARD[trimmed]);
                    } else {
                        out.push(`<span class="math-bb">${formatMathRange(body.content, 0, body.content.length, depth + 1)}</span>`);
                    }
                    i = body.next;
                    continue;
                }
                if (cmd === 'mathit' || cmd === 'mathcal') {
                    const body = readMathGroup(src, cmdEnd, end);
                    out.push(`<span class="${cmd === 'mathcal' ? 'math-cal' : 'math-it'}">${formatMathRange(body.content, 0, body.content.length, depth + 1)}</span>`);
                    i = body.next;
                    continue;
                }
                if (cmd === 'left' || cmd === 'right') {
                    const p = skipMathSpaces(src, cmdEnd, end);
                    i = p < end && src[p] === '.' ? p + 1 : p;
                    continue;
                }
                if (Object.hasOwn(MATH_GREEK, cmd)) {
                    out.push(MATH_GREEK[cmd]);
                    i = cmdEnd;
                    continue;
                }
                if (Object.hasOwn(MATH_SYMBOLS, cmd)) {
                    out.push(MATH_SYMBOLS[cmd]);
                    i = cmdEnd;
                    continue;
                }
                if (MATH_OPERATORS.has(cmd)) {
                    out.push(`<span class="math-op">${escapeHtml(cmd)}</span>`);
                    i = cmdEnd;
                    continue;
                }
                out.push(escapeHtml(`\\${cmd}`));
                i = cmdEnd;
                continue;
            }
            out.push(escapeHtml(ch));
            i++;
            continue;
        }

        if (ch === '^') {
            const arg = readMathGroup(src, i + 1, end);
            out.push(`<sup>${formatMathRange(arg.content, 0, arg.content.length, depth + 1)}</sup>`);
            i = arg.next;
            continue;
        }

        if (ch === '_') {
            const arg = readMathGroup(src, i + 1, end);
            out.push(`<sub>${formatMathRange(arg.content, 0, arg.content.length, depth + 1)}</sub>`);
            i = arg.next;
            continue;
        }

        if (ch === '{') {
            const group = readMathGroup(src, i, end);
            out.push(formatMathRange(group.content, 0, group.content.length, depth + 1));
            i = group.next;
            continue;
        }

        if (ch === '}') {
            i++;
            continue;
        }

        if (ch === '\n') {
            out.push('<br />');
            i++;
            continue;
        }

        out.push(escapeHtml(ch));
        i++;
    }

    return out.join('');
}

/**
 * XSS-safe, zero-dependency LaTeX math expression formatter.
 * Formats common symbols, fractions, radicals, superscripts, and subscripts
 * into semantic HTML while escaping all raw user text via `escapeHtml`.
 */
export function renderMathExpression(tex: string): string {
    if (!tex) return '';
    return formatMathRange(tex, 0, tex.length, 0);
}
