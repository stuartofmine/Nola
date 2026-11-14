/** Actual CLI Web conversation routes. Request bodies use camelCase. */
export interface ChatConversation {
    expertContextOwner?: {roomId:string;participantId:string;agentId:string;agentRevision:number|null;isCurrent:boolean};
    expertParentConversationId?: string;
    isPinned: boolean;
    conversationId: string; title: string; createdAt: string; updatedAt: string;
    selectedModelSource: string; selectedModelId: string; executionBackend: string;
    revision: number; configurationRevision: number;
}
export interface ConversationOperationOwner { conversationId: string | null; generation: number }
export function sameConversationOperation(a: ConversationOperationOwner, b: ConversationOperationOwner): boolean {
    return a.conversationId === b.conversationId && a.generation === b.generation;
}
export function parseChatConversation(value: unknown): ChatConversation {
    if (!value || typeof value !== 'object') throw new Error('chatConversations.invalidResponse');
    const object = value as Record<string, unknown>;
    if (typeof object.isPinned !== 'boolean') throw new Error('chatConversations.invalidResponse');
    if (![object.revision, object.configurationRevision].every(value => Number.isSafeInteger(value) && Number(value) >= 1)) throw new Error('chatConversations.invalidResponse');
    if (['conversationId', 'title', 'createdAt', 'updatedAt', 'selectedModelSource', 'selectedModelId', 'executionBackend'].some(key => typeof object[key] !== 'string') || !(object.conversationId as string).trim()) throw new Error('chatConversations.invalidResponse');
    let expertContextOwner: ChatConversation['expertContextOwner'];
    if (object.expertContextOwner !== undefined) {
        const owner = object.expertContextOwner as Record<string,unknown> | null;
        if (!owner || ['roomId','participantId','agentId'].some(key=>typeof owner[key] !== 'string' || !(owner[key] as string).trim()) ||
            typeof owner.isCurrent !== 'boolean' || !(owner.agentRevision === null || Number.isSafeInteger(owner.agentRevision) && Number(owner.agentRevision)>0)) throw new Error('chatConversations.invalidResponse');
        expertContextOwner = owner as unknown as NonNullable<ChatConversation['expertContextOwner']>;
    }
    if (object.expertParentConversationId !== undefined && (typeof object.expertParentConversationId !== 'string' || !object.expertParentConversationId.trim() || object.expertParentConversationId === object.conversationId || !expertContextOwner)) throw new Error('chatConversations.invalidResponse');
    return {...(expertContextOwner ? {expertContextOwner} : {}),...(typeof object.expertParentConversationId === 'string' ? {expertParentConversationId:object.expertParentConversationId} : {}), isPinned: object.isPinned, conversationId: object.conversationId as string, title: object.title as string, createdAt: object.createdAt as string, updatedAt: object.updatedAt as string, selectedModelSource: object.selectedModelSource as string, selectedModelId: object.selectedModelId as string, executionBackend: object.executionBackend as string,
        revision: object.revision as number, configurationRevision: object.configurationRevision as number };
}
export function conversationPinRequest(configuration: unknown, expectedPinned: boolean): { expectedRevision: number; isPinned: boolean } {
    const value = configuration as { configurationRevision?: unknown; isPinned?: unknown };
    if (!Number.isSafeInteger(value?.configurationRevision) || Number(value.configurationRevision) < 1 || Number(value.configurationRevision) >= Number.MAX_SAFE_INTEGER || value.isPinned !== expectedPinned) throw new Error('chatConversations.pinChanged');
    return { expectedRevision: Number(value.configurationRevision), isPinned: !expectedPinned };
}
export function conversationPinConfirmed(value: unknown, request: { expectedRevision: number; isPinned: boolean }): boolean {
    const receipt = value as { configurationRevision?: unknown; isPinned?: unknown };
    return receipt?.configurationRevision === request.expectedRevision + 1 && receipt.isPinned === request.isPinned;
}
export function parseChatConversations(value: unknown): ChatConversation[] {
    const values = (value as { conversations?: unknown })?.conversations;
    if (!Array.isArray(values)) throw new Error('chatConversations.invalidResponse');
    const parsed = values.map(parseChatConversation);
    if (new Set(parsed.map(item => item.conversationId)).size !== parsed.length) throw new Error('chatConversations.invalidResponse');
    return parsed;
}
export function conversationUrl(id: string): string {
    if (!id.trim()) throw new Error('chatConversations.invalidTarget');
    return `/api/chat/conversations/${encodeURIComponent(id)}`;
}
export function messageWindowUrl(id: string, before: number | null = null, limit = 48, snapshot?: { projectionRevision: number; eventSequence: number }): string {
    if (!Number.isSafeInteger(limit) || limit < 1 || limit > 200 || (before !== null && !Number.isSafeInteger(before))) throw new Error('chatConversations.invalidTarget');
    if (before !== null && (!snapshot || !Number.isSafeInteger(snapshot.projectionRevision) || snapshot.projectionRevision < 0 || !Number.isSafeInteger(snapshot.eventSequence) || snapshot.eventSequence < 0)) throw new Error('chatConversations.invalidTarget');
    return `${conversationUrl(id)}/messages?limit=${limit}${before === null ? '' : `&before=${before}&expectedRevision=${snapshot!.projectionRevision}&expectedEventSequence=${snapshot!.eventSequence}`}`;
}
export function conversationTitleRequest(title: string, expectedRevision: number): { title: string; expectedRevision: number } {
    if (!title.trim() || !Number.isSafeInteger(expectedRevision) || expectedRevision < 1) throw new Error('chatConversations.invalidTitle');
    return { title: title.trim(), expectedRevision };
}
export function conversationBranchRequest(messageId: string | null): { messageId: string } {
    if (messageId !== null && !messageId.trim()) throw new Error('chatConversations.invalidTarget');
    // The Host owns the complete-conversation boundary when no message is selected.
    return { messageId: messageId ?? '' };
}
export function conversationTruncateRequest(messageId: string): { messageId: string; keepMessage: true } {
    if (!messageId.trim()) throw new Error('chatConversations.invalidTarget');
    return { messageId, keepMessage: true };
}
export interface ConversationExport { filename: string; mimeType: string; content: string }
export function parseConversationExport(value: unknown): ConversationExport {
    const payload = value as Partial<ConversationExport> | null;
    if (!payload || typeof payload.filename !== 'string' || typeof payload.mimeType !== 'string' || typeof payload.content !== 'string') throw new Error('chatConversations.invalidResponse');
    // The caller downloads opaque text; never injects returned HTML into the page.
    return { filename: payload.filename.replace(/[\/\\\u0000-\u001f]/g, '_') || 'conversation.txt', mimeType: payload.mimeType, content: payload.content };
}
/** Explicitly in-memory draft cache, not an incognito/privacy promise. An app
 * can choose its storage lifetime; this module does not touch browser storage. */
export function createConversationDraftStore(onwrite: (id: string) => void = () => {}) {
    const drafts = new Map<string, string>();
    return {
        get hasDrafts(): boolean { return drafts.size > 0; },
        read(conversationId: string): string { return drafts.get(conversationId) ?? ''; },
        write(conversationId: string, text: string): void {
            if (!conversationId.trim()) throw new Error('chatConversations.invalidTarget');
            if (text) drafts.set(conversationId, text); else drafts.delete(conversationId);
            onwrite(conversationId);
        },
        delete(conversationId: string): void { drafts.delete(conversationId); onwrite(conversationId); },
        clear(): void { drafts.clear(); },
    };
}
