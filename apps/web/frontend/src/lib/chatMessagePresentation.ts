import { timelinePartText, timelineToolPresentation } from './chatTimeline.ts';
import type { TimelinePart } from './chatTimeline.ts';

export type MessagePresentation = {id: string; kind: 'text'; text: string}
    | {id: string; kind: 'tools'; parts: TimelinePart[]}
    | {id: string; kind: 'part'; part: TimelinePart};

export type MessageFlowPresentation = MessagePresentation | {id: string; kind: 'process'; parts: TimelinePart[]};

/** Partition by typed provenance, never by generated prose or localized titles. */
export function isExecutionPart(part: TimelinePart): boolean {
    return part.value.kind === 'tool' || part.value.kind === 'reasoning'
        || (part.value.kind === 'historical' && (part.value.canonical_kind === 'reasoning'
            || timelinePartText(part, 'reasoning').length > 0));
}

export function isExecutionPresentation(group: MessagePresentation): group is Extract<MessagePresentation, {kind: 'tools' | 'part'}> {
    return group.kind === 'tools' || group.kind === 'part' && isExecutionPart(group.part);
}

/** Only the current Core activity can open itself. Earlier completed rounds
 * remain available at their original anchors without reopening on a new delta. */
export function liveReasoningParts(parts: TimelinePart[]): TimelinePart[] {
    return parts.filter(part => part.value.kind === 'reasoning'
        && ['pending', 'running'].includes(String(part.value.state)));
}

/** Message-scoped choices survive a segment's first Part changing. Explicit
 * collapse wins when previously independent segments become contiguous. */
export function executionProcessChoice(parts: TimelinePart[], choices: Record<string, boolean>): boolean | null {
    const values = parts.map(part => choices[part.id]);
    return values.includes(false) ? false : values.includes(true) ? true : null;
}

export function executionProcessExpanded(choice: boolean | null, initiallyExpanded: boolean): boolean {
    return choice ?? initiallyExpanded;
}

/** Structural renderer segments, never a rewritten transcript. A Surface,
 * withheld/empty text, or another public part is a real process boundary. */
export function messageFlowPresentation(parts: TimelinePart[]): MessageFlowPresentation[] {
    const result: MessageFlowPresentation[] = [];
    for (let index = 0; index < parts.length;) {
        const part = parts[index];
        if (isExecutionPart(part)) {
            const segment: TimelinePart[] = [];
            do { segment.push(parts[index++]); } while (index < parts.length && isExecutionPart(parts[index]));
            result.push({id: part.id, kind: 'process', parts: segment});
        } else {
            const segment: TimelinePart[] = [];
            do { segment.push(parts[index++]); } while (index < parts.length && !isExecutionPart(parts[index]));
            result.push(...messagePresentation(segment));
        }
    }
    return result;
}

/** Only adjacent public prose may join for Markdown continuity, and only
 * runs of at least three explicitly completed tools may become a compact group.
 * References, failures and live activity are real ordering boundaries. */
export function messagePresentation(parts: TimelinePart[]): MessagePresentation[] {
    const result: MessagePresentation[] = [];
    for (let index = 0; index < parts.length;) {
        const part = parts[index];
        const text = timelinePartText(part);
        if (text) {
            let joined = text; index++;
            while (index < parts.length && timelinePartText(parts[index])) joined += timelinePartText(parts[index++]);
            result.push({id: part.id, kind: 'text', text: joined});
        } else if (timelineToolPresentation(part)?.state === 'completed') {
            const completed: TimelinePart[] = [];
            do { completed.push(parts[index++]); } while (index < parts.length && timelineToolPresentation(parts[index])?.state === 'completed');
            if (completed.length >= 3) result.push({id: part.id, kind: 'tools', parts: completed});
            else result.push(...completed.map(part => ({id: part.id, kind: 'part' as const, part})));
        } else { result.push({id: part.id, kind: 'part', part}); index++; }
    }
    return result;
}
