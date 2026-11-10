export type ConversationSummary = {
    conversationId: string;
    title: string;
    updatedAt: string;
};

export function conversationHref(id: string): string {
    return `/chat?conversation=${encodeURIComponent(id)}`;
}

export function filteredConversations(values: ConversationSummary[], query: string): ConversationSummary[] {
    const normalized = query.trim().toLocaleLowerCase();
    return values.filter(value => !normalized || value.title.toLocaleLowerCase().includes(normalized))
        .sort((a, b) => (Date.parse(b.updatedAt) || 0) - (Date.parse(a.updatedAt) || 0) || a.conversationId.localeCompare(b.conversationId));
}
