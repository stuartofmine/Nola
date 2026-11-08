import type { ChatTextAttachment } from './chatTextAttachments.ts';
import { MAX_TEXT_ATTACHMENTS, MAX_TEXT_ATTACHMENT_CHARACTERS, MAX_TEXT_ATTACHMENT_BYTES } from './chatTextAttachments.ts';
import type { PendingSourceSubmission } from './sourceSubmission.ts';

export interface ConversationDraftSubmissionTicket {
    hostId: string; clientId: string; conversationId: string; submissionId: string;
    draftRevision: number; pending: PendingSourceSubmission; committed: boolean;
}

export interface ConversationDraftRecord {
    hostId: string; conversationId: string; text: string; objective: string; attachments: ChatTextAttachment[];
}
export interface ConversationDraftRepository {
    read(hostId: string, conversationId: string): Promise<ConversationDraftRecord | null>;
    write(record: ConversationDraftRecord): Promise<void>;
    delete(hostId: string, conversationId: string): Promise<void>;
    stageSubmission?(record: ConversationDraftRecord, pending: PendingSourceSubmission): Promise<ConversationDraftSubmissionTicket>;
    readSubmission?(hostId: string, clientId: string, submissionId: string): Promise<ConversationDraftSubmissionTicket | null>;
    updateSubmission?(ticket: ConversationDraftSubmissionTicket, pending: PendingSourceSubmission): Promise<void>;
    retireSubmission?(ticket: ConversationDraftSubmissionTicket, retireDraft?: boolean): Promise<boolean>;
    pendingSubmissions?(hostId: string, clientId: string): Promise<ConversationDraftSubmissionTicket[]>;
}
export function validateConversationDraft(value: ConversationDraftRecord): ConversationDraftRecord {
    if (!value || !value.hostId?.trim() || !value.conversationId?.trim()
        || typeof value.text !== 'string' || typeof value.objective !== 'string'
        || value.text.length > 1_000_000 || value.objective.length > 100_000
        || !Array.isArray(value.attachments) || value.attachments.length > MAX_TEXT_ATTACHMENTS
        || new Set(value.attachments.map(a => a?.id)).size !== value.attachments.length
        || value.attachments.some(a => !a || a.conversationId !== value.conversationId
            || typeof a.id !== 'string' || !a.id || typeof a.name !== 'string' || a.name.length > 256
            || typeof a.mimeType !== 'string' || typeof a.clipped !== 'boolean'
            || typeof a.text !== 'string' || a.text.length > MAX_TEXT_ATTACHMENT_CHARACTERS * 2
            || a.text.includes('\0') || !Number.isSafeInteger(a.byteCount)
            || a.byteCount < 0 || a.byteCount > MAX_TEXT_ATTACHMENT_BYTES)) throw new Error('Invalid local conversation draft');
    return structuredClone(value);
}

/** Device-local unsubmitted input only. No messages, run receipts or credentials. */
export class IndexedDBConversationDraftRepository implements ConversationDraftRepository {
    private database: Promise<IDBDatabase> | null = null;
    private revisions = new Map<string, number>();
    private open(): Promise<IDBDatabase> {
        return this.database ??= new Promise((resolve, reject) => {
            const request = indexedDB.open('nola-conversation-workspace', 1);
            request.onupgradeneeded = () => {
                request.result.createObjectStore('drafts', {keyPath: ['hostId', 'conversationId']});
                request.result.createObjectStore('submissions', {keyPath: ['hostId', 'clientId', 'submissionId']});
            };
            request.onsuccess = () => resolve(request.result);
            request.onerror = () => { this.database = null; reject(request.error); };
            request.onblocked = () => { this.database = null; reject(new Error('Draft database blocked')); };
        });
    }
    async read(hostId: string, conversationId: string): Promise<ConversationDraftRecord | null> {
        const database = await this.open();
        return new Promise((resolve, reject) => {
            const request = database.transaction('drafts').objectStore('drafts').get([hostId, conversationId]);
            request.onsuccess = () => {
                try {
                    if (request.result?.deleted === true) throw new Error('Conversation draft was deleted');
                    const value = request.result ? validateConversationDraft(request.result) : null;
                    if (value && (value.hostId !== hostId || value.conversationId !== conversationId)) throw new Error('Foreign draft');
                    this.revisions.set(JSON.stringify([hostId, conversationId]), request.result?.revision ?? 0);
                    resolve(value);
                } catch (error) { reject(error); }
            };
            request.onerror = () => reject(request.error);
        });
    }
    async write(record: ConversationDraftRecord): Promise<void> {
        const snapshot = validateConversationDraft(record);
        const key = JSON.stringify([record.hostId, record.conversationId]);
        const database = await this.open();
        await new Promise<void>((resolve, reject) => {
            const transaction = database.transaction('drafts', 'readwrite');
            const store = transaction.objectStore('drafts');
            const read = store.get([record.hostId, record.conversationId]);
            let revision = 0;
            read.onsuccess = () => {
                revision = read.result?.revision ?? 0;
                if (read.result?.deleted === true || revision !== (this.revisions.get(key) ?? 0)) { transaction.abort(); return; }
                store.put({...snapshot, revision: revision + 1});
            };
            transaction.oncomplete = () => { this.revisions.set(key, revision + 1); resolve(); };
            transaction.onabort = transaction.onerror = () => reject(transaction.error ?? new Error('Draft changed in another window'));
        });
    }
    async delete(hostId: string, conversationId: string): Promise<void> {
        // Keep only an owner tombstone, never deleted content. Another window
        // with an old snapshot must not recreate the conversation's draft.
        await this.mutate(store => { store.put({hostId, conversationId, deleted: true}); });
        this.revisions.delete(JSON.stringify([hostId, conversationId]));
    }
    async stageSubmission(record: ConversationDraftRecord, pending: PendingSourceSubmission): Promise<ConversationDraftSubmissionTicket> {
        validateConversationDraft(record);
        const id = pending.identity;
        if (id.source_host_id !== record.hostId || id.conversation_id !== record.conversationId) throw new Error('Draft submission owner changed');
        const database = await this.open();
        return new Promise((resolve, reject) => {
            const transaction = database.transaction(['drafts', 'submissions'], 'readwrite');
            const read = transaction.objectStore('drafts').get([record.hostId, record.conversationId]);
            let ticket: ConversationDraftSubmissionTicket;
            read.onsuccess = () => {
                const current = read.result;
                const revision = current?.revision ?? 0;
                if (current?.deleted || revision !== (this.revisions.get(JSON.stringify([record.hostId, record.conversationId])) ?? 0)
                    || (current && (current.text !== record.text || JSON.stringify(current.attachments) !== JSON.stringify(record.attachments)))) { transaction.abort(); return; }
                ticket = {hostId: record.hostId, clientId: id.client_id, conversationId: record.conversationId,
                    submissionId: id.submission_id, draftRevision: revision, pending: structuredClone(pending), committed: false};
                transaction.objectStore('submissions').add(ticket);
            };
            transaction.oncomplete = () => resolve(ticket);
            transaction.onabort = transaction.onerror = () => reject(transaction.error ?? new Error('Draft changed before submission'));
        });
    }
    async readSubmission(hostId: string, clientId: string, submissionId: string): Promise<ConversationDraftSubmissionTicket | null> {
        const database = await this.open();
        return new Promise((resolve, reject) => {
            const transaction = database.transaction('submissions', 'readonly');
            const request = transaction.objectStore('submissions').get([hostId, clientId, submissionId]);
            transaction.oncomplete = () => resolve(request.result ?? null);
            transaction.onabort = transaction.onerror = () => reject(transaction.error ?? new Error('Submission ticket read failed'));
        });
    }
    async updateSubmission(ticket: ConversationDraftSubmissionTicket, pending: PendingSourceSubmission): Promise<void> {
        if (JSON.stringify(ticket.pending.identity) !== JSON.stringify(pending.identity)
            || JSON.stringify(ticket.pending.request) !== JSON.stringify(pending.request)) throw new Error('Frozen submission changed');
        const database = await this.open();
        await new Promise<void>((resolve, reject) => {
            const transaction = database.transaction('submissions', 'readwrite');
            const store = transaction.objectStore('submissions');
            const read = store.get([ticket.hostId, ticket.clientId, ticket.submissionId]);
            read.onsuccess = () => {
                if (!read.result || read.result.draftRevision !== ticket.draftRevision) { transaction.abort(); return; }
                store.put({...read.result, pending: structuredClone(pending)});
            };
            transaction.oncomplete = () => resolve();
            transaction.onabort = transaction.onerror = () => reject(transaction.error ?? new Error('Submission ticket changed'));
        });
    }
    async retireSubmission(ticket: ConversationDraftSubmissionTicket, retireDraft = true): Promise<boolean> {
        const database = await this.open();
        return new Promise((resolve, reject) => {
            const transaction = database.transaction(['drafts', 'submissions'], 'readwrite');
            const submissions = transaction.objectStore('submissions');
            const readTicket = submissions.get([ticket.hostId, ticket.clientId, ticket.submissionId]);
            let retired = false;
            readTicket.onsuccess = () => {
                if (!readTicket.result || readTicket.result.draftRevision !== ticket.draftRevision) { transaction.abort(); return; }
                if (readTicket.result.committed) return;
                const drafts = transaction.objectStore('drafts');
                const read = drafts.get([ticket.hostId, ticket.conversationId]);
                read.onsuccess = () => {
                    const current = read.result;
                    if (retireDraft && current && !current.deleted && current.revision === ticket.draftRevision) {
                        drafts.put({...current, text: '', attachments: [], revision: current.revision + 1}); retired = true;
                    }
                    submissions.put({...readTicket.result, committed: true});
                };
            };
            transaction.oncomplete = () => {
                if (retired) this.revisions.set(JSON.stringify([ticket.hostId, ticket.conversationId]), ticket.draftRevision + 1);
                resolve(retired);
            };
            transaction.onabort = transaction.onerror = () => reject(transaction.error ?? new Error('Submission retirement failed'));
        });
    }
    async pendingSubmissions(hostId: string, clientId: string): Promise<ConversationDraftSubmissionTicket[]> {
        const database = await this.open();
        return new Promise((resolve, reject) => {
            const request = database.transaction('submissions').objectStore('submissions').getAll();
            request.onsuccess = () => resolve(request.result.filter((ticket: ConversationDraftSubmissionTicket) =>
                ticket.hostId === hostId && ticket.clientId === clientId && !ticket.committed));
            request.onerror = () => reject(request.error);
        });
    }
    private async mutate(operation: (store: IDBObjectStore) => void): Promise<void> {
        const database = await this.open();
        return new Promise((resolve, reject) => {
            const transaction = database.transaction('drafts', 'readwrite');
            transaction.oncomplete = () => resolve();
            transaction.onabort = transaction.onerror = () => reject(transaction.error ?? new Error('Draft write failed'));
            operation(transaction.objectStore('drafts'));
        });
    }
}
