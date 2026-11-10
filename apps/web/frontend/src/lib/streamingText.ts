/** Presentation-only water reveal. Accepted text and line layout are never paced. */
export const TEXT_REVEAL_MS = 520;
export const TEXT_COMPLETION_MS = 160;
export const MAX_REVEAL_PIECES = 32;
export type RevealPiece = {
    id: number; text: string; started: number; position: number; precedingStarted?: number;
};
export type RevealView = { settled: string; pieces: RevealPiece[] };

/** Same traveling wave as the Apple renderer; later batches follow the preceding tail. */
export function waterRemaining(elapsed: number, position: number, completionElapsed?: number,
    precedingElapsed?: number): number {
    if (completionElapsed !== undefined) {
        const before = Math.max(0, elapsed - completionElapsed);
        elapsed = before + Math.max(0, TEXT_REVEAL_MS - before)
            * Math.min(1, Math.max(0, completionElapsed) / TEXT_COMPLETION_MS);
    }
    const progress = Math.min(1, Math.max(0, elapsed / TEXT_REVEAL_MS));
    const local = Math.min(1, Math.max(0, (progress * 1.34 - position) / .34));
    const value = (1 - local) ** 3;
    return precedingElapsed === undefined ? value : Math.max(value,
        waterRemaining(precedingElapsed, 1, completionElapsed));
}

const graphemes = typeof Intl.Segmenter === 'function'
    ? new Intl.Segmenter(undefined, { granularity: 'grapheme' }) : undefined;

function groups(text: string): string[] {
    // A whole delta is safer than splitting emoji or combining marks on older engines.
    if (!graphemes) return [text];
    const result: string[] = [];
    let group = '', count = 0;
    for (const { segment } of graphemes.segment(text)) {
        group += segment;
        if (++count === 3) { result.push(group); group = ''; count = 0; }
    }
    if (group) result.push(group);
    return result;
}

export class TextReveal {
    private text = '';
    private nextId = 0;
    private settled = '';
    private pieces: RevealPiece[] = [];
    private streaming = false;
    private completedAt: number | undefined;

    update(text: string, visiblePrefix: number, streaming: boolean, now: number, reduceMotion = false): RevealView {
        const finishing = this.streaming && !streaming;
        if (streaming) this.completedAt = undefined;
        else if (finishing) this.completedAt = now;
        this.advance(now);
        if (reduceMotion || !text.startsWith(this.text) || (!streaming && !finishing)) {
            // An unchanged completed stream retains only its already-running tail.
            if (reduceMotion || text !== this.text || this.completedAt === undefined) {
                this.settled = text;
                this.pieces = [];
            }
        } else if (text.length > this.text.length) {
            let known = Math.max(this.text.length, Math.min(text.length, visiblePrefix));
            const cluster = graphemes?.segment(text).containing(known);
            if (cluster && cluster.index < known) known = cluster.index + cluster.segment.length;
            if (known > this.text.length) {
                this.retire(this.pieces.length);
                this.settled += text.slice(this.text.length, known);
            }
            if (known < text.length) {
                const precedingStarted = this.pieces.at(-1)?.started;
                const additions = groups(text.slice(known));
                const offset = Math.max(0, additions.length - MAX_REVEAL_PIECES);
                // A burst never creates an unbounded animation queue.
                if (offset) {
                    this.retire(this.pieces.length);
                    this.settled += additions.slice(0, offset).join('');
                }
                additions.slice(offset).forEach((part, index) => this.pieces.push({
                    id: this.nextId++, text: part, started: now,
                    position: index / Math.max(1, additions.length - offset - 1), precedingStarted,
                }));
                this.retire(Math.max(0, this.pieces.length - MAX_REVEAL_PIECES));
            }
        }
        this.text = text;
        this.streaming = streaming;
        return this.view();
    }

    remaining(piece: RevealPiece, now: number): number {
        return waterRemaining(now - piece.started, piece.position,
            this.completedAt === undefined ? undefined : now - this.completedAt,
            piece.precedingStarted === undefined ? undefined : now - piece.precedingStarted);
    }

    advance(now: number): RevealView {
        let count = 0;
        while (count < this.pieces.length && this.remaining(this.pieces[count], now) <= 0) count++;
        this.retire(count);
        return this.view();
    }

    finish(id: number): RevealView {
        const index = this.pieces.findIndex(piece => piece.id === id);
        if (index >= 0) this.retire(index + 1);
        return this.view();
    }

    private retire(count: number) {
        for (const piece of this.pieces.splice(0, count)) this.settled += piece.text;
    }

    private view(): RevealView {
        return { settled: this.settled, pieces: [...this.pieces] };
    }
}

export const STREAMING_TEXT_SELECTION = Symbol('streaming-text-selection');

/** Preserve a reading selection when settled spans merge in one Markdown view. */
export function captureTextSelection(root: HTMLElement): (() => void) | undefined {
    const focused = root.ownerDocument.activeElement;
    if (focused?.tagName === 'INPUT' || focused?.tagName === 'TEXTAREA' || (focused as HTMLElement | null)?.isContentEditable) return;
    const selection = root.ownerDocument.getSelection();
    if (!selection || selection.isCollapsed || !selection.anchorNode || !selection.focusNode
        || !root.contains(selection.anchorNode) || !root.contains(selection.focusNode)) return;
    function offset(node: Node, position: number) {
        const range = root.ownerDocument.createRange();
        range.selectNodeContents(root);
        range.setEnd(node, position);
        return range.toString().length;
    }
    const anchor = offset(selection.anchorNode, selection.anchorOffset);
    const focus = offset(selection.focusNode, selection.focusOffset);
    return () => {
        if (!root.isConnected) return;
        const walker = root.ownerDocument.createTreeWalker(root, 4 /* SHOW_TEXT */);
        let node: Node | null, position = 0;
        let start: { node: Node; offset: number } | undefined;
        let end: { node: Node; offset: number } | undefined;
        while ((node = walker.nextNode())) {
            const length = node.textContent?.length ?? 0;
            if (!start && anchor <= position + length) start = { node, offset: anchor - position };
            if (!end && focus <= position + length) end = { node, offset: focus - position };
            if (start && end) break;
            position += length;
        }
        if (start && end) selection.setBaseAndExtent(start.node, start.offset, end.node, end.offset);
    };
}
