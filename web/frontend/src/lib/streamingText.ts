/** Presentation-only state: text is always complete; animation never queues it. */
export const TEXT_REVEAL_MS = 640;
export const MAX_REVEAL_PIECES = 32;
export type RevealPiece = { id: number; text: string; started: number };
export type RevealView = { settled: string; pieces: RevealPiece[] };

export class TextReveal {
    private text = '';
    private nextId = 0;
    private settled = '';
    private pieces: RevealPiece[] = [];

    update(text: string, visiblePrefix: number, streaming: boolean, now: number): RevealView {
        if (!streaming || !text.startsWith(this.text)) {
            this.settled = text;
            this.pieces = [];
        } else if (text.length > this.text.length) {
            const known = Math.max(this.text.length, Math.min(text.length, visiblePrefix));
            if (known > this.text.length) {
                this.retire(this.pieces.length);
                this.settled += text.slice(this.text.length, known);
            }
            if (known < text.length) {
                this.pieces.push({ id: this.nextId++, text: text.slice(known), started: now });
            }
            let count = 0;
            while (count < this.pieces.length && now - this.pieces[count].started >= TEXT_REVEAL_MS) count++;
            this.retire(Math.max(count, this.pieces.length - MAX_REVEAL_PIECES));
        }
        this.text = text;
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
