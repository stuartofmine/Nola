export type RetrievalDefault = { enabled: boolean };
export function retrievalDefault(value: unknown): RetrievalDefault {
    if (!value || typeof value !== 'object' || Array.isArray(value) || typeof (value as Record<string, unknown>).enabled !== 'boolean') throw new Error('Invalid retrieval default');
    return { enabled: (value as RetrievalDefault).enabled };
}
export function retrievalDefaultReceipt(value: unknown, enabled: boolean): RetrievalDefault {
    const snapshot = retrievalDefault(value);
    if (snapshot.enabled !== enabled) throw new Error('Unconfirmed retrieval default');
    return snapshot;
}
