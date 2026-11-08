import { apiResponse } from './api.ts';

export type PermissionChoice = 'allow_once' | 'allow_conversation' | 'deny' | 'provide_instruction' | 'return_to_composer';
export interface ChatPermissionChallenge {
    protocol_version: 4;
    allow_conversation: boolean;
    nonce: string; request_id: string; host_id: string; client_id: string; session_id: string; tool_name: string;
    expires_at: number;
    permission_instructions: { permission_request_id: string; instruction: string }[];
}
const identityFields = ['nonce', 'request_id', 'host_id', 'client_id', 'session_id', 'tool_name'] as const;
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const bytes = (value: string) => new TextEncoder().encode(value).length;

export function parseChatPermission(value: unknown): ChatPermissionChallenge | null {
    if (!object(value) || value.protocol_version !== 4 || typeof value.allow_conversation !== 'boolean' || !Number.isSafeInteger(value.expires_at)
        || Number(value.expires_at) <= 0 || !identityFields.every(key => typeof value[key] === 'string' && value[key] !== '')) return null;
    const instructions = value.permission_instructions ?? [];
    if (!Array.isArray(instructions) || instructions.length > 16 || !instructions.every(entry => object(entry)
        && Object.keys(entry).sort().join(',') === 'instruction,permission_request_id'
        && typeof entry.instruction === 'string' && typeof entry.permission_request_id === 'string')) return null;
    return { protocol_version: 4, allow_conversation: value.allow_conversation, ...Object.fromEntries(identityFields.map(key => [key, value[key]])),
        expires_at: Number(value.expires_at), permission_instructions: instructions.map(entry => ({ ...entry })) } as ChatPermissionChallenge;
}

export function permissionIdentity(value: ChatPermissionChallenge): string {
    return JSON.stringify([value.protocol_version, ...identityFields.map(key => value[key]), value.expires_at, value.allow_conversation]);
}
export function permissionExpired(value: ChatPermissionChallenge, nowMs = Date.now()): boolean {
    return !Number.isFinite(nowMs) || value.expires_at <= Math.floor(nowMs / 1000);
}

export function permissionDecision(value: ChatPermissionChallenge, choice: PermissionChoice, instruction = '', nowMs = Date.now()) {
    if (permissionExpired(value, nowMs)) throw new Error('chatPermission.expired');
    if (choice === 'allow_conversation' && !value.allow_conversation) throw new Error('chatPermission.invalid');
    let resolution: Record<string, unknown>;
    if (choice === 'provide_instruction') {
        const text = instruction.trim();
        const total = value.permission_instructions.reduce((sum, entry) => sum + bytes(entry.instruction) + bytes(entry.permission_request_id), 0)
            + bytes(text) + bytes(value.request_id);
        if (!text || bytes(text) > 4096 || value.permission_instructions.length >= 16 || total > 16384) throw new Error('chatPermission.instructionLimit');
        resolution = { type: 'intervention', intervention: { type: 'provide_instruction', instruction: text } };
    } else if (choice === 'return_to_composer') {
        resolution = { type: 'intervention', intervention: { type: 'return_to_composer' } };
    } else if (['deny', 'allow_once', 'allow_conversation'].includes(choice)) {
        resolution = { type: 'authorization', decision: choice };
    } else throw new Error('chatPermission.invalid');
    return { protocol_version: value.protocol_version, ...Object.fromEntries(identityFields.map(key => [key, value[key]])), resolution };
}

export type PermissionSubmissionState = 'ready' | 'submitting' | 'accepted' | 'unknown' | 'rejected';
const sameOriginPermissionFetch: typeof fetch = (input, options) => {
    if (input !== '/api/agent/host-permission/decision') throw new Error('Invalid permission endpoint');
    return apiResponse(input, options);
};
/** One frozen response per challenge. Transport acknowledgement is never an execution receipt. */
export class ChatPermissionSubmission {
    state: PermissionSubmissionState = 'ready';
    private body: string | null = null;
    private invalidated = false;
    readonly challenge: ChatPermissionChallenge;
    constructor(challenge: ChatPermissionChallenge) { this.challenge = structuredClone(challenge); }
    dispose(): void { this.invalidated = true; }
    async submit(choice: PermissionChoice, instruction = '', fetcher: typeof fetch = sameOriginPermissionFetch, nowMs = Date.now()): Promise<void> {
        if (this.invalidated || this.state !== 'ready') return;
        this.body = JSON.stringify(permissionDecision(this.challenge, choice, instruction, nowMs));
        await this.send(fetcher);
    }
    async retry(fetcher: typeof fetch = sameOriginPermissionFetch, nowMs = Date.now()): Promise<void> {
        if (this.invalidated || this.state !== 'unknown' || !this.body || permissionExpired(this.challenge, nowMs)) return;
        await this.send(fetcher);
    }
    private async send(fetcher: typeof fetch): Promise<void> {
        this.state = 'submitting';
        try {
            const response = await fetcher('/api/agent/host-permission/decision', {
                method: 'POST', headers: { 'Content-Type': 'application/json' }, body: this.body,
            });
            // Do not expose provider/Host error bodies. Server failure may follow delivery.
            if (!response.ok) {
                if (!this.invalidated) this.state = response.status >= 400 && response.status < 500 ? 'rejected' : 'unknown';
                return;
            }
            const receipt: unknown = await response.json();
            if (!this.invalidated) this.state = response.status === 200 && object(receipt)
                && receipt.accepted === true && receipt.nonce === this.challenge.nonce ? 'accepted' : 'unknown';
        } catch { if (!this.invalidated) this.state = 'unknown'; }
    }
}
