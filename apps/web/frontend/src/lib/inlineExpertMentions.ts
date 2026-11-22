import type { MainExpertSelection } from './mainExpertSelection.ts';
import type { PersistentAgent, RoomParticipant } from './agentRooms.ts';

export type ExpertMentionSegments = { version: 1; segments: { text: string; agent_id?: string }[] };
export interface InlineExpertMention {
    start: number;
    text: string;
    selection: MainExpertSelection;
}
export type MentionDraft = { epoch: string; occurrences: InlineExpertMention[] };

export function inlineQuery(text: string, cursor: number, mentions: readonly InlineExpertMention[]) {
    const match = /(?:^|[^A-Za-z0-9._%+@-])@([^\s@]*)$/u.exec(text.slice(0, cursor));
    if (!match) return null;
    const start = cursor - match[1].length - 1;
    return mentions.some(m => start >= m.start && start < m.start + m.text.length)
        ? null : { start, end: cursor, query: match[1] };
}

export function candidateIdentity(agent: PersistentAgent, member?: RoomParticipant): MainExpertSelection {
    return {agent_id:agent.id, agent_revision:member?.agent_revision ?? agent.revision,
        participant_id:member?.id ?? null, context_conversation_id:member?.context_conversation_id ?? null};
}

/** The DOM uses UTF-16 selections; the transport exchanges ordered literal strings. */
export function replaceMentionRange(text: string, mentions: InlineExpertMention[], start: number, end: number, replacement: string, selection?: MainExpertSelection) {
    const delta = replacement.length - (end - start);
    const next = mentions.flatMap(m => m.start + m.text.length <= start ? [m] : m.start >= end ? [{...m, start:m.start + delta}] : []);
    if (selection) next.push({start, text:replacement, selection});
    return {text:text.slice(0, start) + replacement + text.slice(end), mentions:next.sort((a,b) => a.start-b.start), cursor:start+replacement.length};
}

/** Use the actual replacement location even when adjacent names or plain text are identical. */
export function reconcileMentionEdit(oldText: string, next: string, caret: number, mentions: InlineExpertMention[]) {
    const right = next.slice(caret);
    const oldEnd = oldText.endsWith(right) ? oldText.length - right.length : oldText.length;
    let start = 0;
    while (start < Math.min(caret, oldEnd) && oldText[start] === next[start]) start++;
    return replaceMentionRange(oldText, mentions, start, oldEnd, next.slice(start, caret)).mentions;
}
export function mentionSegments(text: string, occurrences: InlineExpertMention[]): ExpertMentionSegments {
    let cursor = 0;
    const segments: ExpertMentionSegments['segments'] = [];
    for (const m of occurrences) {
        if (m.start < cursor || !Number.isSafeInteger(m.start) || !m.text.startsWith('@') || m.text.length < 2
            || text.slice(m.start, m.start + m.text.length) !== m.text) throw Error('Invalid mention position');
        if (m.start > cursor) segments.push({text:text.slice(cursor,m.start)});
        segments.push({text:m.text, agent_id:m.selection.agent_id});
        cursor = m.start + m.text.length;
    }
    if (cursor < text.length) segments.push({text:text.slice(cursor)});
    return {version:1, segments};
}
export function mentionTargets(occurrences: InlineExpertMention[]) {
    const targets = new Map<string, MainExpertSelection>();
    for (const {selection} of occurrences) {
        const previous = targets.get(selection.agent_id);
        if (previous && (previous.agent_revision !== selection.agent_revision || previous.participant_id !== selection.participant_id
            || previous.context_conversation_id !== selection.context_conversation_id)) throw Error('Expert configuration changed');
        targets.set(selection.agent_id, selection);
    }
    return Array.from(targets.values(),selection=>({...selection}));
}
const record = (v: unknown): v is Record<string,unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
export function storedMentions(value: unknown, text: string, optimistic = false): InlineExpertMention[] {
    if (!record(value) || value.version !== 1 || !Array.isArray(value.segments) || !Array.isArray(value.selected_experts)) return [];
    const occurrences: InlineExpertMention[] = [];
    let rendered = '';
    for (const segment of value.segments) {
        if (!record(segment) || typeof segment.text !== 'string') return [];
        if (segment.agent_id !== undefined) {
            const selected = value.selected_experts.find(s => record(s) && s.agent_id === segment.agent_id);
            if (!selected || typeof selected.agent_id !== 'string' || !selected.agent_id || !Number.isSafeInteger(selected.agent_revision)
                || selected.agent_revision < 1 || !segment.text.startsWith('@') || segment.text.length < 2
                || !(typeof selected.participant_id === 'string' && typeof selected.context_conversation_id === 'string'
                    || optimistic && selected.participant_id===null && selected.context_conversation_id===null)) return [];
            occurrences.push({start:rendered.length,text:segment.text,selection:{agent_id:selected.agent_id,agent_revision:selected.agent_revision,
                participant_id:selected.participant_id,context_conversation_id:selected.context_conversation_id}});
        }
        rendered += segment.text;
    }
    return rendered === text ? occurrences : [];
}
export function partMentions(part: Record<string,unknown>, text:string, optimistic=false) {
    const projection=part.projection;
    const field=record(projection)&&record(projection.fields)?projection.fields.expert_mentions:undefined;
    const saved=record(field)&&field.availability==='complete'?field.value:undefined;
    return storedMentions(part.expert_mentions ?? (record(part.content_json) ? part.content_json.expert_mentions : undefined)
        ?? saved, text, optimistic);
}
