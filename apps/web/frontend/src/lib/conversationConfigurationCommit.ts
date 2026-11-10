export class ConversationConfigurationConflict extends Error {
    readonly expectedRevision: number;
    readonly actualRevision: number;
    constructor(expectedRevision: number, actualRevision: number) {
        super('Conversation configuration changed; the local edit is retained');
        this.expectedRevision = expectedRevision;
        this.actualRevision = actualRevision;
    }
}

/** The PATCH result is a command outcome; GET remains a snapshot query. */
export function committedConversationConfiguration(value: unknown): Record<string, unknown> {
    if (!value || typeof value !== 'object') throw new Error('Invalid configuration commit outcome');
    const result = value as Record<string, unknown>;
    if (result.status === 'conflict' && Number.isSafeInteger(result.expected_revision) && Number.isSafeInteger(result.actual_revision))
        throw new ConversationConfigurationConflict(Number(result.expected_revision), Number(result.actual_revision));
    if (result.status === 'not_found' && typeof result.conversation_id === 'string') throw new Error('Conversation is unavailable');
    if (result.status !== 'committed' || !result.snapshot || typeof result.snapshot !== 'object' || Array.isArray(result.snapshot))
        throw new Error('Invalid configuration commit outcome');
    return result.snapshot as Record<string, unknown>;
}
