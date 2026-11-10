import { ApiError } from './api.ts';

/** Only fixed catalog keys cross the initialization error boundary. */
export function chatInitializationError(error: unknown): string {
    if (error instanceof ApiError) return error.status === 401 || error.status === 403
        ? 'chatPage.noSession' : 'chatPage.initializationService';
    if (error instanceof Error) {
        if (error.message === 'browser_recovery_storage_unavailable') return 'chatPage.recoveryStorageUnavailable';
        if (error.message === 'invalid_host_session') return 'chatPage.noSession';
        if (error.message === 'notification_host_changed') return 'chatPage.initializationHostChanged';
        if (error.message === 'invalid_session_response' || error.message === 'chatConversations.invalidResponse'
            || error instanceof SyntaxError) return 'chatPage.initializationContract';
    }
    return 'chatPage.initializationService';
}

export function parseInitializationSession(value: unknown): { hostId: string } {
    if (!value || typeof value !== 'object') throw new Error('invalid_session_response');
    const session = value as Record<string, unknown>;
    if (session.authenticated === false) throw new Error('invalid_host_session');
    if (session.authenticated !== true || typeof session.hostId !== 'string' || !session.hostId.trim()) {
        throw new Error('invalid_session_response');
    }
    return { hostId: session.hostId };
}
