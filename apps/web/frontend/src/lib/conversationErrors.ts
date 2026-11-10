import { ApiError } from './api.ts';

/** Display keys only; never retain sensitive response bodies as UI errors. */
export function conversationErrorKey(cause: unknown, operation: 'load' | 'create' | 'write'): string {
    if (cause instanceof ApiError && [401, 403].includes(cause.status)) return 'shell.conversations.auth_error';
    if (operation === 'write') return 'chatConversations.writeFailed';
    return operation === 'create' ? 'shell.conversations.create_error' : 'shell.conversations.load_error';
}
