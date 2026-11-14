import { markdownHref } from './markdown.ts';
import type { TimelinePart } from './chatTimeline.ts';

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const text = (value: unknown): string => typeof value === 'string' ? value : '';
const counter = (value: unknown): string => typeof value === 'string' && /^\d+$/.test(value) ? value
    : typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? String(value) : '';

export interface ChatReference {
    kind: 'source' | 'citation' | 'attachment';
    title: string; uri: string; href?: string; mimeType: string; snippet: string;
    sourceId: string; section: string; page: string; lineStart: string; lineEnd: string;
    rangeStart: string; rangeEnd: string; operation: string;
}

/** Public display only. Local paths never become browser or host-file capabilities. */
export function chatReference(value: unknown): ChatReference | null {
    if (!object(value) || !['source', 'citation', 'attachment'].includes(text(value.kind))) return null;
    const location = object(value.location) ? value.location : {};
    const range = object(value.range) ? value.range : {};
    const uri = text(value.uri);
    const safe = markdownHref(uri);
    return {
        kind: value.kind as ChatReference['kind'], title: text(value.title) || text(value.name), uri,
        href: safe && /^https?:/.test(safe) ? safe : undefined,
        mimeType: text(value.mime_type), snippet: text(value.snippet), sourceId: text(value.source_id),
        section: text(location.section ?? value.section), page: counter(location.page ?? value.page),
        lineStart: counter(location.line_start ?? value.line_start), lineEnd: counter(location.line_end ?? value.line_end),
        rangeStart: counter(range.start ?? value.range_start), rangeEnd: counter(range.end ?? value.range_end),
        operation: text(value.operation),
    };
}

export function timelineReference(part: TimelinePart): ChatReference | null { return chatReference(part.value); }
export function toolArtifacts(value: unknown): ChatReference[] {
    if (!Array.isArray(value)) return [];
    return value.flatMap(item => {
        if (!object(item)) return [];
        const reference = chatReference({...item, kind: 'attachment'});
        return reference && (reference.uri || reference.title) ? [reference] : [];
    });
}

/** Bibliographic aliases share a row; pinpoint locations retain their own row. */
export function bibliography(parts: TimelinePart[]): {part: TimelinePart; reference: ChatReference}[] {
    const seen = new Set<string>();
    return parts.flatMap(part => {
        const reference = timelineReference(part);
        if (!reference || reference.kind === 'attachment') return [];
        const key = JSON.stringify([reference.uri || reference.sourceId || part.id, reference.section,
            reference.page, reference.lineStart, reference.lineEnd, reference.rangeStart, reference.rangeEnd]);
        if (seen.has(key)) return [];
        seen.add(key);
        return [{part, reference}];
    });
}
