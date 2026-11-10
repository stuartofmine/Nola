import { parseChatConversation, parseChatConversations, conversationUrl, type ChatConversation } from './chatConversations.ts';
import { sessionModelSelectionConfirmed, type SessionConfiguration, type SessionModelChoiceState } from './chatSessionSettings.ts';

// Use the configuration snapshot shared by permission/parameter saves, never the
// older conversation-list revision. A lost acknowledgement is not retried here.
export async function commitSessionModel(input: {
    conversation: ChatConversation;
    snapshot: SessionConfiguration | null;
    state: SessionModelChoiceState;
    choice: string;
    request: (url: string, options: { method: string; body: string }) => Promise<unknown>;
    isCurrent: () => boolean;
}): Promise<ChatConversation | null> {
    const { conversation, snapshot, state, choice, request, isCurrent } = input;
    if (!isCurrent() || state.disabled || !snapshot || choice === state.choice && (choice !== 'auto' || !!conversation.selectedModelId || !state.options.length)) return null;
    const option = state.options.find(option => option.value === choice);
    if (choice !== 'auto' && !option) throw new Error('chatSession.invalidModel');
    const requested: { selectedModelSource: 'auto' | 'local' | 'cloud'; selectedModelId?: string } = choice === 'auto'
        ? { selectedModelSource: 'auto' }
        : { selectedModelSource: option!.local ? 'local' : 'cloud', selectedModelId: choice.slice(6) };
    const revision = snapshot.configurationRevision;
    if (!Number.isSafeInteger(revision) || revision < conversation.configurationRevision) throw new Error('chatSession.invalidResponse');
    const value = await request(conversationUrl(conversation.conversationId), {
        method: 'PATCH', body: JSON.stringify({ ...requested, expectedConfigurationRevision: revision }),
    });
    if (!isCurrent()) return null;
    const result = parseChatConversation(value);
    // Core resolves default inheritance to an exact saved model when available.
    // Accept that canonical receipt rather than insisting it echo the UI's auto sentinel.
    const selectionConfirmed = requested.selectedModelSource === 'auto'
        && ['cloud', 'local'].includes(result.selectedModelSource) && !!result.selectedModelId
        || sessionModelSelectionConfirmed(result, requested);
    if (result.conversationId !== conversation.conversationId || result.configurationRevision <= revision
        || !selectionConfirmed) throw new Error('chatSession.invalidResponse');
    return result;
}

export async function recoverSessionModel(conversationId: string, request: (url: string) => Promise<unknown>, isCurrent: () => boolean): Promise<ChatConversation | null> {
    const value = await request('/api/chat/conversations');
    if (!isCurrent()) return null;
    const conversation = parseChatConversations(value).find(item => item.conversationId === conversationId);
    if (!conversation) throw new Error('chatSession.invalidResponse');
    return conversation;
}
