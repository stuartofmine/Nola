export type ConfirmationRequest = {
    title: string;
    description: string;
    confirmLabel: string;
    cancelLabel?: string;
    destructive?: boolean;
};

export type ConfirmationHandle = { ask(request: ConfirmationRequest): Promise<boolean> };

/** One explicit decision at a time. Overlapping requests never inherit consent. */
export class ConfirmationGate {
    private resolve?: (accepted: boolean) => void;
    get pending() { return this.resolve !== undefined; }

    request(): Promise<boolean> | undefined {
        if (this.pending) return undefined;
        return new Promise(resolve => { this.resolve = resolve; });
    }

    settle(accepted: boolean) {
        const resolve = this.resolve;
        this.resolve = undefined;
        resolve?.(accepted);
    }
}
