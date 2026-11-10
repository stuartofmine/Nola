// Core owns validation and transitions; this module decodes presentation data
// and binds a retry to the exact immutable question and answer payload.
export interface WaitQuestion {
    id: string; prompt: string; required: boolean; allow_freeform: boolean; max_text_chars: number;
    options: { id: string; label: string; description: string | null }[];
}
export interface PresentedWait {
    conversation_id: string; run_id: string; task_id: string | null;
    wait: { wait_id: string; revision: number; response_schema_digest: string;
        question_set: { questions: WaitQuestion[] }; state: { type: 'open' } };
}
export type WaitAnswer = { question_id: string; value: { type: 'option'; option_id: string } | { type: 'text'; text: string } };
export type WaitResponse = { type: 'answers'; answers: WaitAnswer[] } | { type: 'decline'; reason: null };
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const string = (v: unknown): v is string => typeof v === 'string' && v.trim().length > 0;
const invalid = (): never => { throw new Error('Invalid user input wait'); };
export function waitIdentity(record: PresentedWait): string {
    return JSON.stringify([record.conversation_id, record.run_id, record.wait.wait_id, record.wait.revision, record.wait.response_schema_digest]);
}
export function parseUserInputWaits(value: unknown, conversation: string): PresentedWait[] {
    if (!object(value) || value.conversation_id !== conversation || !Array.isArray(value.waits) || !Array.isArray(value.task_waits)) return invalid();
    const result: PresentedWait[] = [];
    const seen = new Set<string>();
    for (const [records, child] of [[value.waits, false], [value.task_waits, true]] as const) {
        for (const record of records) {
            if (!object(record) || !string(record.conversation_id) || !string(record.run_id)
                || (!child && record.conversation_id !== conversation) || (child && !string(record.task_id))) return invalid();
            const wait = record.wait;
            if (!object(wait) || !string(wait.wait_id) || !Number.isSafeInteger(wait.revision) || (wait.revision as number) < 1
                || !string(wait.response_schema_digest) || !object(wait.state) || wait.state.type !== 'open'
                || !object(wait.question_set) || !Array.isArray(wait.question_set.questions)) return invalid();
            const questions = wait.question_set.questions.map((q: unknown): WaitQuestion => {
                if (!object(q) || !string(q.id) || !string(q.prompt) || typeof q.required !== 'boolean' || typeof q.allow_freeform !== 'boolean'
                    || !Number.isSafeInteger(q.max_text_chars) || (q.max_text_chars as number) < 1 || !Array.isArray(q.options)) return invalid();
                const options = q.options.map((o: unknown) => {
                    if (!object(o) || !string(o.id) || !string(o.label) || !(o.description === null || typeof o.description === 'string')) return invalid();
                    return { id: o.id, label: o.label, description: o.description };
                });
                return { id: q.id, prompt: q.prompt, required: q.required, allow_freeform: q.allow_freeform, max_text_chars: q.max_text_chars as number, options };
            });
            const parsed: PresentedWait = { conversation_id: record.conversation_id, run_id: record.run_id, task_id: child ? record.task_id as string : null,
                wait: { wait_id: wait.wait_id, revision: wait.revision as number, response_schema_digest: wait.response_schema_digest,
                    question_set: { questions }, state: { type: 'open' } } };
            const identity = waitIdentity(parsed);
            if (seen.has(identity)) return invalid();
            seen.add(identity); result.push(parsed);
        }
    }
    return result;
}
export function freezeWaitResponse(record: PresentedWait, response: WaitResponse, commandId: string): string {
    if (!string(commandId)) return invalid();
    return JSON.stringify({ run_id: record.run_id, response: { command_id: commandId, wait_id: record.wait.wait_id,
        expected_revision: record.wait.revision, response_schema_digest: record.wait.response_schema_digest, response } });
}
