export type KnowledgeField = 'knowledgePoints' | 'wiki';
export type KnowledgeDocument = Record<KnowledgeField, string>;
export interface KnowledgeSnapshot { document: KnowledgeDocument; revision: number }
export interface KnowledgeDraft {
    value: KnowledgeDocument; baseline: KnowledgeDocument; revision: number;
    // Persist an unconfirmed write across component remounts; read before retry.
    uncertain?: boolean; conflict?: boolean; remote?: KnowledgeSnapshot;
}

export function knowledgeDocument(value: unknown, conversationId: string): KnowledgeSnapshot {
    if (!value || typeof value !== 'object') throw new Error('Invalid knowledge response');
    const record = value as Record<string, unknown>;
    if (record.conversationId !== conversationId || !Number.isSafeInteger(record.configurationRevision)
        || (record.configurationRevision as number) < 1) throw new Error('Invalid knowledge owner or revision');
    const knowledge = record.knowledge;
    if (knowledge === null) return {revision: record.configurationRevision as number, document: {knowledgePoints: '', wiki: ''}};
    if (!knowledge || typeof knowledge !== 'object') throw new Error('Missing knowledge document');
    const fields = knowledge as Record<string, unknown>;
    if (typeof fields.keyPointsMarkdown !== 'string' || typeof fields.wikiMarkdown !== 'string'
        || (fields.updatedAt !== undefined && fields.updatedAt !== null
            && (typeof fields.updatedAt !== 'number' || !Number.isFinite(fields.updatedAt)))) {
        throw new Error('Invalid knowledge document');
    }
    return {revision: record.configurationRevision as number,
        document: {knowledgePoints: fields.keyPointsMarkdown, wiki: fields.wikiMarkdown}};
}
export function knowledgeDirty(draft: KnowledgeDraft): boolean {
    return draft.value.knowledgePoints !== draft.baseline.knowledgePoints || draft.value.wiki !== draft.baseline.wiki;
}
export function knowledgePending(draft: KnowledgeDraft | undefined): boolean {
    return !!draft && (knowledgeDirty(draft) || !!draft.uncertain || !!draft.conflict);
}
export function knowledgePatch(draft: KnowledgeDraft) {
    const knowledge: Record<string, string> = {};
    for (const [field, wire] of [['knowledgePoints', 'keyPointsMarkdown'], ['wiki', 'wikiMarkdown']] as const) {
        if (draft.value[field] !== draft.baseline[field]) knowledge[wire] = draft.value[field];
    }
    return {expectedRevision: draft.revision, knowledge};
}
export function reconcileKnowledge(draft: KnowledgeDraft | undefined, remote: KnowledgeSnapshot): KnowledgeDraft {
    if (draft && remote.revision < draft.revision) throw new Error('Stale knowledge snapshot');
    if (!draft || !knowledgeDirty(draft)) return {value: {...remote.document}, baseline: {...remote.document}, revision: remote.revision};
    const value = {...draft.value};
    let conflict = false;
    for (const field of ['knowledgePoints', 'wiki'] as const) {
        if (value[field] === draft.baseline[field]) value[field] = remote.document[field];
        else if (remote.document[field] !== draft.baseline[field] && remote.document[field] !== value[field]) conflict = true;
    }
    // Keep the original base on conflict: Reload must not silently authorize
    // overwriting somebody else's edit on the next attempt.
    if (conflict) return {...draft, uncertain: false, conflict: true, remote};
    return {value, baseline: {...remote.document}, revision: remote.revision};
}
export function resolveKnowledge(draft: KnowledgeDraft, choice: 'remote' | 'local'): KnowledgeDraft {
    if (!draft.conflict || !draft.remote) throw new Error('No knowledge conflict');
    const resolved = reconcileKnowledge(undefined, draft.remote);
    if (choice === 'local') {
        for (const field of ['knowledgePoints', 'wiki'] as const) {
            if (draft.value[field] !== draft.baseline[field]) resolved.value[field] = draft.value[field];
        }
    }
    return resolved;
}
export function knowledgeUrl(conversationId: string): string {
    if (!conversationId.trim()) throw new Error('Missing conversation');
    return `/api/chat/conversations/${encodeURIComponent(conversationId)}/configuration`;
}
