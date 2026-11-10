import { marked } from 'marked';
import type { Token, Tokens } from 'marked';
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
    try { return blocks(marked.lexer(source, { gfm: true, breaks: false })); }
    catch { return [{ kind: 'paragraph', children: [{ kind: 'text', text: source }] }]; }
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
