import { readBrowserClientId } from './browserClientIdentity.ts';
import { get } from 'svelte/store';
import { api } from './api.ts';
import { locale, t } from './i18n.ts';
import { browserNotifications } from './browserNotifications.ts';
import { createWorkspaceCompletions } from './workspaceCompletions.ts';
import { SourceSubmissionStorage } from './sourceSubmission.ts';
import type { PendingSourceSubmission } from './sourceSubmission.ts';
import { createChatTurn, reduceChatTurn } from './chatTurn.ts';
import type { WorkspaceDrafts } from './workspaceDrafts.ts';
import { createSourceSubmissionRecovery } from './sourceSubmissionRecovery.ts';

/** Restore locally retained submissions after re-authenticating the Host.
 * Local records are candidates; the observer still verifies the server seal.
 * Unprepared/uncertain submissions are never prepared or dispatched here. */
export async function restoreBrowserWorkspaceCompletions(workspace: WorkspaceDrafts, signal: AbortSignal): Promise<void> {
    try {
        const session = await api<{ authenticated?: boolean; hostId?: string }>('/api/web/session', { signal });
        if (signal.aborted || session.authenticated !== true || typeof session.hostId !== 'string' || !session.hostId.trim()
            || workspace.authenticatedHostId !== null && workspace.authenticatedHostId !== session.hostId) return;
        const clientId = readBrowserClientId(session.hostId);
        if (!clientId) return;
        const submissions = new SourceSubmissionStorage(localStorage).recoverable(session.hostId, clientId);
        if (signal.aborted) return;
        const host = workspace.forHost(session.hostId);
        for (const pending of submissions) void host.sources.watch(pending);
    } catch { /* No storage/session authority: leave source records untouched. */ }
}

export function createBrowserSourceRecovery(completions: ReturnType<typeof createWorkspaceCompletions>,
    admitted?: (pending: PendingSourceSubmission) => void | Promise<void>) {
    return createSourceSubmissionRecovery({
        storage: () => new SourceSubmissionStorage(localStorage),
        observe: (body, signal) => api('/api/chat/agent/v2/submission', { method: 'POST', body: JSON.stringify(body), signal }),
        admitted: pending => {
            void Promise.resolve(admitted?.(pending)).catch(() => {});
            const id = pending.identity;
            const owner = { hostId: id.source_host_id, clientId: id.client_id, conversationId: id.conversation_id, generation: 0 };
            const initial = createChatTurn(owner, { submissionId: id.submission_id,
                userMessageId: id.user_message_id, assistantMessageId: id.assistant_message_id });
            void completions.watch(reduceChatTurn(initial, { type: 'run_bound', owner,
                runId: pending.receipt!.admission!.identity.run_id }).state);
        },
    });
}

export function createBrowserWorkspaceCompletions() {
    return createWorkspaceCompletions({
        journal: (body, signal) => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(body), signal }),
        completed: evidence => {
            const language = get(locale);
            browserNotifications.completed(evidence, evidence.owner,
                t(language, 'settings.notifications.completedTitle'), t(language, 'settings.notifications.completedBody'));
        },
    });
}
