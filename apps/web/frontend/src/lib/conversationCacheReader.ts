import { parseConversationCacheUsage } from './conversationCacheUsage.ts';
import type { ConversationCacheUsage } from './conversationCacheUsage.ts';

/** Committed usage boundaries only; never aggregate browser token events.
 * Reads coalesce, and epoch fencing protects even transports ignoring abort. */
export class ConversationCacheReader {
    private epoch = 0;
    private selected: string | null = null;
    private controller: AbortController | null = null;
    private pending = false;
    private readonly query: (conversationId: string, signal: AbortSignal) => Promise<unknown>;
    private readonly publish: (usage: ConversationCacheUsage, failed: boolean) => void;

    constructor(
        query: (conversationId: string, signal: AbortSignal) => Promise<unknown>,
        publish: (usage: ConversationCacheUsage, failed: boolean) => void,
    ) {
        this.query = query;
        this.publish = publish;
    }

    select(conversationId: string | null): void {
        this.epoch++;
        this.controller?.abort();
        this.controller = null;
        this.pending = false;
        this.selected = conversationId;
        this.publish(null, false);
        this.refresh();
    }

    refresh(): void {
        if (this.selected === null) return;
        if (this.controller) { this.pending = true; return; }
        const id = this.selected;
        const epoch = this.epoch;
        const controller = new AbortController();
        this.controller = controller;
        const current = () => this.epoch === epoch && !controller.signal.aborted;
        void (async () => {
            try {
                const raw = await this.query(id, controller.signal);
                if (current()) this.publish(parseConversationCacheUsage(raw, id), false);
            } catch {
                // Do not present an outdated successful value as current.
                if (current()) this.publish(null, true);
            } finally {
                if (current()) {
                    this.controller = null;
                    if (this.pending) { this.pending = false; this.refresh(); }
                }
            }
        })();
    }

    dispose(): void {
        this.epoch++;
        this.controller?.abort();
        this.controller = null;
        this.selected = null;
        this.pending = false;
    }
}
