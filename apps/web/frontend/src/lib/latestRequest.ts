// One owner for catalog reads: old responses cannot replace a newer receipt.
// A superseded read resolves undefined, including when its transport ignores abort.
export function latestRequest<T>() {
    let active: AbortController | undefined;
    return {
        cancel() { active?.abort(); active = undefined; },
        async run(read: (signal: AbortSignal) => Promise<T>): Promise<T | undefined> {
            active?.abort();
            const controller = new AbortController();
            active = controller;
            try {
                const result = await read(controller.signal);
                return active === controller && !controller.signal.aborted ? result : undefined;
            } catch (error) {
                if (active === controller && !controller.signal.aborted) throw error;
                return undefined;
            } finally {
                if (active === controller) active = undefined;
            }
        },
    };
}
