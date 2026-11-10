/** Viewport intent is owned by user actions, never inferred from content/layout changes. */
export interface ChatScrollMetrics {
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
}

export interface ChatScrollToken {
    readonly conversationId: string;
    readonly generation: number;
}

/** contentTop is the stable message anchor's position in scroll-content coordinates. */
export interface ChatScrollAnchor { id: string; contentTop: number }
export interface ChatReadingPosition {
    followsLatest: boolean;
    scrollTop: number;
    anchorId: string | null;
    viewportOffset: number;
}
export interface ChatHistoryAnchor {
    readonly token: ChatScrollToken;
    readonly id: string;
    readonly viewportOffset: number;
}

function validMetrics(value: ChatScrollMetrics): boolean {
    return Number.isFinite(value.scrollTop) && Number.isFinite(value.scrollHeight)
        && Number.isFinite(value.clientHeight) && value.scrollHeight >= 0 && value.clientHeight >= 0;
}

function bottom(value: ChatScrollMetrics): number {
    return Math.max(0, value.scrollHeight - value.clientHeight);
}

export function chatNearBottom(value: ChatScrollMetrics, threshold = 24): boolean {
    return validMetrics(value) && Number.isFinite(threshold) && threshold >= 0
        && bottom(value) - Math.max(0, value.scrollTop) <= threshold;
}

/**
 * No DOM reads or writes. Capture token BEFORE awaiting tick/layout and check it again
 * immediately before applying a returned scrollTop. DOM scroll events alone do not
 * prove user intent: classify wheel/touch/keyboard/scrollbar gestures in the adapter.
 * Programmatic scrolling and resize observations must use source='programmatic'.
 */
export class ChatScrollOwnership {
    private conversationId = '';
    private generation = 0;
    private intent: 'latest' | 'reading' = 'latest';
    private interacting = false;
    private observedGesture = false;
    private historyAnchor: ChatHistoryAnchor | null = null;
    private readonly threshold: number;

    constructor(threshold = 24) {
        if (!Number.isFinite(threshold) || threshold < 0) throw new Error('Invalid scroll threshold');
        this.threshold = threshold;
    }

    get followsLatest(): boolean { return this.intent === 'latest'; }
    get token(): ChatScrollToken {
        return { conversationId: this.conversationId, generation: this.generation };
    }
    accepts(token: ChatScrollToken): boolean {
        return token.conversationId === this.conversationId && token.generation === this.generation;
    }
    /** Call on conversation activation/remount; even reopening the same ID invalidates old work. */
    reset(conversationId: string): ChatScrollToken {
        this.conversationId = conversationId;
        this.invalidate();
        this.intent = 'latest';
        this.interacting = false;
        this.observedGesture = false;
        return this.token;
    }
    private invalidate(): void {
        this.generation += 1;
        this.historyAnchor = null;
    }
    beginUserScroll(token: ChatScrollToken): boolean {
        if (!this.accepts(token)) return false;
        this.invalidate();
        this.interacting = true;
        this.observedGesture = false;
        this.intent = 'reading';
        return true;
    }
    observe(token: ChatScrollToken, metrics: ChatScrollMetrics, source: 'user' | 'programmatic'): void {
        if (!this.accepts(token) || !validMetrics(metrics) || source !== 'user') return;
        // Also accepts discrete user scrolls without a gesture lifecycle (e.g. keyboard).
        this.invalidate();
        if (this.interacting) this.observedGesture = true;
        this.intent = chatNearBottom(metrics, this.threshold) ? 'latest' : 'reading';
    }
    endUserScroll(token: ChatScrollToken, metrics: ChatScrollMetrics): void {
        if (!this.accepts(token) || !this.interacting) return;
        // Layout may have changed since the last genuine scroll event. Ending
        // the gesture is not a new user decision about that changed geometry.
        if (!this.observedGesture) this.observe(token, metrics, 'user');
        this.interacting = false;
    }
    returnToLatest(token: ChatScrollToken): ChatScrollToken | null {
        if (!this.accepts(token)) return null;
        this.invalidate();
        this.intent = 'latest';
        this.interacting = false;
        return this.token;
    }
    contentChanged(token: ChatScrollToken, metrics: ChatScrollMetrics): number | null {
        if (!this.accepts(token) || this.interacting || !this.followsLatest || !validMetrics(metrics)) return null;
        return bottom(metrics);
    }
    /** Restore an explicitly saved UI intent, never infer intent from new layout. */
    restoreIntent(token: ChatScrollToken, position: ChatReadingPosition): ChatScrollToken | null {
        if (!this.accepts(token) || !Number.isFinite(position.scrollTop) || !Number.isFinite(position.viewportOffset)) return null;
        this.invalidate(); this.interacting = false; this.observedGesture = false;
        this.intent = position.followsLatest ? 'latest' : 'reading';
        return this.token;
    }
    restoredPosition(token: ChatScrollToken, position: ChatReadingPosition, metrics: ChatScrollMetrics, anchor: ChatScrollAnchor | null): number | null {
        if (!this.accepts(token) || this.interacting || !validMetrics(metrics)) return null;
        if (position.followsLatest) return bottom(metrics);
        const top = anchor && anchor.id === position.anchorId && Number.isFinite(anchor.contentTop)
            ? anchor.contentTop - position.viewportOffset : position.scrollTop;
        return Number.isFinite(top) ? Math.max(0, Math.min(bottom(metrics), top)) : null;
    }
    /** Capture immediately BEFORE inserting older rows, not before the network fetch. */
    beginHistoryPrepend(token: ChatScrollToken, metrics: ChatScrollMetrics, anchor: ChatScrollAnchor): ChatHistoryAnchor | null {
        if (!this.accepts(token) || this.interacting || !validMetrics(metrics)
            || !anchor.id || !Number.isFinite(anchor.contentTop)) return null;
        this.invalidate();
        this.intent = 'reading';
        this.historyAnchor = { token: this.token, id: anchor.id, viewportOffset: anchor.contentTop - metrics.scrollTop };
        return this.historyAnchor;
    }
    /** Same stable anchor after layout: height growth below it must not affect the result. */
    restoreHistoryAnchor(capture: ChatHistoryAnchor, metrics: ChatScrollMetrics, anchor: ChatScrollAnchor | null): number | null {
        if (capture !== this.historyAnchor || !this.accepts(capture.token) || this.interacting) return null;
        this.historyAnchor = null;
        if (!validMetrics(metrics) || !anchor || anchor.id !== capture.id || !Number.isFinite(anchor.contentTop)) return null;
        return Math.max(0, Math.min(bottom(metrics), anchor.contentTop - capture.viewportOffset));
    }
}
