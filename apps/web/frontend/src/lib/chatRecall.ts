export interface RecallHit { message_id: string; part_id: string; role: string; message_ordinal: number; snippet: string; start_character: number }
export function recallUrl(conversationId: string, query: string): string {
    if (!conversationId.trim() || !query.trim() || Array.from(query.trim()).length > 500) throw new Error('Invalid recall query');
    return `/api/chat/conversations/${encodeURIComponent(conversationId)}/recall?q=${encodeURIComponent(query.trim())}`;
}
export function recallHits(value: unknown, conversationId: string, query: string): RecallHit[] {
    if (!value || typeof value !== 'object') throw new Error('Invalid recall response');
    const response = value as Record<string, unknown>;
    if (response.conversationId !== conversationId || response.query !== query.trim() || response.limit !== 20 || !Array.isArray(response.hits) || response.hits.length > 20) throw new Error('Invalid recall owner or bounds');
    const keys = new Set<string>();
    return response.hits.map(value => {
        if (!value || typeof value !== 'object') throw new Error('Invalid recall hit');
        const hit = value as RecallHit;
        if (typeof hit.message_id !== 'string' || !hit.message_id.trim() || typeof hit.part_id !== 'string' || !hit.part_id.trim()
            || typeof hit.role !== 'string' || typeof hit.snippet !== 'string'
            || !Number.isSafeInteger(hit.message_ordinal) || hit.message_ordinal < 0
            || !Number.isSafeInteger(hit.start_character) || hit.start_character < 0) throw new Error('Invalid recall hit');
        const key = JSON.stringify([hit.message_id, hit.part_id, hit.start_character]);
        if (keys.has(key)) throw new Error('Duplicate recall hit'); keys.add(key);
        return hit;
    });
}
