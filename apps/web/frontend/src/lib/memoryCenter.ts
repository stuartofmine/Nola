export interface MemoryRecord {
    id: string; scope: string; kind: string; content: string; summary: string | null;
    tags: string[]; source: string; confidence: number; supersedes: string[];
    importance: number; sensitivity: string | null; expires_at: string | null; created_at: string; updated_at: string;
}
export function memoryDeleteUrl(id: string): string {
    if (!id.trim()) throw new Error('Missing memory identity');
    return `/api/memory/${encodeURIComponent(id)}`;
}
export function memoryDeletionConfirmed(value: unknown, id: string): boolean {
    if (!value || typeof value !== 'object') return false;
    const receipt = value as Record<string, unknown>;
    return receipt.type === 'memory_deleted' && receipt.id === id;
}
export function memoryRecords(value: unknown): MemoryRecord[] {
    if (!value || typeof value !== 'object') throw new Error('Invalid memory response');
    const response = value as Record<string, unknown>;
    if (response.type !== 'memory_records' || response.limit !== 2000 || !Array.isArray(response.records) || response.records.length > 2000) throw new Error('Invalid memory list');
    const ids = new Set<string>();
    return response.records.map(value => {
        if (!value || typeof value !== 'object') throw new Error('Invalid memory record');
        const record = value as MemoryRecord;
        if (!['id', 'scope', 'kind', 'content', 'source', 'created_at', 'updated_at'].every(key => typeof (value as Record<string, unknown>)[key] === 'string')
            || !record.id.trim() || ids.has(record.id) || (record.summary !== null && typeof record.summary !== 'string')
            || !Number.isFinite(record.confidence) || record.confidence < 0 || record.confidence > 1
            || !Number.isFinite(record.importance) || record.importance < 0 || record.importance > 1
            || (record.sensitivity !== null && typeof record.sensitivity !== 'string') || (record.expires_at !== null && typeof record.expires_at !== 'string')
            || !Array.isArray(record.tags) || !record.tags.every(item => typeof item === 'string')
            || !Array.isArray(record.supersedes) || !record.supersedes.every(item => typeof item === 'string' && item.trim())) throw new Error('Invalid memory record');
        ids.add(record.id); return record;
    });
}
export function memoryDraft(record?: MemoryRecord) {
    return {scope: record?.scope ?? 'global', kind: record?.kind ?? 'preference', content: record?.content ?? '', summary: record?.summary ?? '',
        tags: [...(record?.tags ?? [])], confidence: record?.confidence ?? 1, importance: record?.importance ?? 0.5,
        sensitivity: record?.sensitivity ?? null, expires_at: record?.expires_at ?? null};
}
export function memoryWriteReceipt(value: unknown, draft: ReturnType<typeof memoryDraft>, previousId?: string): MemoryRecord {
    if (!value || typeof value !== 'object') throw new Error('Invalid memory receipt');
    const receipt = value as Record<string, unknown>;
    if (receipt.type !== (previousId ? 'memory_revised' : 'memory_created') || (previousId && receipt.previous_id !== previousId)) throw new Error('Wrong memory operation');
    const [record] = memoryRecords({type: 'memory_records', limit: 2000, records: [receipt.record]});
    if (record.content !== draft.content.trim() || record.scope !== draft.scope || record.kind !== draft.kind
        || (previousId && (record.id === previousId || !record.supersedes.includes(previousId)))) throw new Error('Unconfirmed memory write');
    return record;
}
export function memoryView(records: readonly MemoryRecord[], query: string, locale: string) {
    // Filtering must not make a superseded revision appear current.
    const superseded = new Set(records.flatMap(record => record.supersedes));
    const needle = query.trim().toLocaleLowerCase(locale);
    return records.filter(record => !needle || [record.content, record.summary ?? '', record.scope, record.kind, record.source, ...record.tags]
        .some(value => value.toLocaleLowerCase(locale).includes(needle)))
        .map(record => ({record, superseded: superseded.has(record.id)}));
}
