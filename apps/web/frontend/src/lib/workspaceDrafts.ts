import type { ChatReadingPosition } from './chatScroll';
import type { ConversationContentTabs } from './conversationContentTabs';
import { createConversationDraftStore } from './chatConversations.ts';
import { createTextAttachmentDraftStore } from './chatTextAttachments.ts';
import { knowledgePending, type KnowledgeDraft } from './chatKnowledge.ts';
import { SvelteMap } from 'svelte/reactivity';
import { createBrowserWorkspaceCompletions, createBrowserSourceRecovery } from './browserWorkspaceCompletions.ts';
import { IndexedDBConversationDraftRepository, type ConversationDraftRepository, type ConversationDraftSubmissionTicket, type ConversationDraftRecord } from './conversationDraftRepository.ts';
import type { PendingSourceSubmission } from './sourceSubmission.ts';
import type { ConversationSubmissionController } from './conversationSubmissionController.ts';
import type { RoomDraft } from './agentRooms.ts';

export const workspaceDraftsContext = Symbol('workspace-drafts');

/** Memory projections are replaced on Host change; durable drafts remain scoped
 * to their original authenticated Host, never copied to the replacement. */
export function createWorkspaceDrafts(repository: ConversationDraftRepository | null = typeof indexedDB === 'undefined' ? null : new IndexedDBConversationDraftRepository()) {
    const create = (ownerHostId: string) => {
        const expertSelections = new Map<string, readonly string[]>();
        const expertSelectionEpochs = new Map<string,string>();
        const contentTabs = new SvelteMap<string | null, ConversationContentTabs>();
        const readingPositions = new Map<string, ChatReadingPosition>();
        const completions = createBrowserWorkspaceCompletions();
        const controllers = new Map<string, ConversationSubmissionController>();
        const loaded = new Set<string>(), deleted = new Set<string>();
        const touched = new Map<string, Set<string>>(), loading = new Map<string, Promise<void>>();
        const editRevisions = new Map<string, number>();
        const tickets = new Map<string, {ticket: ConversationDraftSubmissionTicket; editRevision: number; draft?:ConversationDraftRecord}>();
        // Expert parent admission is not an ordinary source Run submission.
        // Keep only the original local text edit witness, never a synthetic receipt.
        const expertDrafts = new Map<string, {conversationId: string; editRevision: number; text: string}>();
        const persistenceErrors = new SvelteMap<string, boolean>();
        let hydrating = false, invalidated = false, queue = Promise.resolve();
        const enqueue = (id: string, operation: () => Promise<void>) => {
            queue = queue.then(operation).then(() => { persistenceErrors.delete(id); }, () => { persistenceErrors.set(id, true); });
        };
        const persist = (id: string) => {
            if (!repository || !ownerHostId || invalidated || deleted.has(id)) return;
            const snapshot = {hostId: ownerHostId, conversationId: id, text: text.read(id), objective: objectives.read(id), attachments: attachments.read(id)};
            enqueue(id, () => repository.write(snapshot));
        };
        const changed = (id: string, field: string) => {
            if (hydrating || invalidated || deleted.has(id)) return;
            const fields = touched.get(id) ?? new Set<string>(); fields.add(field); touched.set(id, fields);
            editRevisions.set(id, (editRevisions.get(id) ?? 0) + 1);
            if (!repository || !ownerHostId) return;
            if (!loaded.has(id)) { void load(id); return; }
            persist(id);
        };
        const text = createConversationDraftStore(id => changed(id, 'text'));
        const objectives = createConversationDraftStore(id => changed(id, 'objective'));
        const attachments = createTextAttachmentDraftStore(id => changed(id, 'attachments'));
        function load(id: string): Promise<void> {
            const pending = loading.get(id);
            if (pending) return pending;
            const operation = hydrate(id).finally(() => { loading.delete(id); });
            loading.set(id, operation);
            return operation;
        }
        async function hydrate(id: string) {
            if (!repository || loaded.has(id) || deleted.has(id) || invalidated) return;
            try {
                await queue;
                const record = await repository.read(ownerHostId, id);
                if (invalidated || deleted.has(id)) return;
                if (record) {
                    hydrating = true;
                    try {
                        const fields = touched.get(id);
                        if (!fields?.has('text')) text.write(id, record.text);
                        if (!fields?.has('objective')) objectives.write(id, record.objective);
                        if (!fields?.has('attachments')) attachments.write(id, record.attachments);
                    }
                    finally { hydrating = false; }
                }
                loaded.add(id);
                if (touched.has(id)) persist(id);
                else persistenceErrors.delete(id);
            } catch { persistenceErrors.set(id, true); }
        }
        return { text, objectives, attachments, persistenceErrors, load,
        controllers,
        captureExpertAdmission(id: string, submissionId: string, submittedText: string) {
            if (invalidated || deleted.has(id) || !ownerHostId || !submissionId.trim()
                || repository && !loaded.has(id) || text.read(id) !== submittedText) throw new Error('Expert draft owner unavailable');
            const previous = expertDrafts.get(submissionId);
            if (previous) {
                if (previous.conversationId !== id || previous.text !== submittedText) throw new Error('Expert draft intent changed');
                return; // A retry must never recapture a later edit revision.
            }
            expertDrafts.set(submissionId, {conversationId: id, editRevision: editRevisions.get(id) ?? 0, text: submittedText});
        },
        async acknowledgeExpertAdmission(id: string, submissionId: string): Promise<boolean> {
            const captured = expertDrafts.get(submissionId);
            if (captured && captured.conversationId !== id) throw new Error('Expert draft owner changed');
            let cleared = false;
            const operation = queue.then(async () => {
                if (invalidated || deleted.has(id) || !ownerHostId || repository && !loaded.has(id)) throw new Error('Expert draft owner unavailable');
                const unchanged = !!captured && (editRevisions.get(id) ?? 0) === captured.editRevision;
                // The existing repository's write performs its per-key revision
                // CAS. Do not retire the intent until this durable write succeeds.
                if (repository) await repository.write({hostId: ownerHostId, conversationId: id,
                    text: unchanged ? '' : text.read(id), objective: objectives.read(id), attachments: attachments.read(id)});
                if (unchanged && !invalidated && !deleted.has(id) && (editRevisions.get(id) ?? 0) === captured!.editRevision) {
                    hydrating = true;
                    try { text.write(id, ''); } finally { hydrating = false; }
                    cleared = true;
                }
            });
            // Edits made while the clear is awaiting storage append after this
            // operation through the same queue, so their newer text wins on disk.
            queue = operation.then(() => { persistenceErrors.delete(id); }, () => { persistenceErrors.set(id, true); });
            await operation;
            let durable: Promise<void>;
            do { durable = queue; await durable; } while (durable !== queue);
            if (invalidated || deleted.has(id) || persistenceErrors.has(id)) throw new Error('Expert draft persistence is unconfirmed');
            expertDrafts.delete(submissionId);
            return cleared;
        },
        async stageSubmission(pending: PendingSourceSubmission) {
            const id = pending.identity.conversation_id;
            const editRevision = editRevisions.get(id) ?? 0;
            const record = {hostId: ownerHostId, conversationId: id, text: text.read(id), objective: objectives.read(id), attachments: attachments.read(id)};
            const operation = queue.then(async () => {
                if (invalidated || deleted.has(id) || !loaded.has(id) && repository) throw new Error('Draft owner unavailable');
                if (repository && !repository.stageSubmission) throw new Error('Draft repository cannot persist submissions');
                const ticket = repository ? await repository.stageSubmission!(record, pending) : {
                    hostId: ownerHostId, clientId: pending.identity.client_id, conversationId: id,
                    submissionId: pending.identity.submission_id, draftRevision: editRevision, pending, committed: false};
                tickets.set(ticket.submissionId, {ticket, editRevision, draft:structuredClone(record)});
                return ticket;
            });
            queue = operation.then(() => {}, () => { persistenceErrors.set(id, true); });
            return operation;
        },
        async updateSubmission(pending: PendingSourceSubmission) {
            let entry = tickets.get(pending.identity.submission_id);
            if (!entry) {
                // Admission consumes the draft, not the source receipt. A cold
                // controller can still be reconciling that exact submitted Run.
                const ticket = await repository?.readSubmission?.(ownerHostId, pending.identity.client_id, pending.identity.submission_id);
                if (!ticket || invalidated || deleted.has(ticket.conversationId)
                    || ticket.hostId !== ownerHostId || ticket.clientId !== pending.identity.client_id
                    || ticket.conversationId !== pending.identity.conversation_id || ticket.submissionId !== pending.identity.submission_id
                    || JSON.stringify(ticket.pending.identity) !== JSON.stringify(pending.identity)
                    || JSON.stringify(ticket.pending.request) !== JSON.stringify(pending.request)) throw new Error('Missing or changed durable draft submission');
                entry = {ticket, editRevision: editRevisions.get(ticket.conversationId) ?? 0};
                tickets.set(ticket.submissionId, entry);
            }
            await repository?.updateSubmission?.(entry.ticket, pending);
            entry.ticket = {...entry.ticket, pending};
        },
        async retireSubmission(submissionId: string) {
            const entry = tickets.get(submissionId);
            if (!entry) return;
            const id = entry.ticket.conversationId;
            const operation = queue.then(async () => {
                const retired = repository ? await repository.retireSubmission!(entry.ticket) : true;
                if (retired && !invalidated && !deleted.has(id) && (editRevisions.get(id) ?? 0) === entry.editRevision) {
                    hydrating = true;
                    try { text.write(id, ''); attachments.delete(id); } finally { hydrating = false; }
                }
                entry.ticket = {...entry.ticket, committed: true};
            });
            queue = operation.catch(() => { persistenceErrors.set(id, true); });
            await operation;
        },
        restoreBlockedSubmission(submissionId:string):boolean {
            const entry=tickets.get(submissionId),id=entry?.ticket.conversationId;
            if(!entry?.draft||!id||invalidated||deleted.has(id)||(editRevisions.get(id)??0)!==entry.editRevision)return false;
            const draft=entry.draft;entry.draft=undefined;
            text.write(id,draft.text);attachments.write(id,draft.attachments);
            return true;
        },
        async restoreSubmissions(clientId: string) {
            const restored = await repository?.pendingSubmissions?.(ownerHostId, clientId) ?? [];
            for (const ticket of restored) tickets.set(ticket.submissionId, {ticket, editRevision: editRevisions.get(ticket.conversationId) ?? 0});
            return restored.map(ticket => ticket.pending);
        },
        async abandonSubmission(submissionId: string) {
            const entry = tickets.get(submissionId);
            if (!entry) return;
            await repository?.retireSubmission?.(entry.ticket, false);
            tickets.delete(submissionId);
        },
        forget(id: string) {
            deleted.add(id); loaded.delete(id); touched.delete(id);
            for (const [submissionId, ticket] of expertDrafts) if (ticket.conversationId === id) expertDrafts.delete(submissionId);
            readingPositions.delete(id);
            expertSelections.delete(id); expertSelectionEpochs.delete(id);
            contentTabs.delete(id);
            if (repository && ownerHostId) enqueue(id, () => repository.delete(ownerHostId, id));
        },
        invalidate() { invalidated = true; },
        isDeleted(id: string) { return deleted.has(id); },
        isLoaded(id: string) { return !repository || loaded.has(id); },
        async flush() { await Promise.all(loading.values()); await queue; },
        contentTabs, expertSelections, expertSelectionEpochs, readingPositions,
        knowledge: new SvelteMap<string, KnowledgeDraft>(),
        rooms: new SvelteMap<string, RoomDraft>(),
        replyNotices: new SvelteMap<string, { submissionId: string; userMessageId: string; assistantMessageId: string; kind: 'notSent' | 'uncertain' }[]>(),
        submissions: new Map<string, {submissionId: string; text: string; attachmentIds: string[]}>(),
        transports: new Set<AbortController>(),
        completions, sources: createBrowserSourceRecovery(completions, pending =>
            controllers.get(pending.identity.conversation_id)?.acceptReceipt(pending.receipt)),
        lastConversationId: null as string | null };
    };
    let hostId: string | null = null;
    let current = create('');
    const clear = () => { current.readingPositions.clear(); current.expertSelections.clear(); current.expertSelectionEpochs.clear(); current.contentTabs.clear(); current.rooms.clear(); for (const controller of current.controllers.values()) controller.dispose(); current.controllers.clear(); current.sources.clear(); current.completions.clear(); for (const transport of current.transports) transport.abort(); current.transports.clear(); current.text.clear(); current.objectives.clear(); current.knowledge.clear(); current.replyNotices.clear(); current.attachments.clear(); current.submissions.clear(); current.lastConversationId = null; };
    return {
        forHost(id: string) {
            if (!id.trim()) throw new Error('Missing authenticated Host');
            if (hostId !== id) { current.invalidate(); clear(); current = create(id); hostId = id; }
            return current;
        },
        get hasDrafts() { return current.text.hasDrafts || current.objectives.hasDrafts || current.attachments.hasDrafts || [...current.knowledge.values()].some(knowledgePending) || [...current.rooms.values()].some(room => !!room.text || !!room.pending); },
        get selectedConversationId() { return current.lastConversationId; },
        get authenticatedHostId() { return hostId; },
        forgetConversation(id: string) {
            current.controllers.get(id)?.dispose(); current.controllers.delete(id);
            current.forget(id);
            current.completions.forget(id);
            current.sources.forget(id);
            current.text.delete(id); current.objectives.delete(id); current.knowledge.delete(id); current.attachments.delete(id); current.submissions.delete(id);
            current.replyNotices.delete(id);
            if (current.lastConversationId === id) current.lastConversationId = null;
        },
        clear() { current.invalidate(); clear(); current = create(hostId ?? ''); },
    };
}
export type WorkspaceDrafts = ReturnType<typeof createWorkspaceDrafts>;
